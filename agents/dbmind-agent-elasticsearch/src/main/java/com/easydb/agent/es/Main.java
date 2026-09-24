package com.dbmind.agent.es;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * DBMind Elasticsearch 专属宿主。
 *
 * <p>它做的事很窄：**把用户写的 REST 请求转给集群，再把响应整理成表格**。
 *
 * <p>三个刻意的决定：
 * <ul>
 *   <li><b>不引入官方客户端</b>：客户端库把 ES 版本耦进来（8.x 客户端连 7.x 集群的
 *       某些接口会失败），而工作台要的只是「发请求、看响应」。JDK 自带的 HttpClient 更稳。</li>
 *   <li><b>只读判定不在这里</b>：那属于内核安全闸门（<code>es.rs</code>），宿主只执行。</li>
 *   <li><b>搜索响应用命中而非原样 JSON</b>：`_search` 的结果按命中铺成行
 *       （`_id` / `_index` + `_source` 字段），这才是用户要看的表格。</li>
 * </ul>
 */
public final class Main {

    private static final Gson GSON = new GsonBuilder().serializeNulls().create();
    private static final Map<String, Session> SESSIONS = new ConcurrentHashMap<>();
    private static final Map<String, AtomicBoolean> RUNNING = new ConcurrentHashMap<>();

    private static final HttpClient CLIENT = HttpClient.newBuilder()
            .connectTimeout(Duration.ofSeconds(10))
            .followRedirects(HttpClient.Redirect.NORMAL)
            .build();

    private static final ExecutorService WORKERS = Executors.newCachedThreadPool(runnable -> {
        Thread thread = new Thread(runnable, "dbmind-es-worker");
        thread.setDaemon(true);
        return thread;
    });

    private static final Object OUT_LOCK = new Object();
    private static PrintWriter out;

    private Main() {
    }

    private record Session(String base, String authorization, String serverVersion) {
    }

    /** HTTP 响应（状态码 + 正文）。 */
    private record Reply(int status, String body) {
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
        result.addProperty("agent", "elasticsearch");
        result.addProperty("generic", false);
        JsonArray keys = new JsonArray();
        keys.add("elasticsearch");
        result.add("agentKeys", keys);
        result.addProperty("driverVersion", "jdk-httpclient");
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
                : 9200;
        String username = text(request, "username");
        String password = text(request, "password");

        // 只按端口推断协议：443 → https，其余 http（自定义 CA/双向 TLS 暂不支持，如实写在这里）
        String scheme = port == 443 ? "https" : "http";
        String base = scheme + "://" + host + ":" + port;
        String authorization = null;
        if (username != null && !username.isEmpty()) {
            String raw = username + ":" + (password == null ? "" : password);
            authorization = "Basic " + Base64.getEncoder().encodeToString(raw.getBytes(StandardCharsets.UTF_8));
        }

        Session session = new Session(base, authorization, null);
        Reply reply = send(session, "GET", "/", null, 15_000);
        if (reply.status() == 401 || reply.status() == 403) {
            throw new AgentException("DBMIND-CONN-0003", "认证失败（HTTP " + reply.status() + "）",
                    "检查用户名/密码；ES 8.x 默认开启安全认证");
        }
        if (reply.status() >= 400) {
            throw new AgentException("DBMIND-CONN-0003", "连接失败（HTTP " + reply.status() + "）", brief(reply.body()));
        }
        String version = readVersion(reply.body());
        SESSIONS.put(sessionId, new Session(base, authorization, version));

        JsonObject result = new JsonObject();
        result.addProperty("serverVersion", version);
        result.addProperty("base", base);
        return result;
    }

