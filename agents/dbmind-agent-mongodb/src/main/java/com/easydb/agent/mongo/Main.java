package com.dbmind.agent.mongo;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mongodb.ConnectionString;
import com.mongodb.MongoClientSettings;
import com.mongodb.client.MongoClient;
import com.mongodb.client.MongoClients;
import com.mongodb.client.MongoCursor;
import org.bson.Document;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * DBMind MongoDB 专属宿主。
 *
 * <p>与 JDBC 宿主**共用同一套 NDJSON 协议**（见 docs/architecture.md），差别只在两处：
 * <ul>
 *   <li>握手声明 {@code generic=false} + {@code agentKeys=["mongodb"]}：不冒充通用宿主；</li>
 *   <li>连接方式：既然 Mongo 不是 JDBC，就没有「用户自己放驱动 jar」这回事，
 *       驱动库随宿主打包，连接串由本进程按 mongodb:// 语义拼。</li>
 * </ul>
 *
 * <p>只读判定**不在这里**：那属于内核的安全闸门（`mongo.rs`）。
 * 宿主只执行，不做权限判断 —— 否则「闸门在内核」这条就破了。
 */
public final class Main {

    private static final Gson GSON = new GsonBuilder().serializeNulls().create();
    private static final int SCHEMA_SAMPLE_SIZE = 50;

    private static final Map<String, MongoSession> SESSIONS = new ConcurrentHashMap<>();
    /** requestId -> 可中断标记：游标批次循环据此提前收手。 */
    private static final Map<String, AtomicBoolean> RUNNING = new ConcurrentHashMap<>();

    private static final ExecutorService WORKERS = Executors.newCachedThreadPool(runnable -> {
        Thread thread = new Thread(runnable, "dbmind-mongo-worker");
        thread.setDaemon(true);
        return thread;
    });

    private static final Object OUT_LOCK = new Object();
    private static PrintWriter out;

    private Main() {
    }

