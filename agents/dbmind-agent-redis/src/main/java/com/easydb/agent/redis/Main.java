package com.dbmind.agent.redis;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import redis.clients.jedis.Jedis;
import redis.clients.jedis.Protocol;
import redis.clients.jedis.commands.ProtocolCommand;
import redis.clients.jedis.exceptions.JedisConnectionException;
import redis.clients.jedis.exceptions.JedisDataException;
import redis.clients.jedis.params.ScanParams;
import redis.clients.jedis.resps.ScanResult;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * DBmind Redis 专属宿主。
 *
 * <p>三个刻意的设计：
 * <ul>
 *   <li><b>一个会话一条长连接</b>：Redis 的 {@code SELECT} 与事务都是有状态的，
 *       连接池会把状态搞丢（这次选了 db3，下条命令可能落在 db0）。</li>
 *   <li><b>流式命令明确拒绝</b>：本协议是一次请求一次响应，
 *       {@code SUBSCRIBE}/{@code MONITOR}/{@code BLPOP} 服务不了 —— 直接报错并说明，
 *       而不是挂在那里让界面转圈。</li>
 *   <li><b>只读判定不在这里</b>：那属于内核安全闸门（<code>redis.rs</code>），
 *       宿主只执行。</li>
 * </ul>
 */
public final class Main {

    private static final Gson GSON = new GsonBuilder().serializeNulls().create();
    /** 键空间浏览上限：SCAN 全库可能是百万级键，界面展示必须有界。 */
    private static final int MAX_KEYS = 500;

    /** 这些命令在「一次请求一次响应」的模型里服务不了。 */
    private static final Set<String> STREAMING = Set.of(
            "SUBSCRIBE", "UNSUBSCRIBE", "PSUBSCRIBE", "PUNSUBSCRIBE", "SSUBSCRIBE", "SUNSUBSCRIBE",
            "MONITOR", "SYNC", "PSYNC", "BLPOP", "BRPOP", "BRPOPLPUSH", "BLMOVE", "BZPOPMIN",
            "BZPOPMAX", "XREAD", "XREADGROUP", "WAIT", "MULTI", "EXEC", "DISCARD", "WATCH", "UNWATCH");

    private static final Map<String, Session> SESSIONS = new ConcurrentHashMap<>();
    private static final Map<String, AtomicBoolean> RUNNING = new ConcurrentHashMap<>();

    private static final ExecutorService WORKERS = Executors.newCachedThreadPool(runnable -> {
        Thread thread = new Thread(runnable, "dbmind-redis-worker");
        thread.setDaemon(true);
        return thread;
    });

    private static final Object OUT_LOCK = new Object();
    private static PrintWriter out;

    private Main() {
    }

    /**
     * 一个会话 = 一条 Jedis 长连接 + 当前库（SELECT 会改它）。
     *
     * 连接**坏掉之后要能自己重建**：Jedis 在读超时/断链后会把连接标成 broken 并关掉它，
     * 而内核那边对**非幂等**查询不做重试（`agent_driver::with_lane` 的 `retry_on_stale`
     * 只对幂等调用为真）。也就是说宿主如果不自愈，用户这条连接会一直坏下去，
     * 直到他自己重新连一次 —— 真机验证第一次就是撞在这上面（跑一条 2 秒的命令，
     * 之后整条连接都报「连接已断开」）。所以这里留着重建所需的一切：
     * 会话 id 不变，内核不需要知道中间发生过什么。
     */
    private static final class Session {
        final String host;
        final int port;
        final String username;
        final String password;
        Jedis jedis;
        final int initialDb;
        volatile int currentDb;
        /** 连接已不可用（读超时 / 断链）：下一条命令前先重建。 */
        private volatile boolean broken;

        Session(String host, int port, String username, String password, Jedis jedis, int initialDb) {
            this.host = host;
            this.port = port;
            this.username = username;
            this.password = password;
            this.jedis = jedis;
            this.initialDb = initialDb;
            this.currentDb = initialDb;
        }

        /**
         * 记下「这条连接不能用了」并关掉它。
         *
         * **不在失败的那次调用上重试**：那条命令可能已经在服务端执行过（内核也是这个口径，
         * 见 `retry_on_stale`），重试等于再跑一遍。
         */
        void markBroken() {
            broken = true;
            try {
                jedis.close();
            }
            catch (RuntimeException ignored) {
                // 已经坏了，关的时候再抛什么都无所谓
            }
        }