    private static JsonElement query(String requestId, JsonObject request) {
        Session session = session(request);
        String statement = required(request, "sql");
        int timeoutMs = request.has("timeoutMs") ? request.get("timeoutMs").getAsInt() : 30_000;

        Statement parsed = Statement.parse(statement);
        int maxRows = request.has("maxRows") ? request.get("maxRows").getAsInt() : 2000;
        AtomicBoolean cancelled = new AtomicBoolean(false);
        RUNNING.put(requestId, cancelled);
        try {
            if (cancelled.get()) {
                throw new AgentException("DBMIND-QUERY-0004", "查询已取消", null);
            }
            Reply reply = send(session, parsed.method, parsed.path, parsed.body, timeoutMs);
            if (reply.status() >= 400) {
                throw fromErrorResponse(reply);
            }
            if (reply.body() == null || reply.body().isBlank()) {
                return scalarResult(reply.status() + " (空响应)");
            }
            JsonElement rendered = render(reply.body(), cancelled);
            if (rendered instanceof JsonObject object) {
                // 截断只表示「被 maxRows 砍了」——「命中总数多于本页」是另一回事，
                // 混为一谈会让内核提示出「超过 2000 行」这种假信息
                clip(object, maxRows);
            }
            return rendered;
        }
        finally {
            RUNNING.remove(requestId);
        }
    }

    /** 把 ES 的错误体翻成带类型的错误：ES 自己给的 reason 比我们猜的准。 */
    private static AgentException fromErrorResponse(Reply reply) {
        String type = null;
        String reason = null;
        try {
            JsonObject root = JsonParser.parseString(reply.body()).getAsJsonObject();
            JsonElement error = root.get("error");
            if (error != null && error.isJsonObject()) {
                JsonObject object = error.getAsJsonObject();
                type = object.has("type") ? object.get("type").getAsString() : null;
                reason = object.has("reason") ? object.get("reason").getAsString() : null;
            }
            else if (error != null && error.isJsonPrimitive()) {
                reason = error.getAsString();
            }
        }
        catch (RuntimeException ignored) {
            // 非 JSON 错误体：退化为纯文本
        }
        String message = "HTTP " + reply.status()
                + (type == null ? "" : " [" + type + "]")
                + (reason == null ? "" : " " + reason);
        String code = reply.status() == 401 || reply.status() == 403
                ? "DBMIND-CONN-0003"
                : "DBMIND-QUERY-0002";
        return new AgentException(code, message, brief(reply.body()));
    }

    private static JsonElement tables(JsonObject request) {
        Session session = session(request);
        Reply reply = send(session, "GET",
                "/_cat/indices?format=json&h=index,docs.count,store.size,health,status", null, 20_000);
        if (reply.status() >= 400) {
            throw fromErrorResponse(reply);
        }
        JsonArray tables = new JsonArray();
        JsonElement root = JsonParser.parseString(reply.body());
        if (root.isJsonArray()) {
            for (JsonElement element : root.getAsJsonArray()) {
                if (!element.isJsonObject()) {
                    continue;
                }
                JsonObject row = element.getAsJsonObject();
                JsonElement name = row.get("index");
                if (name == null || name.isJsonNull()) {
                    continue;
                }
                JsonObject table = new JsonObject();
                table.addProperty("name", name.getAsString());
                table.addProperty("kind", "table");
                tables.add(table);
            }
        }
        return tables;
    }

    private static JsonElement columns(JsonObject request) {
        Session session = session(request);
        String index = required(request, "table");
        Reply reply = send(session, "GET", "/" + index + "/_mapping", null, 20_000);
        if (reply.status() >= 400) {
            throw fromErrorResponse(reply);
        }
        JsonArray columns = new JsonArray();
        // _id 是 ES 里每个文档都有的字段，先列出来
        columns.add(column("_id", "keyword"));
        JsonElement root = JsonParser.parseString(reply.body());
        if (root.isJsonObject()) {
            for (Map.Entry<String, JsonElement> entry : root.getAsJsonObject().entrySet()) {
                JsonObject body = entry.getValue().isJsonObject() ? entry.getValue().getAsJsonObject() : null;
                if (body == null || !body.has("mappings")) {
                    continue;
                }
                JsonObject mappings = body.getAsJsonObject("mappings");
                if (mappings.has("properties")) {
                    flatten(mappings.getAsJsonObject("properties"), "", columns);
                }
            }
        }
        return columns;
    }