    public static void main(String[] args) throws Exception {
        out = new PrintWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8), false);
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));

        String line;
        while ((line = in.readLine()) != null) {
            if (line.isBlank()) {
                continue;
            }
            JsonObject request;
            try {
                request = JsonParser.parseString(line).getAsJsonObject();
            }
            catch (RuntimeException e) {
                write(failure("", "DBMIND-INTERNAL-0001", "请求不是合法 JSON", e.toString()));
                continue;
            }
            // 主线程必须能继续读下一行，否则 cancel 永远排不上队
            WORKERS.submit(() -> dispatch(request));
        }
        shutdown();
    }

    private static void dispatch(JsonObject request) {
        String id = request.has("id") && !request.get("id").isJsonNull() ? request.get("id").getAsString() : "";
        String method = request.has("method") ? request.get("method").getAsString() : "";
        try {
            JsonElement result = switch (method) {
                case "handshake" -> handshake();
                case "ping" -> ping();
                case "connect" -> connect(request);
                case "query" -> query(id, request);
                case "tables" -> tables(request);
                case "columns" -> columns(request);
                case "cancel" -> cancel(request);
                case "disconnect" -> disconnect(request);
                case "shutdown" -> {
                    // 先释放资源再回执：内核收到回执后可能立刻强杀进程
                    shutdown();
                    write(ok(id, ping()));
                    System.exit(0);
                    yield null;
                }
                default -> throw new AgentException("DBMIND-INTERNAL-0001", "未知方法：" + method);
            };
            if (result != null) {
                write(ok(id, result));
            }
        }
        catch (AgentException e) {
            write(failure(id, e.code(), e.getMessage(), e.detail()));
        }
        catch (Throwable e) {
            write(failure(id, "DBMIND-QUERY-0002", "agent 内部错误", stackTrace(e)));
        }
    }

    // ------------------------------------------------------------ 方法实现

    private static JsonObject handshake() {
        JsonObject result = new JsonObject();
        result.addProperty("protocolVersion", 1);
        result.addProperty("agent", "mongodb");
        // 专属宿主：明确声明只承载 mongodb，不接 JDBC 类型的活
        result.addProperty("generic", false);
        JsonArray keys = new JsonArray();
        keys.add("mongodb");
        result.add("agentKeys", keys);
        result.addProperty("driverVersion", "mongodb-driver-sync");
        result.addProperty("javaVersion", System.getProperty("java.version"));
        return result;
    }

    private static JsonObject ping() {
        JsonObject result = new JsonObject();
        result.addProperty("pong", true);
        result.addProperty("sessions", SESSIONS.size());
        result.addProperty("running", RUNNING.size());
        return result;
    }

    /**
     * socket 读超时（驱动默认是 0 = 永不超时）。
     *
     * <p>取 11 分钟 = 内核允许的最大查询超时（10 分钟）+ 1 分钟余量：它**不是**用来给查询限时的
     * （那是服务端 `maxTimeMS` 与内核 `timeoutMs` 的事），只是「服务端完全不回包」时的兜底，
     * 免得一条工作线程和一条泳道槽位被永久占住。
     */
    private static final int SOCKET_READ_TIMEOUT_MS = 11 * 60 * 1000;

    private static JsonObject connect(JsonObject request) {
        String sessionId = required(request, "sessionId");
        String host = required(request, "host");
        int port = request.has("port") && !request.get("port").isJsonNull()
                ? request.get("port").getAsInt()
                : 27017;
        String database = text(request, "database");
        String username = text(request, "username");
        String password = text(request, "password");

        String uri = buildUri(host, port, database, username, password);
        MongoClient client;
        try {
            MongoClientSettings settings = MongoClientSettings.builder()
                    .applyConnectionString(new ConnectionString(uri))
                    // 失败要快：默认 30s 会让用户以为界面卡死
                    .applyToClusterSettings(builder -> builder.serverSelectionTimeout(10, java.util.concurrent.TimeUnit.SECONDS))
                    // **有界的读超时**：驱动的默认值是 0（永不超时）。内核最多允许 10 分钟的
                    // 查询超时，所以服务端真挂住时这一层得有个比它更松的兜底 —— 否则那条宿主
                    // 工作线程会**永远**卡在 socket 上，连带那条泳道槽位一起丢掉。
                    // 这里取 11 分钟（内核上限 + 1 分钟余量）。真正限时的是服务端的
                    // `maxTimeMS`（见 applyMaxTime）与内核的 timeoutMs，这一层只是兜底。
                    .applyToSocketSettings(builder -> builder
                            .connectTimeout(10, java.util.concurrent.TimeUnit.SECONDS)
                            .readTimeout(SOCKET_READ_TIMEOUT_MS, java.util.concurrent.TimeUnit.MILLISECONDS))
                    .build();
            client = MongoClients.create(settings);
            String effectiveDb = database == null || database.isBlank() ? "admin" : database;
            // 真正建立连接（驱动是惰性的，不 ping 一下 connect 会「假装成功」）
            client.getDatabase(effectiveDb).runCommand(new Document("ping", 1));
            String version = readServerVersion(client, effectiveDb);
            MongoSession previous = SESSIONS.put(sessionId, new MongoSession(sessionId, client, effectiveDb, version));
            if (previous != null) {
                previous.close();
            }
            JsonObject result = new JsonObject();
            result.addProperty("serverVersion", version);
            result.addProperty("uri", redact(uri));
            return result;
        }
        catch (AgentException e) {
            throw e;
        }
        catch (RuntimeException e) {
            throw new AgentException("DBMIND-CONN-0003", "连接失败：" + e.getMessage(), e.getClass().getSimpleName());
        }
    }

    private static JsonElement query(String requestId, JsonObject request) {
        MongoSession session = session(request);
        String statement = required(request, "sql");
        int maxRows = request.has("maxRows") ? request.get("maxRows").getAsInt() : 2000;
        int timeoutMs = request.has("timeoutMs") ? request.get("timeoutMs").getAsInt() : 30_000;

        CommandParser.Parsed parsed = CommandParser.parse(statement, session.defaultDatabase());
        AtomicBoolean cancelled = new AtomicBoolean(false);
        RUNNING.put(requestId, cancelled);
        try {
            JsonObject result = new JsonObject();
            JsonArray notices = new JsonArray();
            if (parsed.notice() != null) {
                notices.add(parsed.notice());
            }

            if (parsed.command() == null) {
                // 例如 `use otherdb`：没有命令要执行，只回报切换结果
                result.add("columns", singleColumn("result"));
                JsonArray rows = new JsonArray();
                JsonObject row = new JsonObject();
                row.add("ok", JsonParser.parseString("{\"t\":\"integer\",\"v\":1}"));
                rows.add(row);
                result.add("rows", rows);
                result.addProperty("rowCount", 1);
                result.addProperty("truncated", false);
                result.add("affectedRows", null);
                result.add("notices", notices);
                return result;
            }

            String database = parsed.database() != null ? parsed.database() : session.defaultDatabase();
            Document command = parsed.command();
            if (timeoutMs > 0) {
                applyMaxTime(command, timeoutMs);
            }

            Document raw = session.client().getDatabase(database).runCommand(command);
            return ResultWriter.render(raw, parsed, maxRows, cancelled, notices);
        }
        catch (AgentException e) {
            throw e;
        }
        catch (com.mongodb.MongoSocketReadTimeoutException e) {
            // 读超时不是「命令执行失败」：服务端**没有回包**，用户该去看超时/范围，
            // 而不是以为自己那条命令写错了（与 Redis / ES 宿主同一套口径）。
            if (cancelled.get()) {
                throw new AgentException("DBMIND-QUERY-0004", "查询已取消", e.getClass().getSimpleName());
            }
            throw new AgentException("DBMIND-QUERY-0003",
                    "服务端长时间无响应（超过 " + (SOCKET_READ_TIMEOUT_MS / 60_000) + " 分钟没有回包），已中止",
                    "可提高连接配置里的查询超时，或缩小命令范围\n"
                            + "（MongoClient 自带连接池，这条会话不需要重建）");
        }
        catch (RuntimeException e) {
            if (cancelled.get()) {
                throw new AgentException("DBMIND-QUERY-0004", "查询已取消", e.getClass().getSimpleName());
            }
            throw new AgentException("DBMIND-QUERY-0002", "命令执行失败：" + e.getMessage(), e.getClass().getSimpleName());
        }
        finally {
            RUNNING.remove(requestId);
        }
    }

    private static JsonElement tables(JsonObject request) {
        MongoSession session = session(request);
        try {
            JsonArray tables = new JsonArray();
            try (MongoCursor<Document> cursor = session.client()
                    .getDatabase(session.defaultDatabase())
                    .listCollections()
                    .iterator()) {
                while (cursor.hasNext()) {
                    Document doc = cursor.next();
                    JsonObject table = new JsonObject();
                    table.addProperty("name", doc.getString("name"));
                    Object type = doc.get("type");
                    table.addProperty("kind", type != null && "view".equalsIgnoreCase(type.toString()) ? "view" : "table");
                    tables.add(table);
                }
            }
            return tables;
        }
        catch (RuntimeException e) {
            throw new AgentException("DBMIND-QUERY-0002", "读取集合清单失败：" + e.getMessage(), e.getClass().getSimpleName());
        }
    }

    /**
     * 列信息靠**抽样**得出。
     *
     * <p>Mongo 没有固定 schema，说「这张表有哪些列」本身就是个概率问题。
     * 与其假装确定，不如抽样并把抽样事实告诉用户（见 notice）——
     * 这是文档型数据库里最容易被糊弄过去的一处。
     */
    private static JsonElement columns(JsonObject request) {
        MongoSession session = session(request);
        String collection = required(request, "table");
        try {
            Map<String, String> types = new LinkedHashMap<>();
            int sampled = 0;
            try (MongoCursor<Document> cursor = session.client()
                    .getDatabase(session.defaultDatabase())
                    .getCollection(collection)
                    .find()
                    .limit(SCHEMA_SAMPLE_SIZE)
                    .iterator()) {
                while (cursor.hasNext()) {
                    Document doc = cursor.next();
                    sampled++;
                    for (Map.Entry<String, Object> entry : doc.entrySet()) {
                        types.putIfAbsent(entry.getKey(), bsonTypeName(entry.getValue()));
                    }
                }
            }
            if (sampled == 0) {
                // 采样不到任何文档：先确认集合是否存在。
                // 不存在就如实返回空 —— 凭空给出一个 `_id` 会让用户以为集合是存在的。
                boolean exists;
                try (MongoCursor<Document> cursor = session.client()
                        .getDatabase(session.defaultDatabase())
                        .listCollections()
                        .filter(new Document("name", collection))
                        .iterator()) {
                    exists = cursor.hasNext();
                }
                if (!exists) {
                    return new JsonArray();
                }
            }
            // Mongo 文档一定有 _id：存在的集合即使还没数据也把它列出来
            Map<String, String> ordered = new LinkedHashMap<>();
            ordered.put("_id", "objectId");
            ordered.putAll(types);

            JsonArray columns = new JsonArray();
            for (Map.Entry<String, String> entry : ordered.entrySet()) {
                JsonObject column = new JsonObject();
                column.addProperty("name", entry.getKey());
                column.addProperty("typeName", entry.getValue());
                // 抽样无法证明非空，所以一律标可空；_id 是唯一强制字段
                column.addProperty("nullable", !"_id".equals(entry.getKey()));
                column.addProperty("primaryKey", "_id".equals(entry.getKey()));
                column.add("defaultValue", null);
                columns.add(column);
            }
            return columns;
        }
        catch (RuntimeException e) {
            throw new AgentException("DBMIND-QUERY-0002", "读取字段信息失败：" + e.getMessage(), e.getClass().getSimpleName());
        }
    }

    private static JsonObject cancel(JsonObject request) {
        String requestId = required(request, "requestId");
        AtomicBoolean flag = RUNNING.get(requestId);
        boolean cancelled = false;
        if (flag != null) {
            flag.set(true);
            cancelled = true;
        }
        JsonObject result = new JsonObject();
        result.addProperty("cancelled", cancelled);
        return result;
    }

    private static JsonObject disconnect(JsonObject request) {
        String sessionId = required(request, "sessionId");
        MongoSession session = SESSIONS.remove(sessionId);
        if (session != null) {
            session.close();
        }
        JsonObject result = new JsonObject();
        result.addProperty("closed", session != null);
        return result;
    }

    // ------------------------------------------------------------ 工具

    /** 拼连接串。密码里的特殊字符必须转义，否则一个带 @ 的密码就能毁掉整个连接。 */
    static String buildUri(String host, int port, String database, String username, String password) {
        StringBuilder uri = new StringBuilder("mongodb://");
        if (username != null && !username.isBlank()) {
            uri.append(encode(username));
            if (password != null && !password.isEmpty()) {
                uri.append(':').append(encode(password));
            }
            uri.append('@');
        }
        uri.append(host).append(':').append(port).append('/');
        if (database != null && !database.isBlank()) {
            uri.append(encode(database));
        }
        else if (username != null && !username.isBlank()) {
            // 没指定库时认证信息默认在 admin 库
            uri.append("?authSource=admin");
        }
        return uri.toString();
    }

    private static String encode(String value) {
        return URLEncoder.encode(value, StandardCharsets.UTF_8).replace("+", "%20");
    }

    /** 日志里不出现口令。 */
    private static String redact(String uri) {
        return uri.replaceAll("://[^:@/]+:[^@/]+@", "://***:***@");
    }

    private static void applyMaxTime(Document command, int timeoutMs) {
        // 只给读类命令加：写命令不认这个字段，塞进去反而可能报错
        String name = command.isEmpty() ? "" : command.keySet().iterator().next();
        Set<String> readable = Set.of("find", "aggregate", "count", "distinct", "listCollections", "listIndexes");
        if (readable.contains(name) && !command.containsKey("maxTimeMS")) {
            command.put("maxTimeMS", timeoutMs);
        }
    }

    private static String readServerVersion(MongoClient client, String database) {
        try {
            Document build = client.getDatabase(database).runCommand(new Document("buildInfo", 1));
            Object version = build.get("version");
            return version == null ? "MongoDB" : "MongoDB " + version;
        }
        catch (RuntimeException e) {
            return "MongoDB";
        }
    }

    private static void shutdown() {
        for (MongoSession session : SESSIONS.values()) {
            session.close();
        }
        SESSIONS.clear();
        WORKERS.shutdownNow();
    }

    private static MongoSession session(JsonObject request) {
        String sessionId = required(request, "sessionId");
        MongoSession session = SESSIONS.get(sessionId);
        if (session == null) {
            // 会话没了 ⇒ 对内核而言是**连接级失败**（0003，不是「连接不存在」的 0001）：
            // 它会清掉缓存、下一次重连。
            throw new AgentException(
                    "DBMIND-CONN-0003",
                    "会话不存在（宿主可能已重启）：" + sessionId,
                    "内核会按需要重新连接");
        }
        return session;
    }

    private static JsonArray singleColumn(String name) {
        JsonArray columns = new JsonArray();
        JsonObject column = new JsonObject();
        column.addProperty("name", name);
        column.addProperty("typeName", "int32");
        columns.add(column);
        return columns;
    }

    static String bsonTypeName(Object value) {
        if (value == null) {
            return "null";
        }
        if (value instanceof String) {
            return "string";
        }
        if (value instanceof Integer) {
            return "int32";
        }
        if (value instanceof Long) {
            return "int64";
        }
        if (value instanceof Double || value instanceof Float) {
            return "double";
        }
        if (value instanceof java.math.BigDecimal || value instanceof org.bson.types.Decimal128) {
            return "decimal";
        }
        if (value instanceof Boolean) {
            return "bool";
        }
        if (value instanceof org.bson.types.ObjectId) {
            return "objectId";
        }
        if (value instanceof java.util.Date) {
            return "date";
        }
        if (value instanceof org.bson.types.Binary) {
            return "binData";
        }
        if (value instanceof List || value instanceof Object[]) {
            return "array";
        }
        if (value instanceof Document || value instanceof Map) {
            return "object";
        }
        return value.getClass().getSimpleName().toLowerCase();
    }

    private static String required(JsonObject request, String field) {
        String value = text(request, field);
        if (value == null || value.isEmpty()) {
            throw new AgentException("DBMIND-INTERNAL-0001", "缺少必填参数：" + field);
        }
        return value;
    }

    private static String text(JsonObject request, String field) {
        if (!request.has(field) || request.get(field).isJsonNull()) {
            return null;
        }
        return request.get(field).getAsString();
    }

    private static String ok(String id, JsonElement result) {
        JsonObject envelope = new JsonObject();
        envelope.addProperty("id", id);
        envelope.addProperty("ok", true);
        envelope.add("result", result == null ? new JsonObject() : result);
        return GSON.toJson(envelope);
    }

    private static String failure(String id, String code, String message, String detail) {
        JsonObject envelope = new JsonObject();
        envelope.addProperty("id", id);
        envelope.addProperty("ok", false);
        envelope.addProperty("code", code);
        envelope.addProperty("message", message);
        if (detail != null && !detail.isBlank()) {
            envelope.addProperty("detail", detail);
        }
        return GSON.toJson(envelope);
    }

    private static String stackTrace(Throwable throwable) {
        java.io.StringWriter writer = new java.io.StringWriter();
        throwable.printStackTrace(new java.io.PrintWriter(writer));
        return writer.toString();
    }

    private static void write(String line) {
        synchronized (OUT_LOCK) {
            out.println(line);
            out.flush();
        }
    }

    /** agent 侧业务异常：带内核错误码，壳层无需猜。 */
    public static final class AgentException extends RuntimeException {
        private final String code;
        private final String detail;

        public AgentException(String code, String message) {
            this(code, message, null);
        }

        public AgentException(String code, String message, String detail) {
            super(message);
            this.code = code;
            this.detail = detail;
        }

        public String code() {
            return code;
        }

        public String detail() {
            return detail;
        }
    }

    static {
        // 驱动会往 stderr 打日志；内核只读 stdout
        System.setErr(new java.io.PrintStream(java.io.OutputStream.nullOutputStream(), true, StandardCharsets.UTF_8));
    }
}