        /** 下一条命令开始前调用：坏过就重建（重建成功才算修好）。 */
        void ensureUsable() {
            if (!broken) {
                return;
            }
            Jedis fresh = new Jedis(host, port);
            authenticate(fresh, username, password);
            if (currentDb != 0) {
                fresh.select(currentDb);
            }
            jedis = fresh;
            broken = false;
        }
    }

    /** 认证（用户名可选：Redis 6+ 的 ACL）。`connect` 与「坏掉后重建」共用同一段。 */
    private static void authenticate(Jedis jedis, String username, String password) {
        if (password == null || password.isEmpty()) {
            return;
        }
        if (username != null && !username.isEmpty() && !"default".equals(username)) {
            jedis.auth(username, password);
        }
        else {
            jedis.auth(password);
        }
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
            // 主线程继续读下一行，否则 cancel 排不上队
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
        result.addProperty("agent", "redis");
        result.addProperty("generic", false);
        JsonArray keys = new JsonArray();
        keys.add("redis");
        result.add("agentKeys", keys);
        result.addProperty("driverVersion", "jedis");
        result.addProperty("javaVersion", System.getProperty("java.version"));
        return result;
    }

    private static JsonObject ping() {
        JsonObject result = new JsonObject();
        result.addProperty("pong", true);
        result.addProperty("sessions", SESSIONS.size());
        return result;
    }

    private static JsonObject connect(JsonObject request) {
        String sessionId = required(request, "sessionId");
        String host = required(request, "host");
        int port = request.has("port") && !request.get("port").isJsonNull() && request.get("port").getAsInt() > 0
                ? request.get("port").getAsInt()
                : 6379;
        String username = text(request, "username");
        String password = text(request, "password");
        int database = parseDatabase(text(request, "database"));

        Jedis jedis = new Jedis(host, port);
        try {
            authenticate(jedis, username, password);
            if (database != 0) {
                jedis.select(database);
            }
            jedis.ping();
            String version = readVersion(jedis);
            Session previous = SESSIONS.put(
                    sessionId, new Session(host, port, username, password, jedis, database));
            if (previous != null) {
                previous.jedis.close();
            }
            JsonObject result = new JsonObject();
            result.addProperty("serverVersion", version);
            result.addProperty("database", database);
            return result;
        }
        catch (JedisDataException e) {
            jedis.close();
            throw new AgentException("DBMIND-CONN-0003", "连接被拒绝：" + e.getMessage(),
                    "检查密码、ACL 用户与 protected-mode 配置");
        }
        catch (RuntimeException e) {
            jedis.close();
            throw new AgentException("DBMIND-CONN-0003", "连接失败：" + e.getMessage(),
                    e.getClass().getSimpleName());
        }
    }

