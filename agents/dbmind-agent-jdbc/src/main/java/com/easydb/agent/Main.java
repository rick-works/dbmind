package com.dbmind.agent;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.sql.Connection;
import java.sql.DatabaseMetaData;
import java.sql.PreparedStatement;
import java.sql.Driver;
import java.sql.ResultSet;
import java.sql.SQLException;
import java.sql.SQLTimeoutException;
import java.sql.Statement;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Properties;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * DBMind 通用 JDBC 驱动宿主。
 *
 * <p>职责只有四件：**加载驱动类、按内核给的 URL 连接、执行语句、把结果映射成协议形状**。
 * 「怎么连某个数据库」全部来自内核传来的 YAML 元数据（driverClass / urlTemplate），
 * 因此这个进程对具体数据库一无所知，新增数据库类型不需要重新构建它。
 *
 * <p>并发模型：读 stdin 的主线程只做分发，实际工作在线程池里跑 ——
 * 否则 `cancel` 请求会排在长查询后面，取消就永远不生效。
 */
public final class Main {

    private static final Map<String, Session> SESSIONS = new ConcurrentHashMap<>();

    /** 正在执行的一条语句：既要能硬中断，也要记住「这次取消是我们主动做的」。 */
    private static final class Running {
        final Statement statement;
        final AtomicBoolean canceled = new AtomicBoolean(false);

        Running(Statement statement) {
            this.statement = statement;
        }
    }

    /**
     * requestId -> 正在执行的语句，用于硬中断（`Statement.cancel()`）。
     *
     * <p>为什么要带 `canceled` 标记：语句被打断后抛什么异常**由驱动决定**，而 H2 / PostgreSQL
     * 的取消抛的是 `SQLTimeoutException`（SQLSTATE 57014 在 JDBC 里被归到「超时」那一类）——
     * 只看异常类型，用户点的「取消」就会被报成「查询超时」。另外三个专属宿主早就按
     * 「自己记账」处理了，这里补齐。
     */
    private static final Map<String, Running> RUNNING = new ConcurrentHashMap<>();

    private static final ExecutorService WORKERS = Executors.newCachedThreadPool(runnable -> {
        Thread thread = new Thread(runnable, "dbmind-agent-worker");
        thread.setDaemon(true);
        return thread;
    });

