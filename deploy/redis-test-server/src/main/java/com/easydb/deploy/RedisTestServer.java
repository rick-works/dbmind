package com.dbmind.deploy;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import java.util.concurrent.ConcurrentHashMap;
import java.util.regex.Pattern;

/**
 * Redis 的契约级替身：用 JDK 自带的 ServerSocket 讲 RESP2。
 *
 * <pre>
 *   PING / CLIENT / SELECT / INFO [section] / DBSIZE / FLUSHDB / FLUSHALL
 *   TYPE / EXISTS / DEL / KEYS / SCAN [MATCH …] [COUNT …] [TYPE …]
 *   SET / GET
 *   HSET / HGET / HGETALL / HDEL
 *   LPUSH / RPUSH / LRANGE
 *   SADD / SMEMBERS
 *   ZADD / ZRANGE [WITHSCORES]
 * </pre>
 *
 * **为什么要有它**（与 Mongo / ES 两个替身同一理由）：Windows CI 上装不了 redis-server，
 * 而「断言写了却没人跑」正是这一层最容易出的问题。有了它，任何机器（含 CI）
 * 都能把 Redis 那段冒烟跑起来，不需要预装任何东西。
 *
 * 边界必须说清楚：它验证的是「宿主怎么发请求、怎么解析响应」——**客户端是真 Jedis**
 *（5.2.0），所以 RESP 编解码写错会当场炸，不是自说自话。但它**不验证 Redis 自身的语义**：
 * 过期、持久化、集群、事务、发布订阅都不存在。要验证那些得指向一台真的 redis-server
 *（用 {@code DBMIND_SMOKE_REDIS_PORT} 指过去即可，冒烟脚本对两者是同一套断言）。
 *
 * 两处刻意简化（写在这里，免得被误读成「支持」）：
 * - `INFO` 报的版本固定为 `7.2.0` —— 替身不对应任何真实版本，这个数字只用于显示；
 * - `SET` 忽略 `EX` / `NX` 之类选项（替身没有过期这个概念）。
 *
 * 参数：{@code [port]}（默认 6399）
 */
public final class RedisTestServer {

    private static final String WRONG_TYPE =
            "WRONGTYPE Operation against a key holding the wrong kind of value";

    /** db 序号 -> key -> 值。多连接共享（Redis 本来就是共享的）。 */
    private static final Map<Integer, Map<String, Value>> DATABASES = new ConcurrentHashMap<>();

    private RedisTestServer() {
    }

    public static void main(String[] args) throws IOException {
        int port = 6399;
        for (String arg : args) {
            if (arg.matches("\\d+")) {
                port = Integer.parseInt(arg);
            }
        }
        ServerSocket server = new ServerSocket();
        server.setReuseAddress(true);
        server.bind(new InetSocketAddress("127.0.0.1", port));
        System.out.println("redis-test-server listening on redis://127.0.0.1:" + port);
        System.out.flush();
        while (true) {
            Socket socket = server.accept();
            Thread thread = new Thread(() -> serve(socket), "redis-stub-client");
            thread.setDaemon(true);
            thread.start();
        }
    }

    private static void serve(Socket socket) {
        try (socket;
             InputStream in = new BufferedInputStream(socket.getInputStream());
             OutputStream out = new BufferedOutputStream(socket.getOutputStream())) {
            // SELECT 是**连接级**状态（宿主正是靠这条语义切库的），所以按连接保存
            int[] currentDb = {0};
            while (true) {
                List<String> command = readCommand(in);
                if (command == null) {
                    return; // 客户端断开
                }
                if (command.isEmpty()) {
                    continue;
                }
                if (!dispatch(command, currentDb, out)) {
                    out.flush();
                    return; // QUIT：回完 +OK 就关连接（真实 Redis 也是这个行为）
                }
                out.flush();
            }
        }
        catch (IOException closed) {
            // 连接结束：替身不需要为此报错
        }
    }