    private static JsonElement query(String requestId, JsonObject request) {
        Session session = session(request);
        int timeoutMs = request.has("timeoutMs") ? request.get("timeoutMs").getAsInt() : 30_000;
        // 每条命令把**读超时**对齐内核给的 timeoutMs：Jedis 自己的默认值是 2 秒，
        // 那会把「内核允许 30 秒」偷偷变成「只能跑 2 秒」—— 而超时又会被报成断链。
        // 口径与 ES 宿主一致（它用 timeoutMs 设 HTTP 超时，见 dbmind-agent-elasticsearch）。
        session.ensureUsable();
        if (timeoutMs > 0) {
            session.jedis.getClient().setSoTimeout(timeoutMs);
        }
        String statement = required(request, "sql");
        List<String> tokens = CommandLine.tokenize(statement);
        if (tokens.isEmpty()) {
            throw new AgentException("DBMIND-QUERY-0001", "命令为空");
        }
        String name = tokens.get(0).toUpperCase(Locale.ROOT);
        if (STREAMING.contains(name)) {
            throw new AgentException(
                    "DBMIND-QUERY-0001",
                    "暂不支持流式/阻塞命令：" + name,
                    "本宿主是一次请求一次响应的模型：SUBSCRIBE / MONITOR / BLPOP / MULTI 这类命令\n"
                            + "需要长连接推送或阻塞等待，工作台里装不下。用单条命令做验证即可。");
        }

        AtomicBoolean cancelled = new AtomicBoolean(false);
        RUNNING.put(requestId, cancelled);
        try {
            if (cancelled.get()) {
                throw new AgentException("DBMIND-QUERY-0004", "查询已取消", null);
            }
            // SELECT 必须走状态化路径：直接 sendCommand 不会更新会话记录的当前库
            if ("SELECT".equals(name) && tokens.size() == 2) {
                int database = parseDatabase(tokens.get(1));
                session.jedis.select(database);
                session.currentDb = database;
                return scalarResult("OK");
            }
            Object reply = send(session.jedis, name, tokens);
            return render(name, reply, wantsPairs(name, tokens), cancelled);
        }
        catch (JedisDataException e) {
            throw new AgentException("DBMIND-QUERY-0002", "命令执行失败：" + e.getMessage(), null);
        }
        catch (JedisConnectionException e) {
            // 读超时**不是**「连接断了」：内核给的是 timeoutMs，把它说成断链会让用户
            // 去查网络（真机验证第一次就是这么报的）。与 ES 宿主同一套口径：
            // 超时归 QUERY-0003，并说清该调哪个配置。
            boolean timedOut = e.getCause() instanceof java.net.SocketTimeoutException;
            // 这条连接已经不可用（Jedis 会标 broken 并关掉它）⇒ 记下来，下一条命令前重建。
            session.markBroken();
            if (timedOut) {
                throw new AgentException("DBMIND-QUERY-0003",
                        "命令超过 " + timeoutMs + " ms 未返回，已中止",
                        "可提高连接配置里的查询超时，或缩小命令范围\n"
                                + "（这条会话已自动重建，下一条命令不受影响）");
            }
            // CONN-0003（连接失败），不是 0001（连接不存在）：内核只对 0003 清会话缓存并重连。
            throw new AgentException("DBMIND-CONN-0003", "连接已断开：" + e.getMessage(),
                    "宿主会按需要重连；若持续失败请检查网络与超时配置");
        }
        finally {
            RUNNING.remove(requestId);
        }
    }

    private static Object send(Jedis jedis, String name, List<String> tokens) {
        String[] args = tokens.subList(1, tokens.size()).toArray(new String[0]);
        Protocol.Command known = knownCommand(name);
        if (known != null) {
            return jedis.sendCommand(known, args);
        }
        // 未知命令名也要能发出去：Redis 的命令由服务端裁决，宿主不该做白名单
        ProtocolCommand custom = () -> name.getBytes(StandardCharsets.UTF_8);
        return jedis.sendCommand(custom, args);
    }

    private static Protocol.Command knownCommand(String name) {
        try {
            return Protocol.Command.valueOf(name);
        }
        catch (IllegalArgumentException e) {
            return null;
        }
    }

    private static JsonElement tables(JsonObject request) {
        Session session = session(request);
        JsonArray tables = new JsonArray();
        try {
            String info = session.jedis.info("keyspace");
            for (String line : info.split("\n")) {
                String trimmed = line.trim();
                if (trimmed.startsWith("db") && trimmed.contains(":")) {
                    String name = trimmed.substring(0, trimmed.indexOf(':'));
                    JsonObject table = new JsonObject();
                    table.addProperty("name", name);
                    table.addProperty("kind", "table");
                    tables.add(table);
                }
            }
            if (tables.isEmpty()) {
                // 没有任何键：仍把当前库列出来，否则界面是空的、用户不知道连到哪了
                JsonObject table = new JsonObject();
                table.addProperty("name", "db" + session.currentDb);
                table.addProperty("kind", "table");
                tables.add(table);
            }
            return tables;
        }
        catch (RuntimeException e) {
            // 测试替身/受限实例可能不支持 INFO：退化到「当前库」，不让浏览功能整体失效
            JsonObject table = new JsonObject();
            table.addProperty("name", "db" + session.currentDb);
            table.addProperty("kind", "table");
            tables.add(table);
            return tables;
        }
    }

