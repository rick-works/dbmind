package com.dbmind.deploy;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;

import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicLong;

/**
 * ES 的契约级替身：只实现宿主真正会调的端点。
 *
 * <pre>
 *   GET    /                         集群信息（版本）
 *   GET    /_cat/indices?format=json 索引清单
 *   GET    /{index}/_mapping         字段映射
 *   GET    /{index}/_search          搜索（命中来自替身里的文档状态）
 *   GET    /_search                  搜索全部索引
 *   POST   /{index}/_doc             写入（返回生成的 _id）
 *   PUT    /{index}/_doc/{id}        按 id 写入
 *   POST   /{index}/_update/{id}     按 id 局部更新（合并 {"doc": {…}} 的顶层字段）
 *   DELETE /{index}/_doc/{id}        删文档
 *   PUT    /{index}                  建索引
 *   DELETE /{index}                  删索引
 *   GET    /_cluster/health          集群健康
 * </pre>
 *
 * **为什么有状态**：就地编辑这条链要能回答「改完再查一次，是不是只有那一行变了」——
 * 一个只回固定响应的替身连「到底改没改到」都说不出来。初始状态就是原先写死的那两条
 * `logs` 命中，所以既有的冒烟断言不受影响。
 *
 * 边界必须说清楚：它验证的是「宿主怎么发请求、怎么解析响应」，**不验证 ES 自身的语义**
 *（查询 DSL、分词、分片都不存在；`_update` 只做顶层字段合并，不做脚本）。
 * 真正跑通 ES 语义要靠 deploy/ 下的 compose recipe。
 *
 * 参数：{@code [port] [--require-auth]}
 */
public final class EsTestServer {

    private static boolean requireAuth = false;

    /** index -> (id -> `_source` 的 JSON 文本)。同步包装：保证「读」与「改」不会互相撕裂。 */
    private static final Map<String, Map<String, String>> INDEXES =
            Collections.synchronizedMap(new LinkedHashMap<>());
    private static final AtomicLong SEQ = new AtomicLong(1);

    private EsTestServer() {
    }

    public static void main(String[] args) throws IOException {
        int port = 9201;
        for (String arg : args) {
            if ("--require-auth".equals(arg)) {
                requireAuth = true;
            }
            else if (arg.matches("\\d+")) {
                port = Integer.parseInt(arg);
            }
        }
        seed();
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", port), 0);
        server.createContext("/", EsTestServer::handle);
        // 真实 ES 是并发服务多请求的；`setExecutor(null)`（默认）只在**分发线程**里
        // 逐个处理，慢响应会互相排队 —— 那既不像真机，也让「占满 4 条会话」的窗口
        // 随实现漂移。给一个固定线程池（守护线程，进程随时可杀）。
        server.setExecutor(Executors.newFixedThreadPool(8, runnable -> {
            Thread thread = new Thread(runnable, "es-stub-worker");
            thread.setDaemon(true);
            return thread;
        }));
        server.start();
        System.out.println("es-test-server listening on http://127.0.0.1:" + port
                + (requireAuth ? " (auth required)" : ""));
        System.out.flush();
    }

    /** 初值 = 原先写死的那两条 `logs` 命中；外加 `users`，让 `_cat/indices` 与状态一致。 */
    private static void seed() {
        Map<String, String> logs = new LinkedHashMap<>();
        logs.put("1", "{\"message\":\"hello\",\"level\":\"info\",\"ts\":\"2026-09-17T10:00:00Z\"}");
        logs.put("2", "{\"message\":\"中文日志\",\"level\":\"warn\",\"user\":{\"id\":7,\"name\":\"alice\"}}");
        INDEXES.put("logs", logs);

        Map<String, String> users = new LinkedHashMap<>();
        users.put("1", "{\"id\":7,\"name\":\"alice\"}");
        users.put("2", "{\"id\":8,\"name\":\"bob\"}");
        INDEXES.put("users", users);
    }