    /** 处理一条命令；返回 `false` 表示**这条连接该关了**（目前只有 QUIT）。 */
    private static boolean dispatch(List<String> command, int[] currentDb, OutputStream out) throws IOException {
        String name = command.get(0).toLowerCase(Locale.ROOT);
        Map<String, Value> db = database(currentDb[0]);
        switch (name) {
            case "ping" -> {
                if (command.size() > 1) {
                    writeBulk(out, command.get(1));
                }
                else {
                    writeStatus(out, "PONG");
                }
            }
            // Jedis 5 建连时会发 CLIENT SETINFO：实现成空操作，免得连接初始化报错
            case "client" -> writeStatus(out, "OK");
            // DEBUG SLEEP <秒>：真实 Redis 也有这条命令。宿主层需要一条「足够慢」的命令
            // 才能造出「第二条排队等会话」的场景（宿主并不知道它慢，照常一次请求一次响应）。
            // 替身是**每连接一个线程**，所以这里只睡住这一个连接，不影响别的连接。
            case "debug" -> {
                if (command.size() >= 3 && "sleep".equalsIgnoreCase(command.get(1))) {
                    long seconds = parseLong(command, 2, 0);
                    try {
                        Thread.sleep(Math.max(0, seconds) * 1000L);
                    }
                    catch (InterruptedException interrupted) {
                        Thread.currentThread().interrupt();
                    }
                    writeStatus(out, "OK");
                }
                else {
                    writeError(out, "ERR unknown DEBUG subcommand");
                }
            }
            case "select" -> {
                if (command.size() < 2 || !command.get(1).matches("\\d+")) {
                    writeError(out, "ERR invalid DB index");
                }
                else {
                    currentDb[0] = Integer.parseInt(command.get(1));
                    writeStatus(out, "OK");
                }
            }
            case "info" -> writeBulk(out, info(command.size() > 1 ? command.get(1) : ""));
            case "dbsize" -> writeInteger(out, db.size());
            case "flushdb" -> {
                db.clear();
                writeStatus(out, "OK");
            }
            case "flushall" -> {
                DATABASES.clear();
                writeStatus(out, "OK");
            }
            case "type" -> {
                Value value = command.size() > 1 ? db.get(command.get(1)) : null;
                writeStatus(out, value == null ? "none" : value.type);
            }
            case "exists" -> {
                long found = 0;
                for (int i = 1; i < command.size(); i++) {
                    if (db.containsKey(command.get(i))) {
                        found++;
                    }
                }
                writeInteger(out, found);
            }
            case "del" -> {
                long removed = 0;
                for (int i = 1; i < command.size(); i++) {
                    if (db.remove(command.get(i)) != null) {
                        removed++;
                    }
                }
                writeInteger(out, removed);
            }
            case "keys" -> {
                String pattern = command.size() > 1 ? command.get(1) : "*";
                List<String> keys = new ArrayList<>();
                for (String key : snapshot(db).keySet()) {
                    if (matches(pattern, key)) {
                        keys.add(key);
                    }
                }
                writeArray(out, keys);
            }
            case "scan" -> scan(command, db, out);
            case "set" -> {
                if (command.size() < 3) {
                    writeError(out, "ERR wrong number of arguments for 'set' command");
                }
                else {
                    db.put(command.get(1), Value.string(command.get(2)));
                    writeStatus(out, "OK");
                }
            }
            case "get" -> {
                Value value = command.size() > 1 ? db.get(command.get(1)) : null;
                if (value == null) {
                    writeBulk(out, null);
                }
                else if (!"string".equals(value.type)) {
                    writeError(out, WRONG_TYPE);
                }
                else {
                    writeBulk(out, value.text);
                }
            }
            case "hset" -> hset(command, db, out);
            case "hget" -> {
                Value value = command.size() > 1 ? db.get(command.get(1)) : null;
                if (value == null) {
                    writeBulk(out, null);
                }
                else if (!"hash".equals(value.type)) {
                    writeError(out, WRONG_TYPE);
                }
                else {
                    writeBulk(out, command.size() > 2 ? value.hash.get(command.get(2)) : null);
                }
            }
            case "hgetall" -> {
                Value value = command.size() > 1 ? db.get(command.get(1)) : null;
                if (value == null) {
                    writeArray(out, List.of());
                }
                else if (!"hash".equals(value.type)) {
                    writeError(out, WRONG_TYPE);
                }
                else {
                    // RESP2 的 HGETALL 是**扁平数组** field1,value1,field2,value2…
                    //（宿主据此识别「成对回复」并按两列展示）
                    List<String> flat = new ArrayList<>();
                    value.hash.forEach((field, item) -> {
                        flat.add(field);
                        flat.add(item);
                    });
                    writeArray(out, flat);
                }
            }
            case "hdel" -> {
                Value value = command.size() > 1 ? db.get(command.get(1)) : null;
                if (value == null || !"hash".equals(value.type)) {
                    writeInteger(out, 0);
                }
                else {
                    long removed = 0;
                    for (int i = 2; i < command.size(); i++) {
                        if (value.hash.remove(command.get(i)) != null) {
                            removed++;
                        }
                    }
                    writeInteger(out, removed);
                }
            }
            case "lpush", "rpush" -> {
                Value value = getOrCreate(db, arg(command, 1), "list", out);
                if (value != null) {
                    for (int i = 2; i < command.size(); i++) {
                        if ("lpush".equals(name)) {
                            value.list.add(0, command.get(i));
                        }
                        else {
                            value.list.add(command.get(i));
                        }
                    }
                    writeInteger(out, value.list.size());
                }
            }
            case "lrange" -> {
                Value value = command.size() > 1 ? db.get(command.get(1)) : null;
                if (value == null) {
                    writeArray(out, List.of());
                }
                else if (!"list".equals(value.type)) {
                    writeError(out, WRONG_TYPE);
                }
                else {
                    int size = value.list.size();
                    int from = clampIndex(parseLong(command, 2, 0), size);
                    int to = clampIndex(parseLong(command, 3, -1), size);
                    List<String> slice = from > to
                            ? List.of()
                            : new ArrayList<>(value.list.subList(from, to + 1));
                    writeArray(out, slice);
                }
            }
            case "sadd" -> {
                Value value = getOrCreate(db, arg(command, 1), "set", out);
                if (value != null) {
                    long added = 0;
                    for (int i = 2; i < command.size(); i++) {
                        if (value.set.add(command.get(i))) {
                            added++;
                        }
                    }
                    writeInteger(out, added);
                }
            }
            case "smembers" -> {
                Value value = command.size() > 1 ? db.get(command.get(1)) : null;
                if (value == null) {
                    writeArray(out, List.of());
                }
                else if (!"set".equals(value.type)) {
                    writeError(out, WRONG_TYPE);
                }
                else {
                    writeArray(out, new ArrayList<>(value.set));
                }
            }
            case "zadd" -> {
                Value value = getOrCreate(db, arg(command, 1), "zset", out);
                if (value != null) {
                    long added = 0;
                    for (int i = 2; i + 1 < command.size(); i += 2) {
                        if (value.zset.put(command.get(i + 1), command.get(i)) == null) {
                            added++;
                        }
                    }
                    writeInteger(out, added);
                }
            }
            case "zrange" -> {
                Value value = command.size() > 1 ? db.get(command.get(1)) : null;
                if (value == null) {
                    writeArray(out, List.of());
                }
                else if (!"zset".equals(value.type)) {
                    writeError(out, WRONG_TYPE);
                }
                else {
                    boolean withScores = command.stream().anyMatch("withscores"::equalsIgnoreCase);
                    List<String> members = new ArrayList<>(value.zset.keySet());
                    int size = members.size();
                    int from = clampIndex(parseLong(command, 2, 0), size);
                    int to = clampIndex(parseLong(command, 3, -1), size);
                    List<String> slice = new ArrayList<>();
                    if (from <= to) {
                        for (String member : members.subList(from, to + 1)) {
                            slice.add(member);
                            if (withScores) {
                                slice.add(value.zset.get(member));
                            }
                        }
                    }
                    writeArray(out, slice);
                }
            }
            // QUIT：回 +OK 然后**关掉连接**。宿主层需要能造一次「连接被切断」，
            // 用来验「断链之后这条会话还能自愈」（宿主层的 `宿主层_断链之后仍能自愈`）。
            case "quit" -> {
                writeStatus(out, "OK");
                return false;
            }
            // 未知命令由**服务端**裁决 —— 与真实 Redis 一致（宿主不替它猜）
            default -> writeError(out, "ERR unknown command '" + command.get(0) + "'");
        }
        return true;
    }