    /**
     * 键空间浏览：把「库」当表、把**键**当列（类型即 Redis 类型）。
     *
     * <p>为什么这样映射：Redis 没有表与列，而内核的结构浏览是两级的。
     * 与其编一个假 schema，不如直接把键列出来 —— 这是 Redis 用户真正要看的东西。
     * 代价是必须有上限（{@link #MAX_KEYS}），否则百万键会拖死界面。
     */
    private static JsonElement columns(JsonObject request) {
        Session session = session(request);
        String table = required(request, "table");
        int targetDb = parseDatabase(table.startsWith("db") ? table.substring(2) : table);
        boolean switched = targetDb != session.currentDb;
        try {
            if (switched) {
                session.jedis.select(targetDb);
                session.currentDb = targetDb;
            }
            Map<String, String> types = new LinkedHashMap<>();
            String cursor = "0";
            ScanParams params = new ScanParams().match("*").count(200);
            do {
                ScanResult<String> batch = session.jedis.scan(cursor, params);
                cursor = batch.getCursor();
                for (String key : batch.getResult()) {
                    if (types.size() >= MAX_KEYS) {
                        break;
                    }
                    types.putIfAbsent(key, "key");
                }
                if (types.size() >= MAX_KEYS) {
                    break;
                }
            } while (!"0".equals(cursor));

            JsonArray columns = new JsonArray();
            for (Map.Entry<String, String> entry : types.entrySet()) {
                JsonObject column = new JsonObject();
                column.addProperty("name", entry.getKey());
                column.add("typeName", JsonNull.INSTANCE);
                column.addProperty("nullable", true);
                column.addProperty("primaryKey", false);
                column.add("defaultValue", JsonNull.INSTANCE);
                columns.add(column);
            }
            // 补类型（用流水线，避免 N 次往返）：顺序与 types 的键顺序一致
            redis.clients.jedis.Pipeline pipeline = session.jedis.pipelined();
            List<redis.clients.jedis.Response<String>> pending = new ArrayList<>();
            for (String key : types.keySet()) {
                pending.add(pipeline.type(key));
            }
            if (!pending.isEmpty()) {
                pipeline.sync();
            }
            for (int i = 0; i < pending.size(); i++) {
                JsonObject column = columns.get(i).getAsJsonObject();
                column.addProperty("typeName", pending.get(i).get());
            }
            return columns;
        }
        catch (RuntimeException e) {
            throw new AgentException("DBMIND-QUERY-0002", "读取键列表失败：" + e.getMessage(), null);
        }
        finally {
            // 还原会话原本的库：浏览结构不该改变用户后续命令的落点
            if (switched) {
                try {
                    session.jedis.select(session.initialDb);
                    session.currentDb = session.initialDb;
                }
                catch (RuntimeException ignored) {
                    // 还原失败不掩盖主流程结果
                }
            }
        }
    }

    private static JsonObject cancel(JsonObject request) {
        String requestId = required(request, "requestId");
        AtomicBoolean flag = RUNNING.get(requestId);
        boolean found = flag != null;
        if (flag != null) {
            flag.set(true);
        }
        JsonObject result = new JsonObject();
        result.addProperty("cancelled", found);
        result.addProperty("note", "Redis 命令通常毫秒级返回，取消只在命令开始前有效");
        return result;
    }

    private static JsonObject disconnect(JsonObject request) {
        String sessionId = required(request, "sessionId");
        Session session = SESSIONS.remove(sessionId);
        if (session != null) {
            session.jedis.close();
        }
        JsonObject result = new JsonObject();
        result.addProperty("closed", session != null);
        return result;
    }

    // ------------------------------------------------------------ 结果映射

    /**
     * 这条命令的回复是不是「成对」（field, value, field, value …）。
     *
     * <p>RESP2 里 `HGETALL` 把哈希拍成一个扁平数组，按单列渲染会变成
     * f1 / v1 上下四行 —— 明明是一张两列表，却被摊成一条竖线。
     * 这类命令不少（`CONFIG GET`、`ZRANGE … WITHSCORES`、`HRANDFIELD … WITHVALUES`），
     * 一条条判比让用户自己去数下标靠谱。
     */
    private static boolean wantsPairs(String command, List<String> tokens) {
        if (Set.of("HGETALL", "CONFIG").contains(command)) {
            return true;
        }
        boolean scoresOrValues = tokens.stream()
                .skip(1)
                .map(token -> token.toUpperCase(Locale.ROOT))
                .anyMatch(token -> "WITHSCORES".equals(token) || "WITHVALUES".equals(token));
        if (!scoresOrValues) {
            return false;
        }
        return Set.of("ZRANGE", "ZREVRANGE", "ZRANGEBYSCORE", "ZREVRANGEBYSCORE", "ZRANGEBYLEX",
                "ZREVRANGEBYLEX", "HRANDFIELD", "ZRANDMEMBER", "ZPOPMIN", "ZPOPMAX").contains(command);
    }