    /** 结构浏览时接受的 TABLE_TYPE 白名单（不同引擎的取值差异很大，故不放 "TABLE" 一档）。 */
    private static final String[] TABLE_TYPES = {
        "TABLE",
        "BASE TABLE",
        "VIEW",
        "MATERIALIZED VIEW",
        "FOREIGN TABLE",
        "SYNONYM",
        "SYSTEM TABLE",
    };

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
            } catch (RuntimeException e) {
                write(Protocol.failure("", "DBMIND-INTERNAL-0001", "请求不是合法 JSON", e.toString()));
                continue;
            }
            // 放到工作线程：主线程必须能继续读下一行，否则 cancel 无法插队
            WORKERS.submit(() -> dispatch(request));
        }
        // stdin EOF：父进程已退出
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
                case "setautocommit" -> setAutoCommit(request);
                case "commit" -> txEnd(request, true);
                case "rollback" -> txEnd(request, false);
                case "disconnect" -> disconnect(request);
                case "shutdown" -> {
                    // 顺序很重要：**先关闭会话（落盘）再回执**。
                    // 内核收到回执后可能立刻强杀进程；若先回执，H2 这类嵌入式引擎
                    // 来不及把最后一个 chunk 刷盘，最后一次写会静默丢失（实测踩过）。
                    shutdown();
                    write(Protocol.ok(id, ping()));
                    System.exit(0);
                    yield null;
                }
                default -> throw new Protocol.AgentException("DBMIND-INTERNAL-0001", "未知方法：" + method);
            };
            if (result != null) {
                write(Protocol.ok(id, result));
            }
        } catch (Protocol.AgentException e) {
            write(Protocol.failure(id, e.code(), e.getMessage(), e.detail()));
        } catch (Throwable e) {
            write(Protocol.failure(id, "DBMIND-QUERY-0002", internalMessage(e), stackTrace(e)));
        }
    }

    /**
     * 「内部错误」也要说清是什么错。
     *
     * <p>老写法只回一句 {@code agent 内部错误}，真正的原因留在 detail 里 —— 界面不显示，
     * 于是「驱动类装载失败」这种一眼能修的问题，表现成一个看起来无解的「内部错误」。
     * 实测：ClickHouse 拿到的是不含传递依赖的瘦包，装载时抛
     * {@code NoClassDefFoundError: org/slf4j/LoggerFactory}，用户看到的就是「内部错误」。
     *
     * <p>驱动装载失败会**额外给一句怎么办**：这类错误的根因（缺哪个类）只在**第一次**
     * 失败时有 cause，之后 JVM 只回「Could not initialize class」，光看异常链看不出来。
     */
    private static String internalMessage(Throwable e) {
        String cause = deepestCause(e);
        if (e instanceof NoClassDefFoundError
                || e instanceof ExceptionInInitializerError
                || e instanceof ClassNotFoundException) {
            return "驱动类无法装载：" + cause
                    + "。多半是驱动包不完整（缺传递依赖）—— 删掉 ~/.dbmind/drivers/<类型>/ 下的 jar 让它重新下载";
        }
        return "agent 内部错误：" + cause;
    }

    /** 异常链最深处那一层（最多 5 跳），带类名与消息。 */
    private static String deepestCause(Throwable e) {
        Throwable current = e;
        for (int i = 0; i < 5 && current.getCause() != null; i++) {
            current = current.getCause();
        }
        String message = current.getMessage();
        return current.getClass().getSimpleName()
                + (message == null || message.isEmpty() ? "" : ": " + message);
    }

    // ------------------------------------------------------------ 方法实现

    private static JsonObject handshake() {
        JsonObject result = new JsonObject();
        result.addProperty("protocolVersion", 1);
        result.addProperty("agent", "jdbc");
        // 通用宿主：不枚举 agentKey（那会变成第二份类型清单），内核按 YAML 决定用哪个驱动类
        result.addProperty("generic", true);
        result.add("agentKeys", new JsonArray());
        result.addProperty("javaVersion", System.getProperty("java.version"));
        result.addProperty("javaVendor", System.getProperty("java.vendor"));
        return result;
    }

    private static JsonObject ping() {
        JsonObject result = new JsonObject();
        result.addProperty("pong", true);
        result.addProperty("sessions", SESSIONS.size());
        result.addProperty("running", RUNNING.size());
        return result;
    }

    private static JsonObject connect(JsonObject request) {
        String sessionId = required(request, "sessionId");
        String agentKey = required(request, "agentKey");
        String driverClass = required(request, "driverClass");
        String url = required(request, "url");
        boolean readOnly = request.has("readOnly") && request.get("readOnly").getAsBoolean();
        List<String> jars = stringList(request, "driverJars");

        long startedAt = System.currentTimeMillis();
        Driver driver = DriverLoader.instantiate(agentKey, driverClass, jars);
        long afterLoad = System.currentTimeMillis();

        Properties properties = new Properties();
        if (hasText(request, "username")) {
            properties.setProperty("user", request.get("username").getAsString());
        }
        if (hasText(request, "password")) {
            properties.setProperty("password", request.get("password").getAsString());
        }
        // 连接级驱动参数（内核 `extra.params` 透传过来）：例如自签证书的 SQL Server
        // 需要 trustServerCertificate=true。刻意放在账号/口令**之后**，
        // 让显式参数能覆盖默认值（比如用参数指定别的 user）。
        if (request.has("params") && request.get("params").isJsonObject()) {
            for (Map.Entry<String, JsonElement> entry :
                    request.getAsJsonObject("params").entrySet()) {
                JsonElement value = entry.getValue();
                if (value == null || value.isJsonNull()) {
                    continue;
                }
                properties.setProperty(
                        entry.getKey(),
                        value.isJsonPrimitive() ? value.getAsString() : value.toString());
            }
        }
        // 刻意不把 readOnly 放进连接属性：它不是标准的 JDBC 连接属性，
        // H2 之类驱动会在 connect 阶段执行 SET READONLY 并直接抛错（实测）。
        // 只读由内核的安全闸门强制，驱动侧只做「尽力而为」的提示（见下方 setReadOnly）。

        Connection connection;
        try {
            connection = driver.connect(url, properties);
        } catch (SQLException e) {
            throw new Protocol.AgentException("DBMIND-CONN-0003", "连接失败：" + e.getMessage(), describe(e));
        }
        if (connection == null) {
            throw new Protocol.AgentException("DBMIND-CONN-0003", "驱动不接受该 URL", url);
        }
        long afterConnect = System.currentTimeMillis();
        try {
            connection.setReadOnly(readOnly);
        } catch (SQLException ignored) {
            // 部分驱动不支持只读提示；内核的策略闸门才是权威
        }

        Session previous = SESSIONS.put(sessionId, new Session(sessionId, agentKey, url, connection));
        if (previous != null) {
            previous.close();
        }

        JsonObject result = new JsonObject();
        result.addProperty("serverVersion", serverVersion(connection));
        result.addProperty("driverClass", driverClass);
        result.addProperty("url", url);
        // 分段计时：新建一条会话到底慢在哪一段。实测这条路径上单次建会话约 5.4 秒，
        // 而服务端 `skip_name_resolve` 已经是 ON（排除反向解析），所以必须靠数字定位。
        trace("connect 分段 加载驱动=" + (afterLoad - startedAt) + "ms 建连=" + (afterConnect - afterLoad)
                + "ms 只读+版本=" + (System.currentTimeMillis() - afterConnect) + "ms url=" + url);
        if (System.getenv("DBMIND_AGENT_TRACE") != null) {
            probeUrlVariants(driver, url, properties);
        }
        return result;
    }

    /**
     * 这条语句是否**只可能**返回结果集（值得设 setMaxRows）。
     *
     * <p>判定刻意保守：拿不准就返回 false（不设上限）。代价只是可能多拉一些行回来
     * （仍会被 {@link ResultMapper#read} 按 maxRows 截断），换来的是**任何写语句都不会
     * 被静默截断** —— 后者是数据正确性问题，前者只是性能。
     *
     * <p>`with`（CTE 里可能包着 DELETE / UPDATE）与 `call` / `exec`（存储过程内部可能
     * 既查又写）因此都不设上限。
     */
    private static boolean mayReturnRows(String sql) {
        String head = leadingKeyword(sql);
        return "select".equals(head)
                || "show".equals(head)
                || "explain".equals(head)
                || "describe".equals(head)
                || "desc".equals(head)
                || "values".equals(head)
                || "table".equals(head)
                || "pragma".equals(head);
    }

    /** 首个关键字（跳过空白、`--` 行注释、块注释与前置括号），转小写。 */
    private static String leadingKeyword(String sql) {
        if (sql == null) {
            return "";
        }
        int i = 0;
        int n = sql.length();
        while (i < n) {
            char c = sql.charAt(i);
            if (Character.isWhitespace(c) || c == '(') {
                i++;
            } else if (c == '-' && i + 1 < n && sql.charAt(i + 1) == '-') {
                while (i < n && sql.charAt(i) != '\n') {
                    i++;
                }
            } else if (c == '/' && i + 1 < n && sql.charAt(i + 1) == '*') {
                int end = sql.indexOf("*/", i + 2);
                i = end < 0 ? n : end + 2;
            } else {
                break;
            }
        }
        int start = i;
        while (i < n && (Character.isLetterOrDigit(sql.charAt(i)) || sql.charAt(i) == '_')) {
            i++;
        }
        return sql.substring(start, i).toLowerCase();
    }

    private static JsonElement query(String requestId, JsonObject request) {
        Session session = session(request);
        String sql = required(request, "sql");
        int maxRows = request.has("maxRows") ? request.get("maxRows").getAsInt() : 2000;
        int timeoutMs = request.has("timeoutMs") ? request.get("timeoutMs").getAsInt() : 30_000;

        // 内核要裸值紧凑行时才用（默认关闭：老内核不认这个格式）

        boolean compact = request.has("compact") && request.get("compact").getAsBoolean();
        session.touch();

        try (Statement statement = session.connection().createStatement()) {
            if (maxRows > 0 && mayReturnRows(sql)) {
                // 多取一行，用来判断「还有更多」而不是猜测。
                //
                // **只对有结果集的语句设**：SQL Server 的驱动会把 setMaxRows 也套在
                // DELETE / UPDATE 上，于是「清空表」只删掉 maxRows+1 行，接口还照样回
                // success —— 静默少删比直接报错危险得多。实测（mssql-jdbc 12.8.1）：
                // 菜单「清空表」传 maxRows=1 ⇒ 只删 2 行，表里剩下一大半数据。
                // 结果行数本来也由 ResultMapper.read 按 maxRows 截断，这里只是
                // 「别一次把几百万行拉回来」的省流手段，去掉对读语句的正确性没有影响。
                statement.setMaxRows(maxRows + 1);
            }
            if (timeoutMs > 0) {
                statement.setQueryTimeout(Math.max(1, timeoutMs / 1000));
            }
            Running running = new Running(statement);
            RUNNING.put(requestId, running);
            try {
                boolean hasResultSet = statement.execute(sql);
                if (hasResultSet) {
                    try (ResultSet resultSet = statement.getResultSet()) {
                        if (resultSet == null) {
                            return emptyResult(0);
                        }
                        if (compact) {
                            // 紧凑模式：行文本在这里直接透传，不再经过 Gson 的树。
                            // 自己写完响应后回 null，dispatch 看到 null 就不会再写一遍。
                            StringBuilder rowsJson = new StringBuilder(1 << 16);
                            JsonObject head = ResultMapper.readCompactInto(resultSet, maxRows, rowsJson);
                            write(Protocol.ok(requestId, head, rowsJson.toString()));
                            return null;
                        }
                        return ResultMapper.read(resultSet, maxRows);
                    }
                }
                int affected = statement.getUpdateCount();
                return emptyResult(Math.max(affected, 0));
            } catch (SQLException e) {
                // 归类必须放在**这里**：账本还在手上（下面的 finally 才摘掉）。
                // 我第一次把它写在外层 catch，那时 `RUNNING` 已经空了 —— 于是被取消的语句
                // 还是被判成「超时」（host 层那条测试当场抓住了这个偷懒）。
                Protocol.AgentException failure = queryFailure(running, e);
                if (isConnectionLost(e)) {
                    // 连接级失败 ⇒ 这条会话**不能再留在宿主里**：内核收到 CONN-0001 会清掉
                    // 自己的缓存，下一次调用会重新 connect。留着它只会让每一次调用都失败
                    //（内核不会为非幂等查询重试，见 agent_driver::with_lane）。
                    dropSession(session);
                }
                throw failure;
            } finally {
                RUNNING.remove(requestId);
            }
        } catch (SQLException e) {
            // 走到这里的是「建 statement / 关资源」阶段的失败：没有账本，只能按异常类型归类
            Protocol.AgentException failure = queryFailure(null, e);
            if (isConnectionLost(e)) {
                dropSession(session);
            }
            throw failure;
        }
    }

    /** 丢掉一条会话：从表里摘掉并关掉连接。内核下一次调用会重新 connect。 */
    private static void dropSession(Session session) {
        SESSIONS.remove(session.id());
        session.close();
    }

    // ------------------------------------------------------------ 事务控制（事务模式）
    //
    // 三个 RPC 走的都是 JDBC 标准接口（Connection.setAutoCommit / commit / rollback），
    // 对所有 agent 类型的数据源通用 —— 不碰任何方言语法（SET autocommit 那种只有 MySQL 系认）。
    // 会话仍按 sessionId 定位：内核的亲和泳道（limit=1）保证「同一个编辑器会话」永远
    // 落在同一条物理连接上，事务状态于是跨请求保留。连接断开重连后由内核重新对齐
    // autocommit（见 agent_driver::with_lane 的事务对齐钩子）。

    /** 关/开 autocommit：事务模式的 begin（autoCommit=false）与收尾恢复（true）共用。 */
    private static JsonElement setAutoCommit(JsonObject request) {
        Session session = session(request);
        boolean autoCommit = request.has("autoCommit") && request.get("autoCommit").getAsBoolean();
        session.touch();
        try {
            session.connection().setAutoCommit(autoCommit);
            JsonObject result = new JsonObject();
            result.addProperty("autoCommit", session.connection().getAutoCommit());
            // 探针：JDBC 层状态之外，再到**同一条连接**上问一次服务端的 @@autocommit。
            // 两者不一致 = 驱动只更新了本地状态（useLocalSessionState 类行为），事务模式就靠不住
            try (java.sql.Statement st = session.connection().createStatement();
                 java.sql.ResultSet rs = st.executeQuery("select @@autocommit, connection_id()")) {
                if (rs.next()) {
                    result.addProperty("serverAutocommit", rs.getInt(1));
                    result.addProperty("probeConnId", rs.getString(2));
                }
            } catch (SQLException ignored) {
                // 探针失败不影响主流程（个别库没有这个变量）
            }
            return result;
        } catch (SQLException e) {
            if (isConnectionLost(e)) {
                dropSession(session);
            }
            throw new Protocol.AgentException("DBMIND-QUERY-0002",
                    "setAutoCommit(" + autoCommit + ") 失败：" + e.getMessage(), describe(e));
        }
    }

    /** commit / rollback：挂在未提交事务上的连接由内核负责收尾。 */
    private static JsonElement txEnd(JsonObject request, boolean doCommit) {
        Session session = session(request);
        session.touch();
        try {
            if (doCommit) {
                session.connection().commit();
            } else {
                session.connection().rollback();
            }
            // 事务收尾后把连接交回 autocommit，回到普通执行模式的默认状态
            try {
                session.connection().setAutoCommit(true);
            } catch (SQLException ignored) {
                // 个别驱动在收尾后短暂拒绝状态切换；内核后续 begin 会再设一次
            }
            return ping();
        } catch (SQLException e) {
            if (isConnectionLost(e)) {
                dropSession(session);
            }
            throw new Protocol.AgentException("DBMIND-QUERY-0002",
                    (doCommit ? "commit" : "rollback") + " 失败：" + e.getMessage(), describe(e));
        }
    }

    /**
     * 这条 `SQLException` 是不是「连接没了」。
     *
     * <p>判据用标准的 **SQLState 类 `08`**（connection exception）—— 驱动普遍会给；
     * 驱动给 null 时退回到消息里的常见说法（H2 的 "connection is closed" 等）。
     * 之所以要单独判它：这类失败**不能**只当成「SQL 执行失败」（0002）—— 那条连接已经
     * 不可用了，用户会一直失败下去。
     */
    private static boolean isConnectionLost(SQLException e) {
        String state = e.getSQLState();
        if (state != null && state.startsWith("08")) {
            return true;
        }
        String text = e.getMessage() == null ? "" : e.getMessage().toLowerCase(java.util.Locale.ROOT);
        return text.contains("connection is closed") || text.contains("connection closed")
                || text.contains("connection reset") || text.contains("connection is broken")
                || text.contains("connection refused") || text.contains("communications link failure");
    }

    /**
     * 元数据读取期的 {@code SQLException}：**先判「这条连接是不是没了」**，再决定报哪个码。
     *
     * <p>与 {@link #queryFailure} 同一个道理，但元数据这条路原来漏了 —— 一律报
     * {@code DBMIND-QUERY-0002}（SQL 执行失败）。后果不只是文案不准：内核**只对 CONN-0003
     * 清会话缓存并重连**（见 {@code agent_driver::with_lane}），报成 0002 等于告诉内核
     * 「连接没问题，只是 SQL 错了」。实测（真机 MySQL）：{@code wait_timeout} 把连接切掉后，
     * 每次展开都失败在 {@code 读取表清单失败：No operations allowed after connection closed
     * （SQLState=08003）}，而且**永远好不了**，除非用户手动重连。
     *
     * <p>判成 CONN-0003 之后必须**顺手摘掉坏会话**（{@link #dropSession}）：内核只会
     * 「下一次调用重新 connect」，坏会话还留在表里的话，下一次拿到的还是它。
     */
    private static Protocol.AgentException metadataFailure(Session session, SQLException e, String what) {
        if (isConnectionLost(e)) {
            dropSession(session);
            return new Protocol.AgentException("DBMIND-CONN-0003", "连接已断开：" + e.getMessage(), describe(e));
        }
        return new Protocol.AgentException("DBMIND-QUERY-0002", what + "：" + e.getMessage(), describe(e));
    }

    /**
     * 把执行期的 `SQLException` 翻成协议错误码。
     *
     * <p>顺序有讲究：**先看我们自己记的账**（`running.canceled`），再看超时，最后看通用取消状态。
     * 因为 H2 / PostgreSQL 的「取消」与「超时」都报 SQLSTATE 57014 —— 只看异常类型的话，
     * 用户点「取消」会看到「查询超时」（host 层那条取消测试抓到的就是这个）。
     */
    private static Protocol.AgentException queryFailure(Running running, SQLException e) {
        if (running != null && running.canceled.get()) {
            return new Protocol.AgentException("DBMIND-QUERY-0004", "查询已取消", describe(e));
        }
        if (e instanceof SQLTimeoutException || "57014".equals(e.getSQLState())) {
            return new Protocol.AgentException("DBMIND-QUERY-0003", "查询超时", describe(e));
        }
        if (isCanceledByState(e)) {
            return new Protocol.AgentException("DBMIND-QUERY-0004", "查询已取消", describe(e));
        }
        if (isConnectionLost(e)) {
            // **不能**当成普通的「SQL 执行失败」（0002）：这条连接已经不可用了。
            // 报 **CONN-0003**（连接失败）而不是 CONN-0001 —— 内核的码表里 0001 是
            // 「连接不存在」、0003 才是「连接失败」，而**只有 0003 会让内核清掉会话缓存
            // 并重连**（见 agent_driver::with_lane 与 error.rs 的码表）。用错码的话，
            // 用户既看到错的故事，这条连接也永远不会自愈。
            return new Protocol.AgentException("DBMIND-CONN-0003", "连接已断开：" + e.getMessage(), describe(e));
        }
        return new Protocol.AgentException("DBMIND-QUERY-0002", "SQL 执行失败：" + e.getMessage(), describe(e));
    }

    private static JsonElement tables(JsonObject request) {
        Session session = session(request);
        session.touch();
        // 一行入口追踪：走的是哪条路、agentKey 与 URL 各是什么。
        trace("tables 进入 agentKey=" + session.agentKey()
                + " mysqlFamily=" + isMysqlFamily(session) + " url=" + session.url());
        // MySQL 系先走一条**普通 SQL**。
        //
        // 为什么：mysql / mariadb / doris 的 URL 上带着 `useCursorFetch=true`（为让同步大表走
        // 服务端游标，见 plugins/connection-types/mysql.yaml），这本身是对的 —— 问题在于
        // **驱动的元数据 API 会给自己内部的语句设 fetchSize**，于是每条元数据查询都被放大成
        // 「服务端预处理 + 游标开/取/关」。实测（真机 MySQL）：读一个 288 张表的库要 10.3 秒，
        // 而同一个连接读一个 **0 张表**的库也要 10.1 秒 —— 开销与数据量无关，是固定的往返次数。
        // 同一内核、同一套代码下，没带这个参数的 SQL Server 只要 244 毫秒。
        //
        // 一条普通 Statement（不设 fetchSize）不受 `useCursorFetch` 影响，所以这里直接查
        // information_schema：普通 SQL，一次往返就够。
        if (isMysqlFamily(session)) {
            JsonArray fast = tablesViaInformationSchema(session);
            if (fast != null) {
                return fast;
            }
        }
        try {
            DatabaseMetaData meta = session.connection().getMetaData();
            String[] scope = lookupScope(session.connection());
            JsonArray tables = new JsonArray();
            // JDBC 的 types 过滤是**精确匹配 TABLE_TYPE**：只写 "TABLE" 会漏掉
            // 按规范报 "BASE TABLE" 的引擎（DuckDB 实测如此，返回空清单）。
            // 这里给一份白名单，未列出的类型按「表」处理即可。
            try (ResultSet resultSet = meta.getTables(scope[0], scope[1], "%", TABLE_TYPES)) {
                while (resultSet.next()) {
                    JsonObject table = new JsonObject();
                    table.addProperty("name", resultSet.getString("TABLE_NAME"));
                    String type = resultSet.getString("TABLE_TYPE");
                    table.addProperty("kind", type != null && type.toUpperCase().contains("VIEW") ? "view" : "table");
                    tables.add(table);
                }
            }
            return tables;
        } catch (SQLException e) {
            throw metadataFailure(session, e, "读取表清单失败");
        }
    }

    private static JsonElement columns(JsonObject request) {
        Session session = session(request);
        String table = required(request, "table");
        session.touch();
        // 大小写敏感差异（H2/Derby 大写、PG 小写）用候选名兜一层，比让用户自己试要省事
        for (String candidate : caseVariants(table)) {
            JsonArray columns = readColumns(session, candidate);
            if (!columns.isEmpty()) {
                return columns;
            }
        }
        return new JsonArray();
    }

    /**
     * 取主键列名集合。
     *
     * 为什么不是一次调用：JDBC 的 `(catalog, schema)` 各家语义不同，同一张表上
     * 「`getColumns` 能返回、`getPrimaryKeys` 却返回空」是**真实存在**的
     * （MySQL 实测如此：两套实现对方言/参数组合的容忍度不一致）。
     * 这里按几种组合依次试，取第一个**非空**结果 —— 比赌某一种写法稳；
     * 某个组合不被驱动支持就吞掉异常试下一个（拿不到主键不该让整次取列失败）。
     */
    private static Set<String> readPrimaryKeys(DatabaseMetaData meta, String[] scope, String table) {
        Set<String> keys = new LinkedHashSet<>();
        String[][] combos = {
            {scope[0], scope[1]},
            {scope[0], null},
            {null, scope[1]},
            {null, null},
        };
        for (String[] combo : combos) {
            try (ResultSet rs = meta.getPrimaryKeys(combo[0], combo[1], table)) {
                while (rs.next()) {
                    keys.add(rs.getString("COLUMN_NAME"));
                }
                if (!keys.isEmpty()) {
                    break;
                }
            } catch (Exception ignored) {
                // 该组合不被驱动支持 → 试下一个
            }
        }
        return keys;
    }

    /**
     * 这条会话是不是「MySQL 系」（mysql / mariadb / doris）。
     *
     * <p>判据以**驱动 URL** 为准，agentKey 只作兜底。理由：要绕开的那个 `useCursorFetch`
     * 是驱动特有的参数，所以判断依据本来就应该「用的是哪个驱动」。而 agentKey 的取值受
     * 内核侧配置影响，和这里期望的字符串并不保证一致 —— 曾经因此让整条快路径**静默不生效**：
     * jar 换了、宿主也重启了、耗时却一点没变，从结果上看像「代码没编进去」，很难查。
     */
    private static boolean isMysqlFamily(Session session) {
        String url = session.url() == null ? "" : session.url();
        if (url.startsWith("jdbc:mysql:") || url.startsWith("jdbc:mariadb:")) {
            return true;
        }
        String key = session.agentKey();
        return "mysql".equals(key) || "mariadb".equals(key) || "doris".equals(key);
    }

    /**
     * MySQL 系的表清单快路径。
     *
     * <p>返回 {@code null} 表示「这条快路径用不了，请回退到通用元数据 API」—— **不是**「这个库没有表」。
     * 两者必须分开：真·空库是合法的正常结果（返回空数组就够了），而快路径不适用时必须让调用方
     * 走回退，否则会把「查不出来」静默变成「没有表」。
     */
    private static JsonArray tablesViaInformationSchema(Session session) {
        long startedAt = System.currentTimeMillis();
        try (Statement statement = session.connection().createStatement()) {
            // `database()` 为 NULL = 这条会话没指定默认库。此时下面的 where 条件永远不成立，
            // 会返回空数组 —— 那会把「没指定库」错报成「空库」。通用路径对 catalog=null 的
            // 语义是「列出所有库的表」，这里交回给它，保持行为一致。
            try (ResultSet probe = statement.executeQuery("select database()")) {
                if (probe.next() && probe.getString(1) == null) {
                    trace("tables 快路径跳过：会话没有默认库（database() 为 NULL），已回退");
                    return null;
                }
            }
            JsonArray tables = new JsonArray();
            try (ResultSet resultSet = statement.executeQuery(
                    "select TABLE_NAME, TABLE_TYPE from information_schema.TABLES"
                            + " where TABLE_SCHEMA = database()")) {
                while (resultSet.next()) {
                    JsonObject table = new JsonObject();
                    table.addProperty("name", resultSet.getString("TABLE_NAME"));
                    String type = resultSet.getString("TABLE_TYPE");
                    table.addProperty("kind", type != null && type.toUpperCase().contains("VIEW") ? "view" : "table");
                    tables.add(table);
                }
            }
            // 留一行追踪：这张表清单走的是快路径还是回退路径、各花了多久 ——
            // 排查「某个库为什么慢」时，没有这行只能靠猜。
            trace("tables 快路径命中 表数=" + tables.size()
                    + " 耗时=" + (System.currentTimeMillis() - startedAt) + "ms");
            return tables;
        } catch (SQLException | RuntimeException e) {
            // 权限不足 / 驱动不认 / 服务端没有 information_schema（老版本）都可能走到这里。
            // 不报错：让调用方回退到通用路径，行为与以前完全一致。
            trace("tables 快路径失败，已回退： " + e);
            return null;
        }
    }

    /**
     * 诊断用：把 URL 上的参数**逐个去掉**再各连一次，看是哪一项让建连变慢。
     *
     * <p>背景：实测 mysql / doris 的**新建会话**稳定要 ~5.1 秒，而已经逐项排除——
     * 纯 TCP 建连 29~47ms、DNS 3ms、服务端 `skip_name_resolve=ON`、驱动类加载 11ms、
     * 读版本 58ms、`InetAddress.getLocalHost()` 15ms —— 也就是说这 5 秒必须落在
     * **驱动 `connect()` 内部**。把参数一项项拿掉对比，是定位它最直接的办法。
     *
     * <p>**只在设了环境变量 `DBMIND_AGENT_TRACE` 时才跑**：正常运行时一行都不会执行，
     * 零开销。连接用完立刻关；失败只记耗时，绝不影响任何正常连接（连的就是同一个库、
     * 同一套凭证，只是参数不同）。
     */
    private static void probeUrlVariants(Driver driver, String url, Properties properties) {
        String[] drops = {
            "&useCursorFetch=true",
            "&serverTimezone=Asia/Shanghai",
            "&allowPublicKeyRetrieval=true",
            "&useUnicode=true&characterEncoding=UTF-8",
            "&rewriteBatchedStatements=true",
            "&socketTimeout=600000",
            "&connectTimeout=10000",
            "&useSSL=false",
        };
        trace("probe 开始：URL 变体对比（基线见上面 connect 分段里的「建连」）");
        for (String drop : drops) {
            String variant = url.replace(drop, "");
            if (variant.equals(url)) {
                trace("probe 去掉 " + drop + " → 该 URL 里没有这一项，跳过");
                continue;
            }
            long startedAt = System.currentTimeMillis();
            try (Connection probe = driver.connect(variant, properties)) {
                trace("probe 去掉 " + drop + " → " + (System.currentTimeMillis() - startedAt)
                        + "ms  连接成功");
            } catch (Exception e) {
                trace("probe 去掉 " + drop + " → " + (System.currentTimeMillis() - startedAt)
                        + "ms  ✗ " + e.getClass().getSimpleName());
            }
        }
        // 再连一次原样的 URL：确认「同一个 URL 连着建两条」是否都慢 ——
        // 若第二条变快，说明慢的是「进程内首次建连」这一次性动作，而不是 URL 参数。
        long again = System.currentTimeMillis();
        try (Connection probe = driver.connect(url, properties)) {
            trace("probe 原 URL 再连一次 → " + (System.currentTimeMillis() - again) + "ms  连接成功");
        } catch (Exception e) {
            trace("probe 原 URL 再连一次 → " + (System.currentTimeMillis() - again)
                    + "ms  ✗ " + e.getClass().getSimpleName());
        }
    }

    /**
     * 关键路径追踪：写一行到**位置确定**的文件。
     *
     * <p>为什么不直接 `System.err`：宿主 stderr 由内核重定向到
     * `~/.dbmind/logs/agent-jdbc.err.log`，而那条链路实测**取不到内容**（文件一直是 0 字节，
     * 进程确实在跑）—— 排查性能问题时最需要的信息恰好拿不到。这里改为写
     * `java.io.tmpdir` 下的固定文件名：这个位置由 JVM 决定，一定能找到。
     *
     * <p>只在「表清单」这条路径上写（一次请求一行），并且任何异常都吞掉 ——
     * 追踪不能影响正常请求。不需要时删掉这个文件即可。
     */
    private static void trace(String message) {
        try {
            java.nio.file.Path target = java.nio.file.Path.of(
                    System.getProperty("java.io.tmpdir", "."), "dbmind-agent-jdbc-trace.log");
            java.nio.file.Files.writeString(
                    target,
                    System.currentTimeMillis() + " " + message + System.lineSeparator(),
                    java.nio.file.StandardOpenOption.CREATE,
                    java.nio.file.StandardOpenOption.APPEND);
        } catch (Exception ignored) {
            // 追踪失败不影响任何功能
        }
    }

    private static JsonArray readColumns(Session session, String table) {
        // MySQL 系（mysql / mariadb / doris）先走 information_schema 快路径：
        // Doris 实测 JDBC getColumns 稳定返回空（而 information_schema 直查列类型齐全）——
        // 列类型一断，前端的表头类型 / 高级筛选 / 选中区汇总会跟着全部哑掉。
        if (isMysqlFamily(session)) {
            JsonArray fast = columnsViaInformationSchema(session, table);
            if (fast != null && !fast.isEmpty()) {
                return fast;
            }
        }
        JsonArray columns = new JsonArray();
        try {
            DatabaseMetaData meta = session.connection().getMetaData();
            String[] scope = lookupScope(session.connection());
            Set<String> primaryKeys = readPrimaryKeys(meta, scope, table);
            try (ResultSet resultSet = meta.getColumns(scope[0], scope[1], table, "%")) {
                while (resultSet.next()) {
                    JsonObject column = new JsonObject();
                    String name = resultSet.getString("COLUMN_NAME");
                    column.addProperty("name", name);
                    column.addProperty("typeName", resultSet.getString("TYPE_NAME"));
                    String nullable = resultSet.getString("IS_NULLABLE");
                    column.addProperty("nullable", nullable == null || !nullable.equalsIgnoreCase("NO"));
                    column.addProperty("primaryKey", primaryKeys.contains(name));
                    String defaultValue = resultSet.getString("COLUMN_DEF");
                    if (defaultValue == null) {
                        column.add("defaultValue", null);
                    } else {
                        column.addProperty("defaultValue", defaultValue);
                    }
                    columns.add(column);
                }
            }
        } catch (SQLException e) {
            throw metadataFailure(session, e, "读取列信息失败");
        }
        return columns;
    }

    /**
     * MySQL 系的列信息快路径：直接查 information_schema.COLUMNS。
     *
     * <p>与 {@link #tablesViaInformationSchema} 同一套约定：返回 {@code null} = 快路径
     * 用不了（回退到通用 JDBC 元数据 API），**不是**「这张表没有列」；空数组 =
     * 查询成功但没命中（比如会话没默认库、表不存在）—— 此时同样交回调用方处理。
     */
    private static JsonArray columnsViaInformationSchema(Session session, String table) {
        long startedAt = System.currentTimeMillis();
        try {
            JsonArray columns = new JsonArray();
            try (PreparedStatement ps = session.connection().prepareStatement(
                    "select COLUMN_NAME, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY"
                            + " from information_schema.COLUMNS"
                            + " where TABLE_SCHEMA = database() and TABLE_NAME = ?"
                            + " order by ORDINAL_POSITION")) {
                ps.setString(1, table);
                try (ResultSet rs = ps.executeQuery()) {
                    while (rs.next()) {
                        JsonObject column = new JsonObject();
                        String name = rs.getString("COLUMN_NAME");
                        column.addProperty("name", name);
                        column.addProperty("typeName", rs.getString("DATA_TYPE"));
                        String nullable = rs.getString("IS_NULLABLE");
                        column.addProperty("nullable", nullable == null || !nullable.equalsIgnoreCase("NO"));
                        // COLUMN_KEY = 'PRI' 即主键（MySQL 系的惯例位）
                        column.addProperty("primaryKey", "PRI".equalsIgnoreCase(rs.getString("COLUMN_KEY")));
                        String defaultValue = rs.getString("COLUMN_DEFAULT");
                        if (defaultValue == null) {
                            column.add("defaultValue", null);
                        } else {
                            column.addProperty("defaultValue", defaultValue);
                        }
                        columns.add(column);
                    }
                }
            }
            trace("columns 快路径命中 列数=" + columns.size()
                    + " 耗时=" + (System.currentTimeMillis() - startedAt) + "ms");
            return columns;
        } catch (SQLException | RuntimeException e) {
            // 权限不足 / 老版本没有 information_schema 等都可能走到这里：
            // 不报错，回退到通用路径，行为与以前完全一致。
            trace("columns 快路径失败，已回退：" + e);
            return null;
        }
    }

    private static JsonObject cancel(JsonObject request) {
        String requestId = required(request, "requestId");
        Running running = RUNNING.get(requestId);
        boolean cancelled = false;
        if (running != null) {
            // 先记账、再打断：顺序反了标记会丢（cancel() 会让查询线程立刻抛异常）
            running.canceled.set(true);
            try {
                running.statement.cancel();
                cancelled = true;
            } catch (SQLException e) {
                throw new Protocol.AgentException("DBMIND-QUERY-0002", "取消失败：" + e.getMessage(), describe(e));
            }
        }
        JsonObject result = new JsonObject();
        result.addProperty("cancelled", cancelled);
        return result;
    }

    private static JsonObject disconnect(JsonObject request) {
        String sessionId = required(request, "sessionId");
        Session session = SESSIONS.remove(sessionId);
        if (session != null) {
            session.close();
        }
        JsonObject result = new JsonObject();
        result.addProperty("closed", session != null);
        return result;
    }

    // ------------------------------------------------------------ 工具

    private static void shutdown() {
        for (Session session : SESSIONS.values()) {
            session.close();
        }
        SESSIONS.clear();
        WORKERS.shutdownNow();
    }

    private static Session session(JsonObject request) {
        String sessionId = required(request, "sessionId");
        Session session = SESSIONS.get(sessionId);
        if (session == null) {
            // 会话没了 ⇒ 对内核而言是**连接级失败**（0003）：它会清掉缓存、下一次重连。
            throw new Protocol.AgentException(
                    "DBMIND-CONN-0003",
                    "会话不存在（agent 可能已重启）：" + sessionId,
                    "内核会按需要重新连接");
        }
        return session;
    }

    /** 返回 [catalog, schema]：不同数据库把「库」放在不同位置，这里统一探测。 */
    private static String[] lookupScope(Connection connection) {
        String catalog = null;
        String schema = null;
        try {
            catalog = connection.getCatalog();
        } catch (SQLException ignored) {
            // 不支持 catalog
        }
        try {
            schema = connection.getSchema();
        } catch (SQLException | AbstractMethodError ignored) {
            // 不支持 schema（如 MySQL）
        }
        return new String[] {catalog, schema};
    }

    private static List<String> caseVariants(String table) {
        Set<String> variants = new LinkedHashSet<>();
        variants.add(table);
        variants.add(table.toUpperCase());
        variants.add(table.toLowerCase());
        return new ArrayList<>(variants);
    }

    private static JsonObject emptyResult(int affectedRows) {
        JsonObject result = new JsonObject();
        result.add("columns", new JsonArray());
        result.add("rows", new JsonArray());
        result.addProperty("rowCount", 0);
        result.addProperty("truncated", false);
        result.addProperty("affectedRows", affectedRows);
        result.add("notices", new JsonArray());
        return result;
    }

    private static String serverVersion(Connection connection) {
        try {
            DatabaseMetaData meta = connection.getMetaData();
            return meta.getDatabaseProductName() + " " + meta.getDatabaseProductVersion();
        } catch (SQLException e) {
            return null;
        }
    }

    /**
     * 「操作被取消」的通用 SQLState。
     *
     * <p>注意 57014 **不在这里**：H2 / PostgreSQL 的「取消」与「超时」都是 57014，
     * 光看它分不开，只能靠 `RUNNING` 里那份「是不是我们取消的」账本来分（见 query 的 catch）。
     */
    private static boolean isCanceledByState(SQLException e) {
        return "HY008".equals(e.getSQLState());
    }

    private static String describe(SQLException e) {
        StringBuilder builder = new StringBuilder();
        builder.append(e.getClass().getSimpleName());
        if (e.getSQLState() != null) {
            builder.append(", SQLState=").append(e.getSQLState());
        }
        builder.append(", errorCode=").append(e.getErrorCode());
        if (e.getNextException() != null) {
            builder.append("\nnext: ").append(e.getNextException().getMessage());
        }
        return builder.toString();
    }

    private static String stackTrace(Throwable e) {
        java.io.StringWriter writer = new java.io.StringWriter();
        e.printStackTrace(new java.io.PrintWriter(writer));
        return writer.toString();
    }

    private static String required(JsonObject request, String field) {
        if (!hasText(request, field)) {
            throw new Protocol.AgentException("DBMIND-INTERNAL-0001", "缺少必填参数：" + field);
        }
        return request.get(field).getAsString();
    }

    private static boolean hasText(JsonObject request, String field) {
        return request.has(field)
                && !request.get(field).isJsonNull()
                && !request.get(field).getAsString().isEmpty();
    }

    private static List<String> stringList(JsonObject request, String field) {
        List<String> values = new ArrayList<>();
        if (!request.has(field) || !request.get(field).isJsonArray()) {
            return values;
        }
        for (JsonElement element : request.getAsJsonArray(field)) {
            if (!element.isJsonNull()) {
                values.add(element.getAsString());
            }
        }
        return values;
    }

    private static void write(String line) {
        synchronized (OUT_LOCK) {
            out.println(line);
            out.flush();
        }
    }

    static {
        // 驱动可能往 stderr 打日志；内核只读 stdout，这里把 stderr 丢掉保持安静
        System.setErr(new java.io.PrintStream(java.io.OutputStream.nullOutputStream(), true, StandardCharsets.UTF_8));
    }
}