    private static void scan(List<String> command, Map<String, Value> db, OutputStream out) throws IOException {
        String pattern = null;
        String typeFilter = null;
        for (int i = 2; i < command.size(); i++) {
            String option = command.get(i).toLowerCase(Locale.ROOT);
            if ("match".equals(option) && i + 1 < command.size()) {
                pattern = command.get(++i);
            }
            else if ("type".equals(option) && i + 1 < command.size()) {
                typeFilter = command.get(++i).toLowerCase(Locale.ROOT);
            }
            else if ("count".equals(option) && i + 1 < command.size()) {
                i++; // 替身一次返回全部：COUNT 只当提示
            }
        }
        List<String> keys = new ArrayList<>();
        for (Map.Entry<String, Value> entry : snapshot(db).entrySet()) {
            if (pattern != null && !matches(pattern, entry.getKey())) {
                continue;
            }
            if (typeFilter != null && !typeFilter.equals(entry.getValue().type)) {
                continue;
            }
            keys.add(entry.getKey());
        }
        // 游标恒为 "0"：一次返回完（调用方拿到 0 就知道结束了）
        out.write("*2\r\n".getBytes(StandardCharsets.UTF_8));
        writeBulk(out, "0");
        writeArray(out, keys);
    }

    private static void hset(List<String> command, Map<String, Value> db, OutputStream out) throws IOException {
        Value value = getOrCreate(db, arg(command, 1), "hash", out);
        if (value == null) {
            return;
        }
        long added = 0;
        for (int i = 2; i + 1 < command.size(); i += 2) {
            if (value.hash.put(command.get(i), command.get(i + 1)) == null) {
                added++;
            }
        }
        writeInteger(out, added);
    }