    private static JsonElement render(String command, Object reply, boolean pairs, AtomicBoolean cancelled) {
        if (cancelled.get()) {
            throw new AgentException("DBMIND-QUERY-0004", "查询已取消", null);
        }
        JsonObject result = new JsonObject();
        JsonArray notices = new JsonArray();
        JsonArray rows = new JsonArray();
        List<String> columns = new ArrayList<>();

        if (reply == null) {
            columns.add("value");
            rows.add(row(cell(null)));
            notices.add("命令返回空（键不存在，或没有匹配的数据）");
        }
        else if (reply instanceof List<?> list && pairs && list.size() % 2 == 0) {
            columns.add("field");
            columns.add("value");
            for (int i = 0; i + 1 < list.size(); i += 2) {
                rows.add(row(cell(list.get(i)), cell(list.get(i + 1))));
            }
        }
        else if (reply instanceof List<?> list) {
            columns.add("value");
            for (Object item : list) {
                rows.add(row(cell(item)));
            }
            // 成对命令返回奇数个元素本身就不正常，如实说明而不是悄悄丢掉最后一个
            if (pairs) {
                notices.add("回复元素个数为奇数，已按单列展示");
            }
        }
        else if (reply instanceof Map<?, ?> map) {
            columns.add("field");
            columns.add("value");
            for (Map.Entry<?, ?> entry : map.entrySet()) {
                rows.add(row(cell(entry.getKey()), cell(entry.getValue())));
            }
        }
        else {
            columns.add("value");
            rows.add(row(cell(reply)));
        }

        JsonArray columnArray = new JsonArray();
        for (String name : columns) {
            JsonObject column = new JsonObject();
            column.addProperty("name", name);
            column.add("typeName", JsonNull.INSTANCE);
            columnArray.add(column);
        }

        result.add("columns", columnArray);
        result.add("rows", rows);
        result.addProperty("rowCount", rows.size());
        result.addProperty("truncated", false);
        // 写命令的整数回复就是「影响行数」；读命令不编这个数
        if (isWriteReply(command, reply)) {
            result.addProperty("affectedRows", ((Number) reply).longValue());
        }
        else {
            result.add("affectedRows", JsonNull.INSTANCE);
        }
        result.add("notices", notices);
        return result;
    }

    private static boolean isWriteReply(String command, Object reply) {
        if (!(reply instanceof Number)) {
            return false;
        }
        Set<String> counting = Set.of("DEL", "UNLINK", "EXPIRE", "PEXPIRE", "SADD", "SREM", "HSET",
                "HDEL", "LPUSH", "RPUSH", "LPOP", "RPOP", "ZADD", "ZREM", "PFADD", "XADD", "XDEL");
        return counting.contains(command);
    }

    private static JsonArray row(JsonObject... cells) {
        JsonArray row = new JsonArray();
        for (JsonObject cell : cells) {
            row.add(cell);
        }
        return row;
    }

    /** 单值单行结果（`SELECT` / `PING` 这类状态回复）。 */
    private static JsonObject scalarResult(String value) {
        JsonObject result = new JsonObject();
        JsonArray columns = new JsonArray();
        JsonObject column = new JsonObject();
        column.addProperty("name", "value");
        column.add("typeName", JsonNull.INSTANCE);
        columns.add(column);
        result.add("columns", columns);
        JsonArray rows = new JsonArray();
        rows.add(row(cell(value)));
        result.add("rows", rows);
        result.addProperty("rowCount", 1);
        result.addProperty("truncated", false);
        result.add("affectedRows", JsonNull.INSTANCE);
        result.add("notices", new JsonArray());
        return result;
    }