    private static void handle(HttpExchange exchange) throws IOException {
        String method = exchange.getRequestMethod();
        String path = exchange.getRequestURI().getPath();
        String query = exchange.getRequestURI().getQuery();
        // 请求体必须读完，否则连接会被复用方看到半截数据
        String requestBody = new String(exchange.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);

        if (requireAuth) {
            List<String> auth = exchange.getRequestHeaders().get("Authorization");
            if (auth == null || auth.isEmpty()) {
                respond(exchange, 401, "{\"error\":{\"type\":\"security_exception\","
                        + "\"reason\":\"missing authentication credentials\"},\"status\":401}");
                return;
            }
        }

        // 按路径段分派：`/logs/_update/1` 这类形状用 startsWith/endsWith 判容易串档
        List<String> parts = new ArrayList<>();
        for (String segment : path.split("/")) {
            if (!segment.isEmpty()) {
                parts.add(segment);
            }
        }

        String body;
        int status = 200;
        if (parts.isEmpty()) {
            body = "{\"name\":\"es-stub\",\"cluster_name\":\"dbmind-stub\",\"version\":{"
                    + "\"number\":\"8.13.4\",\"distribution\":\"elasticsearch\"}}";
        }
        else if ("_cat".equals(parts.get(0))) {
            body = catIndices();
        }
        else if ("_cluster".equals(parts.get(0)) && parts.size() >= 2 && "health".equals(parts.get(1))) {
            body = "{\"cluster_name\":\"dbmind-stub\",\"status\":\"green\",\"number_of_nodes\":1,"
                    + "\"active_shards\":2,\"unassigned_shards\":0}";
        }
        else if (isSearch(parts)) {
            if (query != null && query.contains("slow_ms=")) {
                // 给宿主层测试造一条「慢响应」：取消链路（排队等会话、在途请求、
                // 渲染阶段检查令牌）要有一条**不立刻返回**的请求才验得出来。
                // 真实集群不会这么慢，这里只是个可调旋钮。
                try {
                    Thread.sleep(slowMillis(query));
                }
                catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                }
            }
            if (query != null && query.contains("error=true")) {
                status = 400;
                body = "{\"error\":{\"type\":\"parsing_exception\",\"reason\":\"unknown query [not_a_query]\"},\"status\":400}";
            }
            else if (parts.isEmpty()) {
                body = searchBody(indexNames());
            }
            else {
                body = searchBody(List.of(parts.get(0)));
            }
        }
        else if (parts.size() == 2 && "_mapping".equals(parts.get(1))) {
            body = mapping(parts.get(0));
        }
        else if (parts.size() == 3 && "_update".equals(parts.get(1))) {
            String index = parts.get(0);
            String id = parts.get(2);
            String existing = getDoc(index, id);
            if (existing == null) {
                // 真实 ES 的语义：文档不存在就 404 —— **不**在这里悄悄创建一份
                status = 404;
                body = "{\"_index\":\"" + index + "\",\"_id\":\"" + id + "\",\"error\":{"
                        + "\"type\":\"document_missing_exception\",\"reason\":\"[" + id
                        + "]: document missing\"},\"status\":404}";
            }
            else {
                // 只合并 {"doc": {…}} 里的**顶层字段** —— 与真实 ES 的 doc 语义一致
                LinkedHashMap<String, String> merged = Json.pairs(existing);
                String doc = Json.field(requestBody, "doc");
                if (doc != null) {
                    merged.putAll(Json.pairs(doc));
                }
                putDoc(index, id, Json.stringify(merged));
                body = "{\"_index\":\"" + index + "\",\"_id\":\"" + id
                        + "\",\"result\":\"updated\",\"_shards\":{\"total\":1,\"successful\":1}}";
            }
        }
        else if (parts.size() == 2 && "_doc".equals(parts.get(1))) {
            if (!"POST".equals(method) && !"PUT".equals(method)) {
                status = 405;
                body = "{\"error\":{\"type\":\"illegal_argument_exception\",\"reason\":\"method not allowed\"}}";
            }
            else {
                String index = parts.get(0);
                String id = "stub-" + SEQ.getAndIncrement();
                putDoc(index, id, requestBody);
                status = 201;
                body = "{\"_index\":\"" + index + "\",\"_id\":\"" + id
                        + "\",\"result\":\"created\",\"_shards\":{\"total\":1,\"successful\":1}}";
            }
        }
        else if (parts.size() == 3 && "_doc".equals(parts.get(1))) {
            String index = parts.get(0);
            String id = parts.get(2);
            if ("DELETE".equals(method)) {
                synchronized (INDEXES) {
                    Map<String, String> docs = INDEXES.get(index);
                    if (docs != null) {
                        docs.remove(id);
                    }
                }
                body = "{\"_index\":\"" + index + "\",\"_id\":\"" + id + "\",\"result\":\"deleted\"}";
            }
            else {
                putDoc(index, id, requestBody);
                status = 201;
                body = "{\"_index\":\"" + index + "\",\"_id\":\"" + id
                        + "\",\"result\":\"created\",\"_shards\":{\"total\":1,\"successful\":1}}";
            }
        }
        else if (parts.size() == 1 && "PUT".equals(method) && !parts.get(0).startsWith("_")) {
            String index = parts.get(0);
            synchronized (INDEXES) {
                INDEXES.computeIfAbsent(index, key -> new LinkedHashMap<>());
            }
            body = "{\"acknowledged\":true,\"shards_acknowledged\":true,\"index\":\"" + index + "\"}";
        }
        else if (parts.size() == 1 && "DELETE".equals(method) && !parts.get(0).startsWith("_")) {
            synchronized (INDEXES) {
                INDEXES.remove(parts.get(0));
            }
            body = "{\"acknowledged\":true}";
        }
        else {
            status = 404;
            body = "{\"error\":{\"type\":\"index_not_found_exception\",\"reason\":\"no such path \"},\"status\":404}";
        }
        respond(exchange, status, body);
    }

    /** 从查询串里读 `slow_ms=<毫秒>`（缺省 0）：宿主层测试造慢响应用。 */
    private static long slowMillis(String query) {
        for (String pair : query.split("&")) {
            if (pair.startsWith("slow_ms=")) {
                try {
                    return Math.max(0, Long.parseLong(pair.substring("slow_ms=".length())));
                }
                catch (NumberFormatException ignored) {
                    return 0;
                }
            }
        }
        return 0;
    }

    /** `{index}/_search` 与 `/_search`（全索引）都算搜索。 */
    private static boolean isSearch(List<String> parts) {
        return (parts.size() == 2 && "_search".equals(parts.get(1)))
                || (parts.size() == 1 && "_search".equals(parts.get(0)));
    }

    private static List<String> indexNames() {
        synchronized (INDEXES) {
            return new ArrayList<>(INDEXES.keySet());
        }
    }

    private static String getDoc(String index, String id) {
        synchronized (INDEXES) {
            Map<String, String> docs = INDEXES.get(index);
            return docs == null ? null : docs.get(id);
        }
    }

    private static void putDoc(String index, String id, String source) {
        synchronized (INDEXES) {
            INDEXES.computeIfAbsent(index, key -> new LinkedHashMap<>()).put(id, source);
        }
    }

    /**
     * 搜索响应：命中**逐条来自替身里的文档状态**。
     *
     * 就地编辑那一组断言要的正是这个 —— 改完再搜一次，看是不是只有目标文档变了。
     */
    private static String searchBody(List<String> indexes) {
        StringBuilder hits = new StringBuilder();
        int total = 0;
        synchronized (INDEXES) {
            for (String index : indexes) {
                Map<String, String> docs = INDEXES.get(index);
                if (docs == null) {
                    continue;
                }
                for (Map.Entry<String, String> doc : docs.entrySet()) {
                    if (hits.length() > 0) {
                        hits.append(',');
                    }
                    hits.append("{\"_index\":\"").append(index)
                            .append("\",\"_id\":\"").append(doc.getKey())
                            .append("\",\"_source\":").append(doc.getValue()).append('}');
                    total++;
                }
            }
        }
        return "{\"took\":3,\"timed_out\":false,\"hits\":{\"total\":{\"value\":" + total
                + ",\"relation\":\"eq\"},\"hits\":[" + hits + "]}}";
    }

    private static String catIndices() {
        StringBuilder out = new StringBuilder("[");
        synchronized (INDEXES) {
            for (Map.Entry<String, Map<String, String>> entry : INDEXES.entrySet()) {
                if (out.length() > 1) {
                    out.append(',');
                }
                out.append("{\"index\":\"").append(entry.getKey())
                        .append("\",\"docs.count\":\"").append(entry.getValue().size())
                        .append("\",\"store.size\":\"8kb\",\"health\":\"green\",\"status\":\"open\"}");
            }
        }
        return out.append(']').toString();
    }

    private static String mapping(String index) {
        return "{\"" + index + "\":{\"mappings\":{\"properties\":{"
                + "\"message\":{\"type\":\"text\",\"fields\":{\"keyword\":{\"type\":\"keyword\"}}},"
                + "\"level\":{\"type\":\"keyword\"},"
                + "\"ts\":{\"type\":\"date\"},"
                + "\"user\":{\"properties\":{\"id\":{\"type\":\"long\"},\"name\":{\"type\":\"keyword\"}}}"
                + "}}}}";
    }

    private static void respond(HttpExchange exchange, int status, String body) throws IOException {
        byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
        exchange.getResponseHeaders().add("Content-Type", "application/json; charset=UTF-8");
        exchange.sendResponseHeaders(status, bytes.length);
        try (OutputStream out = exchange.getResponseBody()) {
            out.write(bytes);
        }
    }

    /** 极小的 JSON 顶层工具：只需要「取顶层字段」与「按顶层字段合并」，不引通用解析器。 */
    static final class Json {

        private Json() {
        }

        /** 解析顶层 `"key": value` 对；value 保存**原文**（含嵌套结构）。 */
        static LinkedHashMap<String, String> pairs(String json) {
            LinkedHashMap<String, String> out = new LinkedHashMap<>();
            if (json == null) {
                return out;
            }
            int i = skipWhitespace(json, 0);
            if (i >= json.length() || json.charAt(i) != '{') {
                return out;
            }
            i++;
            while (i < json.length()) {
                i = skipWhitespace(json, i);
                if (i >= json.length() || json.charAt(i) == '}') {
                    break;
                }
                if (json.charAt(i) == ',') {
                    i++;
                    continue;
                }
                if (json.charAt(i) != '"') {
                    break;
                }
                int[] end = new int[1];
                String key = readRawString(json, i, end);
                if (key == null) {
                    break;
                }
                i = skipWhitespace(json, end[0]);
                if (i >= json.length() || json.charAt(i) != ':') {
                    break;
                }
                i = skipWhitespace(json, i + 1);
                int valueEnd = scanValue(json, i);
                if (valueEnd < 0) {
                    break;
                }
                out.put(key, json.substring(i, valueEnd));
                i = valueEnd;
            }
            return out;
        }

        static String field(String json, String name) {
            return pairs(json).get(name);
        }

        static String stringify(LinkedHashMap<String, String> pairs) {
            StringBuilder out = new StringBuilder("{");
            for (Map.Entry<String, String> entry : pairs.entrySet()) {
                if (out.length() > 1) {
                    out.append(',');
                }
                out.append('"').append(entry.getKey()).append("\":").append(entry.getValue());
            }
            return out.append('}').toString();
        }

        private static int skipWhitespace(String json, int i) {
            while (i < json.length() && Character.isWhitespace(json.charAt(i))) {
                i++;
            }
            return i;
        }

        /** 读一个字符串字面量，返回**原文**（保留转义），end[0] = 收尾引号之后的位置。 */
        private static String readRawString(String json, int i, int[] end) {
            if (i >= json.length() || json.charAt(i) != '"') {
                return null;
            }
            int start = i + 1;
            i = start;
            while (i < json.length()) {
                char current = json.charAt(i);
                if (current == '\\') {
                    i += 2;
                    continue;
                }
                if (current == '"') {
                    end[0] = i + 1;
                    return json.substring(start, i);
                }
                i++;
            }
            return null;
        }

        /** 跳过一个值（字符串 / 嵌套结构 / 标量），返回其结束位置。 */
        private static int scanValue(String json, int i) {
            if (i >= json.length()) {
                return -1;
            }
            char current = json.charAt(i);
            if (current == '"') {
                int[] end = new int[1];
                return readRawString(json, i, end) == null ? -1 : end[0];
            }
            if (current == '{' || current == '[') {
                int depth = 0;
                while (i < json.length()) {
                    char ch = json.charAt(i);
                    if (ch == '"') {
                        int[] end = new int[1];
                        if (readRawString(json, i, end) == null) {
                            return -1;
                        }
                        i = end[0];
                        continue;
                    }
                    if (ch == '{' || ch == '[') {
                        depth++;
                    }
                    else if (ch == '}' || ch == ']') {
                        depth--;
                        if (depth == 0) {
                            return i + 1;
                        }
                    }
                    i++;
                }
                return -1;
            }
            while (i < json.length() && json.charAt(i) != ',' && json.charAt(i) != '}') {
                i++;
            }
            return i;
        }
    }
}