    /** 把 mapping 的 properties 铺平：嵌套对象用点号连接，子字段（fields）也列出来。 */
    private static void flatten(JsonObject properties, String prefix, JsonArray columns) {
        for (Map.Entry<String, JsonElement> entry : properties.entrySet()) {
            if (!entry.getValue().isJsonObject()) {
                continue;
            }
            JsonObject definition = entry.getValue().getAsJsonObject();
            String name = prefix.isEmpty() ? entry.getKey() : prefix + "." + entry.getKey();
            String type = definition.has("type")
                    ? definition.get("type").getAsString()
                    : (definition.has("properties") ? "object" : "unknown");
            columns.add(column(name, type));
            if (definition.has("properties")) {
                flatten(definition.getAsJsonObject("properties"), name, columns);
            }
            if (definition.has("fields") && definition.get("fields").isJsonObject()) {
                for (Map.Entry<String, JsonElement> sub : definition.getAsJsonObject("fields").entrySet()) {
                    String subType = sub.getValue().isJsonObject()
                            && sub.getValue().getAsJsonObject().has("type")
                                    ? sub.getValue().getAsJsonObject().get("type").getAsString()
                                    : "unknown";
                    columns.add(column(name + "." + sub.getKey(), subType));
                }
            }
        }
    }

    private static JsonObject cancel(JsonObject request) {
        String requestId = required(request, "requestId");
        AtomicBoolean flag = RUNNING.get(requestId);
        if (flag != null) {
            flag.set(true);
        }
        JsonObject result = new JsonObject();
        result.addProperty("cancelled", flag != null);
        result.addProperty("note", "已在途的 HTTP 请求无法中断；取消影响后续步骤，超时由 timeoutMs 控制");
        return result;
    }

    private static JsonObject disconnect(JsonObject request) {
        String sessionId = required(request, "sessionId");
        boolean removed = SESSIONS.remove(sessionId) != null;
        JsonObject result = new JsonObject();
        result.addProperty("closed", removed);
        return result;
    }

    // ------------------------------------------------------------ 响应渲染

    private static JsonElement render(String body, AtomicBoolean cancelled) {
        if (cancelled.get()) {
            throw new AgentException("DBMIND-QUERY-0004", "查询已取消", null);
        }
        JsonElement root = JsonParser.parseString(body);
        JsonArray notices = new JsonArray();

        // 1) 搜索响应：按命中铺成行（用户要的是表格，不是原样 JSON）
        if (root.isJsonObject()) {
            JsonObject object = root.getAsJsonObject();
            JsonElement hits = object.get("hits");
            if (hits != null && hits.isJsonObject() && hits.getAsJsonObject().has("hits")) {
                return renderSearch(hits.getAsJsonObject(), notices);
            }
        }

        // 2) 数组响应（_cat 系列带 format=json）
        if (root.isJsonArray()) {
            JsonArray array = root.getAsJsonArray();
            Map<String, Boolean> names = new LinkedHashMap<>();
            for (JsonElement element : array) {
                if (element.isJsonObject()) {
                    for (String key : element.getAsJsonObject().keySet()) {
                        names.putIfAbsent(key, Boolean.TRUE);
                    }
                }
            }
            JsonArray rows = new JsonArray();
            for (JsonElement element : array) {
                JsonArray row = new JsonArray();
                for (String name : names.keySet()) {
                    JsonElement value = element.isJsonObject() ? element.getAsJsonObject().get(name) : element;
                    row.add(cell(value));
                }
                rows.add(row);
            }
            return result(names, rows, null, notices);
        }

        // 3) 普通对象：响应本身就是一行
        if (root.isJsonObject()) {
            Map<String, Boolean> names = new LinkedHashMap<>();
            for (String key : root.getAsJsonObject().keySet()) {
                names.putIfAbsent(key, Boolean.TRUE);
            }
            JsonArray row = new JsonArray();
            for (String name : names.keySet()) {
                row.add(cell(root.getAsJsonObject().get(name)));
            }
            JsonArray rows = new JsonArray();
            rows.add(row);
            return result(names, rows, affectedRows(root.getAsJsonObject()), notices);
        }

        return scalarResult(root.isJsonNull() ? "null" : root.getAsString());
    }