    /**
     * 取该键的值；不存在就按期望类型建一个；**类型不符时写出 WRONGTYPE 并返回 null**
     *（与真实 Redis 一致：对一个字符串键执行 HSET 是错的，不是「新建一个」）。
     */
    private static Value getOrCreate(Map<String, Value> db, String key, String type, OutputStream out)
            throws IOException {
        Value existing = db.get(key);
        if (existing != null) {
            if (type.equals(existing.type)) {
                return existing;
            }
            writeError(out, WRONG_TYPE);
            return null;
        }
        Value created = Value.of(type);
        db.put(key, created);
        return created;
    }

    private static String arg(List<String> command, int index) {
        return index < command.size() ? command.get(index) : "";
    }

    private static String info(String section) {
        String wanted = section == null ? "" : section.toLowerCase(Locale.ROOT);
        boolean all = wanted.isEmpty() || "all".equals(wanted) || "default".equals(wanted);
        StringBuilder out = new StringBuilder();
        if (all || "server".equals(wanted)) {
            out.append("# Server\r\n")
                    .append("redis_version:7.2.0\r\n")
                    .append("redis_mode:standalone\r\n")
                    .append("os:dbmind-stub\r\n")
                    .append("\r\n");
        }
        if (all || "keyspace".equals(wanted)) {
            out.append("# Keyspace\r\n");
            for (Map.Entry<Integer, Map<String, Value>> entry : new TreeMap<>(DATABASES).entrySet()) {
                if (!entry.getValue().isEmpty()) {
                    out.append("db").append(entry.getKey())
                            .append(":keys=").append(entry.getValue().size())
                            .append(",expires=0,avg_ttl=0\r\n");
                }
            }
        }
        return out.toString();
    }

    private static Map<String, Value> database(int index) {
        return DATABASES.computeIfAbsent(index, key -> new ConcurrentHashMap<>());
    }

    /** 快照：并发下遍历不炸，且顺序稳定。 */
    private static Map<String, Value> snapshot(Map<String, Value> db) {
        return new TreeMap<>(db);
    }

    private static long parseLong(List<String> command, int index, long fallback) {
        if (index >= command.size()) {
            return fallback;
        }
        try {
            return Long.parseLong(command.get(index));
        }
        catch (NumberFormatException invalid) {
            return fallback;
        }
    }

    /** Redis 的下标语义：负数从尾部数。 */
    private static int clampIndex(long raw, int size) {
        long index = raw < 0 ? size + raw : raw;
        if (index < 0) {
            return 0;
        }
        return index >= size ? size - 1 : (int) index;
    }