    /** 值 → 内核 CellValue（与其它宿主一致：null/integer/real/text/blob）。 */
    private static JsonObject cell(Object value) {
        if (value == null) {
            return tagged("null");
        }
        if (value instanceof Boolean flag) {
            return integer(flag ? 1 : 0);
        }
        if (value instanceof Number number) {
            if (number instanceof Double || number instanceof Float) {
                return real(number.doubleValue());
            }
            return integer(number.longValue());
        }
        if (value instanceof byte[] bytes) {
            // Jedis 的通用命令路径把批量字符串回成 byte[]（Redis 的字符串是二进制安全的）。
            // 绝大多数是真文本：严格按 UTF-8 解码，解不出来才当二进制 ——
            // 否则每个 SET/GET 都会显示成 <blob N B>，工作台直接废掉。
            String decoded = tryDecodeUtf8(bytes);
            if (decoded != null) {
                return text(decoded);
            }
            JsonObject payload = new JsonObject();
            payload.addProperty("len", bytes.length);
            int take = Math.min(bytes.length, 256);
            byte[] slice = new byte[take];
            System.arraycopy(bytes, 0, slice, 0, take);
            payload.addProperty("previewBase64", Base64.getEncoder().encodeToString(slice));
            JsonObject cell = tagged("blob");
            cell.add("v", payload);
            return cell;
        }
        if (value instanceof List<?> list) {
            StringBuilder text = new StringBuilder("[");
            for (int i = 0; i < list.size(); i++) {
                if (i > 0) {
                    text.append(", ");
                }
                text.append(String.valueOf(list.get(i)));
            }
            return text(text.append(']').toString());
        }
        return text(String.valueOf(value));
    }

    /** 严格 UTF-8 解码：失败返回 null（交给 blob 分支）。 */
    private static String tryDecodeUtf8(byte[] bytes) {
        try {
            return StandardCharsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(java.nio.charset.CodingErrorAction.REPORT)
                    .onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT)
                    .decode(java.nio.ByteBuffer.wrap(bytes))
                    .toString();
        }
        catch (java.nio.charset.CharacterCodingException e) {
            return null;
        }
    }

    private static JsonObject tagged(String tag) {
        JsonObject cell = new JsonObject();
        cell.addProperty("t", tag);
        return cell;
    }

    private static JsonObject integer(long value) {
        JsonObject cell = tagged("integer");
        cell.addProperty("v", value);
        return cell;
    }

    private static JsonObject real(double value) {
        JsonObject cell = tagged("real");
        cell.addProperty("v", value);
        return cell;
    }

    private static JsonObject text(String value) {
        JsonObject cell = tagged("text");
        cell.addProperty("v", value);
        return cell;
    }

    // ------------------------------------------------------------ 工具

    private static int parseDatabase(String database) {
        if (database == null || database.isBlank()) {
            return 0;
        }
        String trimmed = database.trim();
        if (trimmed.startsWith("db")) {
            trimmed = trimmed.substring(2);
        }
        try {
            int value = Integer.parseInt(trimmed);
            if (value < 0) {
                throw new NumberFormatException(trimmed);
            }
            return value;
        }
        catch (NumberFormatException e) {
            // 这是**连接配置**问题，不是「连接不存在」：报 CONN-0002（配置无效）。
            throw new AgentException(
                    "DBMIND-CONN-0002",
                    "数据库要填库号（0-15）：" + database,
                    "Redis 用数字分库（SELECT <n>）；库名不是连接参数");
        }
    }

    private static String readVersion(Jedis jedis) {
        try {
            String info = jedis.info("server");
            for (String line : info.split("\n")) {
                String trimmed = line.trim();
                if (trimmed.startsWith("redis_version:")) {
                    return "Redis " + trimmed.substring("redis_version:".length()).trim();
                }
            }
        }
        catch (RuntimeException ignored) {
            // 有的实例/替身不给 INFO
        }
        return "Redis";
    }

    private static void shutdown() {
        for (Session session : SESSIONS.values()) {
            try {
                session.jedis.close();
            }
            catch (RuntimeException ignored) {
                // 关闭失败不阻塞退出
            }
        }
        SESSIONS.clear();
        WORKERS.shutdownNow();
    }

    private static Session session(JsonObject request) {
        String sessionId = required(request, "sessionId");
        Session session = SESSIONS.get(sessionId);
        if (session == null) {
            // 会话没了 ⇒ 对内核而言是**连接级失败**（0003，不是「连接不存在」的 0001）。
            throw new AgentException("DBMIND-CONN-0003", "会话不存在（宿主可能已重启）：" + sessionId,
                    "内核会按需要重新连接");
        }
        return session;
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

    static {
        // 驱动日志走 stderr；内核只读 stdout
        System.setErr(new java.io.PrintStream(java.io.OutputStream.nullOutputStream(), true,
                StandardCharsets.UTF_8));
    }

    /** agent 侧业务异常：带内核错误码。 */
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
}