    private static JsonObject renderSearch(JsonObject hits, JsonArray notices) {
        JsonArray hitArray = hits.getAsJsonArray("hits");
        Map<String, Boolean> names = new LinkedHashMap<>();
        names.put("_id", Boolean.TRUE);
        names.put("_index", Boolean.TRUE);
        for (JsonElement element : hitArray) {
            if (!element.isJsonObject()) {
                continue;
            }
            JsonElement source = element.getAsJsonObject().get("_source");
            if (source != null && source.isJsonObject()) {
                for (String key : source.getAsJsonObject().keySet()) {
                    names.putIfAbsent(key, Boolean.TRUE);
                }
            }
        }
        JsonArray rows = new JsonArray();
        for (JsonElement element : hitArray) {
            if (!element.isJsonObject()) {
                continue;
            }
            JsonObject hit = element.getAsJsonObject();
            JsonObject source = hit.get("_source") != null && hit.get("_source").isJsonObject()
                    ? hit.getAsJsonObject("_source")
                    : new JsonObject();
            JsonArray row = new JsonArray();
            for (String name : names.keySet()) {
                if ("_id".equals(name) || "_index".equals(name)) {
                    row.add(cell(hit.get(name)));
                }
                else {
                    row.add(cell(source.get(name)));
                }
            }
            rows.add(row);
        }
        // total 只在需要时给出（track_total_hits 未开时 ES 会返回关系型 total）
        long total = 0;
        boolean hasTotal = false;
        JsonElement totalElement = hits.get("total");
        if (totalElement != null && totalElement.isJsonObject() && totalElement.getAsJsonObject().has("value")) {
            total = totalElement.getAsJsonObject().get("value").getAsLong();
            hasTotal = true;
        }
        if (hasTotal && total > rows.size()) {
            notices.add("共命中 " + total + " 条，本次返回 " + rows.size() + " 条");
        }
        return result(names, rows, null, notices);
    }

    /** 按 maxRows 裁剪行（截断语义与 SQL 侧一致）。 */
    private static void clip(JsonObject result, int maxRows) {
        JsonArray rows = result.getAsJsonArray("rows");
        if (rows == null || maxRows <= 0 || rows.size() <= maxRows) {
            return;
        }
        JsonArray visible = new JsonArray();
        for (int i = 0; i < maxRows; i++) {
            visible.add(rows.get(i));
        }
        result.add("rows", visible);
        result.addProperty("rowCount", visible.size());
        result.addProperty("truncated", true);
    }

    private static Long affectedRows(JsonObject object) {
        JsonElement result = object.get("result");
        if (result != null && result.isJsonPrimitive()
                && java.util.Set.of("created", "updated", "deleted", "not_found").contains(result.getAsString())) {
            return "not_found".equals(result.getAsString()) ? 0L : 1L;
        }
        for (String field : new String[] {"deleted", "updated", "created", "total"}) {
            JsonElement value = object.get(field);
            if (value != null && value.isJsonPrimitive() && value.getAsJsonPrimitive().isNumber()) {
                return value.getAsLong();
            }
        }
        return null;
    }

    private static JsonObject result(Map<String, Boolean> names, JsonArray rows, Long affected, JsonArray notices) {
        JsonArray columns = new JsonArray();
        for (String name : names.keySet()) {
            JsonObject column = new JsonObject();
            column.addProperty("name", name);
            column.add("typeName", JsonNull.INSTANCE);
            columns.add(column);
        }
        JsonObject result = new JsonObject();
        result.add("columns", columns);
        result.add("rows", rows);
        result.addProperty("rowCount", rows.size());
        result.addProperty("truncated", false);
        if (affected == null) {
            result.add("affectedRows", JsonNull.INSTANCE);
        }
        else {
            result.addProperty("affectedRows", affected);
        }
        result.add("notices", notices);
        return result;
    }