    /** 只支持 `*` 与 `?`：够 KEYS / SCAN 的 MATCH 用。 */
    private static boolean matches(String pattern, String key) {
        if (pattern == null || "*".equals(pattern)) {
            return true;
        }
        StringBuilder regex = new StringBuilder();
        for (char current : pattern.toCharArray()) {
            if (current == '*') {
                regex.append(".*");
            }
            else if (current == '?') {
                regex.append('.');
            }
            else {
                regex.append(Pattern.quote(String.valueOf(current)));
            }
        }
        return key.matches(regex.toString());
    }

    // ------------------------------------------------------------ RESP 读写

    /** 读一条命令；返回 null 表示连接已断开。 */
    private static List<String> readCommand(InputStream in) throws IOException {
        int first = in.read();
        if (first < 0) {
            return null;
        }
        if (first != '*') {
            // 内联命令（redis-cli 的写法）：读到行尾按空白切
            List<String> inline = new ArrayList<>();
            for (String part : readLine(in, first).trim().split("\\s+")) {
                if (!part.isEmpty()) {
                    inline.add(part);
                }
            }
            return inline;
        }
        int count = readNumberLine(in);
        List<String> command = new ArrayList<>(Math.max(count, 0));
        for (int i = 0; i < count; i++) {
            int marker = in.read();
            if (marker != '$') {
                return null; // 只支持 bulk 参数（Jedis 只会这么发）
            }
            int length = readNumberLine(in);
            byte[] payload = in.readNBytes(length);
            in.skipNBytes(2); // 结尾的 \r\n
            command.add(new String(payload, StandardCharsets.UTF_8));
        }
        return command;
    }

    private static String readLine(InputStream in, int first) throws IOException {
        StringBuilder line = new StringBuilder();
        line.append((char) first);
        int current;
        while ((current = in.read()) >= 0) {
            if (current == '\r') {
                in.read(); // 吃掉 \n
                break;
            }
            line.append((char) current);
        }
        return line.toString();
    }

    private static int readNumberLine(InputStream in) throws IOException {
        String line = readLine(in, in.read()).trim();
        try {
            return Integer.parseInt(line);
        }
        catch (NumberFormatException invalid) {
            return 0;
        }
    }

    private static void writeStatus(OutputStream out, String status) throws IOException {
        out.write(("+" + status + "\r\n").getBytes(StandardCharsets.UTF_8));
    }

    private static void writeError(OutputStream out, String message) throws IOException {
        out.write(("-" + message + "\r\n").getBytes(StandardCharsets.UTF_8));
    }

    private static void writeInteger(OutputStream out, long value) throws IOException {
        out.write((":" + value + "\r\n").getBytes(StandardCharsets.UTF_8));
    }

    /** null 写成 RESP 的 nil（`$-1`）—— 调用方据此表达「没有这个键」。 */
    private static void writeBulk(OutputStream out, String value) throws IOException {
        if (value == null) {
            out.write("$-1\r\n".getBytes(StandardCharsets.UTF_8));
            return;
        }
        byte[] payload = value.getBytes(StandardCharsets.UTF_8);
        out.write(("$" + payload.length + "\r\n").getBytes(StandardCharsets.UTF_8));
        out.write(payload);
        out.write("\r\n".getBytes(StandardCharsets.UTF_8));
    }

    private static void writeArray(OutputStream out, List<String> values) throws IOException {
        out.write(("*" + values.size() + "\r\n").getBytes(StandardCharsets.UTF_8));
        for (String value : values) {
            writeBulk(out, value);
        }
    }

    /** 一个键的值。类型用字符串表示（与 `TYPE` 的回复同形）。 */
    private static final class Value {

        final String type;
        String text;
        final Map<String, String> hash = new LinkedHashMap<>();
        final List<String> list = new ArrayList<>();
        final LinkedHashSet<String> set = new LinkedHashSet<>();
        /** member -> score（替身保持插入序；真实 Redis 按分数排） */
        final Map<String, String> zset = new LinkedHashMap<>();

        private Value(String type) {
            this.type = type;
        }

        static Value of(String type) {
            return new Value(type);
        }

        static Value string(String value) {
            Value created = new Value("string");
            created.text = value;
            return created;
        }
    }
}