    private static JsonObject scalarResult(String value) {
        Map<String, Boolean> names = new LinkedHashMap<>();
        names.put("value", Boolean.TRUE);
        JsonArray rows = new JsonArray();
        JsonArray row = new JsonArray();
        row.add(cell(new com.google.gson.JsonPrimitive(value)));
        rows.add(row);
        return result(names, rows, null, new JsonArray());
    }

    private static JsonElement cell(JsonElement value) {
        if (value == null || value.isJsonNull()) {
            return tagged("null");
        }
        if (value.isJsonObject() || value.isJsonArray()) {
            // 嵌套结构按紧凑 JSON 显示：表格里塞不进去，展开看文本更实际
            return text(GSON.toJson(value));
        }
        if (value.getAsJsonPrimitive().isBoolean()) {
            return integer(value.getAsBoolean() ? 1 : 0);
        }
        if (value.getAsJsonPrimitive().isNumber()) {
            double asDouble = value.getAsDouble();
            if (asDouble == Math.rint(asDouble) && Math.abs(asDouble) < 9.0e15) {
                return integer(value.getAsLong());
            }
            return real(asDouble);
        }
        return text(value.getAsString());
    }

    private static JsonObject column(String name, String type) {
        JsonObject column = new JsonObject();
        column.addProperty("name", name);
        column.addProperty("typeName", type);
        column.addProperty("nullable", !"_id".equals(name));
        column.addProperty("primaryKey", "_id".equals(name));
        column.add("defaultValue", JsonNull.INSTANCE);
        return column;
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

    // ------------------------------------------------------------ HTTP 与工具

    private static Reply send(Session session, String method, String path, String body, int timeoutMs) {
        String url = session.base() + path;
        HttpRequest.Builder builder = HttpRequest.newBuilder(URI.create(url))
                .timeout(Duration.ofMillis(Math.max(1_000, timeoutMs)))
                .header("Accept", "application/json");
        if (session.authorization() != null) {
            builder.header("Authorization", session.authorization());
        }
        if (body != null && !body.isBlank()) {
            builder.header("Content-Type", "application/json");
            builder.method(method, HttpRequest.BodyPublishers.ofString(body, StandardCharsets.UTF_8));
        }
        else if ("POST".equals(method) || "PUT".equals(method)) {
            builder.method(method, HttpRequest.BodyPublishers.noBody());
        }
        else {
            builder.method(method, HttpRequest.BodyPublishers.noBody());
        }
        try {
            HttpResponse<String> response = CLIENT.send(builder.build(), HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
            return new Reply(response.statusCode(), response.body());
        }
        catch (java.net.http.HttpTimeoutException e) {
            throw new AgentException("DBMIND-QUERY-0003", "请求超时：" + url,
                    "可提高连接配置里的查询超时，或缩小查询范围");
        }
        catch (java.io.IOException e) {
            // 连接级失败 ⇒ CONN-0003（「连接失败」），不是 0001（「连接不存在」）：
            // 内核只对 0003 清会话缓存并重连。
            throw new AgentException("DBMIND-CONN-0003", "请求失败：" + e.getMessage(), url);
        }
        catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AgentException("DBMIND-QUERY-0004", "请求被中断", null);
        }
    }

    private static String readVersion(String body) {
        try {
            JsonObject root = JsonParser.parseString(body).getAsJsonObject();
            JsonElement version = root.get("version");
            if (version != null && version.isJsonObject()) {
                JsonObject object = version.getAsJsonObject();
                String number = object.has("number") ? object.get("number").getAsString() : null;
                String distribution = object.has("distribution") ? object.get("distribution").getAsString() : "Elasticsearch";
                if (number != null) {
                    return distribution + " " + number;
                }
            }
        }
        catch (RuntimeException ignored) {
            // 非标准响应（例如网关返回的 HTML）
        }
        return "Elasticsearch";
    }

    private static String brief(String text) {
        if (text == null) {
            return null;
        }
        String one = text.replaceAll("\\s+", " ").trim();
        return one.length() > 300 ? one.substring(0, 300) + "…" : one;
    }

    private static void shutdown() {
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
