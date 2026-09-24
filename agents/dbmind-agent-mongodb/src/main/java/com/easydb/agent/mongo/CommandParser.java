package com.dbmind.agent.mongo;

import org.bson.Document;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/**
 * 把「用户写的语句」翻译成 Mongo 命令文档。
 *
 * <p>支持三种写法（与 mongosh 的常见用法对齐）：
 * <pre>
 *   1) 原生命令      { "find": "users", "filter": { "age": 30 } }
 *   2) 集合方法      db.users.find({ age: 30 }).sort({ age: 1 }).limit(10)
 *   3) 命令方法      db.runCommand({ "ping": 1 })  /  db.adminCommand({ ... })
 *   外加：use &lt;db&gt; 切库
 * </pre>
 *
 * <p>**不支持的写法一律明确报错**，并列出支持的清单。Mongo 的写法极多，
 * 猜错的代价是「执行了用户没想执行的命令」——这类静默错误比报错难查得多。
 */
public final class CommandParser {

    /** 解析结果：命令 + 目标库/集合 + 给用户的提示。command 为 null 表示只切库。 */
    public record Parsed(String database, String collection, Document command, String notice) {
    }

    private CommandParser() {
    }

    public static Parsed parse(String statement, String defaultDatabase) {
        String text = stripComments(statement).trim();
        if (text.isEmpty()) {
            throw error("语句为空");
        }

        String lower = text.toLowerCase(Locale.ROOT);
        if (lower.startsWith("use ")) {
            String database = text.substring(4).trim();
            if (database.isEmpty()) {
                throw error("use 后面要跟数据库名");
            }
            return new Parsed(database, null, null, "当前数据库已切换为 " + database);
        }

        if (text.startsWith("{")) {
            Document command = parseDocument(text, "命令");
            if (command.isEmpty()) {
                throw error("命令不能为空");
            }
            return new Parsed(null, null, command, null);
        }

        if (!lower.startsWith("db.")) {
            throw error("无法识别的语句：" + brief(text));
        }

        List<String> chain = splitTopLevel(text, '.');
        // chain[0] == "db"
        if (chain.size() < 2) {
            throw error("语句不完整：" + brief(text));
        }

        String head = chain.get(1);
        int methodStart;
        String collection;
        String database = null;
        Document direct = null;

        if (head.toLowerCase(Locale.ROOT).startsWith("getcollection(")) {
            collection = unwrapArg(innerArgs(head).get(0));
            methodStart = 2;
        }
        else if (head.toLowerCase(Locale.ROOT).startsWith("runcommand(")) {
            direct = parseDocument(innerArgs(head).get(0), "runCommand");
            collection = null;
            methodStart = 2;
        }
        else if (head.toLowerCase(Locale.ROOT).startsWith("admincommand(")) {
            direct = parseDocument(innerArgs(head).get(0), "adminCommand");
            collection = null;
            database = "admin";
            methodStart = 2;
        }
        else if (head.contains("(")) {
            // db.<method>(...) —— 没有集合名的库级命令（dropDatabase / serverStatus …）
            direct = null;
            collection = null;
            methodStart = 1;
        }
        else {
            collection = head;
            methodStart = 2;
        }

        if (direct != null) {
            return new Parsed(database, null, direct, null);
        }

        Document command = null;
        if (methodStart == 1) {
            command = libraryCommand(chain.get(1));
        }
        else {
            if (chain.size() <= methodStart) {
                throw error("缺少要执行的方法：" + brief(text));
            }
            command = collectionCommand(collection, chain.get(methodStart), chain, methodStart);
        }

        return new Parsed(database, collection, command, null);
    }

    // ---------------------------------------------------------- 库级方法

    private static Document libraryCommand(String token) {
        List<String> args = innerArgs(token);
        String method = methodName(token);
        return switch (method) {
            case "dropdatabase" -> new Document("dropDatabase", 1);
            case "listcollections" -> new Document("listCollections", 1);
            case "serverstatus" -> new Document("serverStatus", 1);
            case "stats" -> new Document("dbStats", 1);
            case "ping" -> new Document("ping", 1);
            case "version", "buildinfo" -> new Document("buildInfo", 1);
            default -> throw unsupported("库级方法", method, args);
        };
    }

    // ---------------------------------------------------------- 集合方法

    private static Document collectionCommand(
            String collection, String head, List<String> chain, int methodStart) {
        String method = methodName(head).toLowerCase(Locale.ROOT);
        List<String> args = innerArgs(head);

        Document command = new Document();
        switch (method) {
            case "find" -> {
                command.put("find", collection);
                command.put("filter", argDocument(args, 0, new Document()));
                Document projection = argDocumentOrNull(args, 1);
                if (projection != null) {
                    command.put("projection", projection);
                }
            }
            case "findone" -> {
                command.put("find", collection);
                command.put("filter", argDocument(args, 0, new Document()));
                command.put("limit", 1);
            }
            case "aggregate" -> {
                command.put("aggregate", collection);
                command.put("pipeline", argArray(args, 0));
                command.put("cursor", new Document());
            }
            case "count", "countdocuments", "estimateddocumentcount" -> {
                command.put("count", collection);
                command.put("query", argDocument(args, 0, new Document()));
            }
            case "distinct" -> {
                if (args.isEmpty()) {
                    throw error("distinct 需要字段名");
                }
                command.put("distinct", collection);
                command.put("key", unwrapArg(args.get(0)));
                command.put("query", argDocument(args, 1, new Document()));
            }
            case "insertone" -> {
                command.put("insert", collection);
                command.put("documents", List.of(argDocument(args, 0, new Document())));
            }
            case "insertmany" -> {
                command.put("insert", collection);
                command.put("documents", argArray(args, 0));
            }
            case "updateone", "updatemany", "replaceone" -> {
                command.put("update", collection);
                Document update = new Document("q", argDocument(args, 0, new Document()));
                update.put("u", argDocument(args, 1, new Document()));
                if ("updatemany".equals(method)) {
                    update.put("multi", true);
                }
                Document options = argDocumentOrNull(args, 2);
                if (options != null) {
                    if (Boolean.TRUE.equals(options.getBoolean("upsert"))) {
                        update.put("upsert", true);
                    }
                    if (options.containsKey("arrayFilters")) {
                        update.put("arrayFilters", options.get("arrayFilters"));
                    }
                }
                command.put("updates", List.of(update));
            }
            case "deleteone", "deletemany" -> {
                command.put("delete", collection);
                Document delete = new Document("q", argDocument(args, 0, new Document()));
                delete.put("limit", "deleteone".equals(method) ? 1 : 0);
                command.put("deletes", List.of(delete));
            }
            case "findoneandupdate", "findoneandreplace", "findoneanddelete" -> {
                Document modify = new Document("query", argDocument(args, 0, new Document()));
                if ("findoneanddelete".equals(method)) {
                    modify.put("remove", true);
                }
                else {
                    modify.put("update", argDocument(args, 1, new Document()));
                }
                Document options = argDocumentOrNull(args, 2);
                if (options != null) {
                    Object projection = options.get("projection");
                    if (projection != null) {
                        modify.put("fields", projection);
                    }
                    Object returnNew = options.get("returnDocument");
                    if (returnNew != null) {
                        modify.put("new", "after".equalsIgnoreCase(String.valueOf(returnNew)));
                    }
                    if (Boolean.TRUE.equals(options.getBoolean("upsert"))) {
                        modify.put("upsert", true);
                    }
                }
                command.put("findAndModify", collection);
                command.putAll(modify);
            }
            case "drop" -> command.put("drop", collection);
            case "createindex" -> {
                Document keys = argDocument(args, 0, new Document());
                Document index = new Document("key", keys);
                String name = keys.keySet().stream()
                        .map(key -> key + "_" + keys.get(key))
                        .reduce((a, b) -> a + "_" + b)
                        .orElse("idx");
                index.put("name", name);
                Document options = argDocumentOrNull(args, 1);
                if (options != null) {
                    options.forEach(index::putIfAbsent);
                    index.put("key", keys);
                }
                command.put("createIndexes", collection);
                command.put("indexes", List.of(index));
            }
            default -> throw unsupported("集合方法", method, args);
        }

        // 链式修饰：sort / limit / skip / projection / hint / batchSize
        for (int i = methodStart + 1; i < chain.size(); i++) {
            String token = chain.get(i);
            String name = methodName(token).toLowerCase(Locale.ROOT);
            List<String> chainArgs = innerArgs(token);
            switch (name) {
                case "sort" -> command.put("sort", argDocument(chainArgs, 0, new Document()));
                case "limit" -> command.put("limit", (int) numberValue(chainArgs, 0));
                case "skip" -> command.put("skip", (int) numberValue(chainArgs, 0));
                case "projection", "project" -> command.put("projection", argDocument(chainArgs, 0, new Document()));
                case "hint" -> command.put("hint", argDocument(chainArgs, 0, new Document()));
                case "batchsize" -> command.put("batchSize", (int) numberValue(chainArgs, 0));
                default -> throw error("暂不支持的链式调用：" + methodName(token));
            }
        }
        return command;
    }

    // ---------------------------------------------------------- 文本工具

    private static String methodName(String token) {
        int paren = token.indexOf('(');
        return (paren < 0 ? token : token.substring(0, paren)).trim();
    }

    /** 取括号内的参数，并按顶层逗号切分。 */
    private static List<String> innerArgs(String token) {
        int open = token.indexOf('(');
        if (open < 0) {
            return new ArrayList<>();
        }
        int close = token.lastIndexOf(')');
        String inside = close > open ? token.substring(open + 1, close) : token.substring(open + 1);
        List<String> args = new ArrayList<>();
        for (String part : splitTopLevel(inside, ',')) {
            args.add(part.trim());
        }
        return args;
    }

    private static String unwrapArg(String arg) {
        String value = arg.trim();
        if (value.length() >= 2
                && (value.startsWith("\"") && value.endsWith("\"") || value.startsWith("'") && value.endsWith("'"))) {
            return value.substring(1, value.length() - 1);
        }
        return value;
    }

    private static Document argDocument(List<String> args, int index, Document fallback) {
        if (index >= args.size() || args.get(index).isBlank()) {
            return fallback;
        }
        return parseDocument(args.get(index), "参数 " + (index + 1));
    }

    private static Document argDocumentOrNull(List<String> args, int index) {
        if (index >= args.size() || args.get(index).isBlank()) {
            return null;
        }
        return parseDocument(args.get(index), "参数 " + (index + 1));
    }

    private static List<Document> argArray(List<String> args, int index) {
        if (index >= args.size() || args.get(index).isBlank()) {
            return new ArrayList<>();
        }
        Document wrapper = parseDocument("{\"v\": " + args.get(index) + "}", "数组参数");
        Object value = wrapper.get("v");
        List<Document> documents = new ArrayList<>();
        if (value instanceof List<?> list) {
            for (Object item : list) {
                if (item instanceof Document document) {
                    documents.add(document);
                }
                else {
                    throw error("数组参数里只能放文档（如 [ { $match: {} } ]）");
                }
            }
        }
        return documents;
    }

    private static double numberValue(List<String> args, int index) {
        if (index >= args.size()) {
            throw error("缺少数字参数");
        }
        try {
            return Double.parseDouble(args.get(index).trim());
        }
        catch (NumberFormatException e) {
            throw error("需要一个数字，实际是：" + args.get(index));
        }
    }

    /** 宽松 JSON 解析：允许不带引号的键（与 mongosh 写法一致）。 */
    static Document parseDocument(String text, String what) {
        try {
            return Document.parse(text);
        }
        catch (RuntimeException e) {
            throw new Main.AgentException(
                    "DBMIND-QUERY-0001",
                    what + "不是合法的 JSON/文档：" + brief(text),
                    e.getMessage());
        }
    }

    /** 按顶层分隔符切分：括号/方括号/花括号内与字符串内的分隔符不算。 */
    static List<String> splitTopLevel(String text, char separator) {
        List<String> parts = new ArrayList<>();
        int depth = 0;
        char quote = 0;
        StringBuilder current = new StringBuilder();
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (quote != 0) {
                current.append(c);
                if (c == quote) {
                    quote = 0;
                }
                continue;
            }
            if (c == '"' || c == '\'') {
                quote = c;
                current.append(c);
                continue;
            }
            switch (c) {
                case '{', '[', '(' -> depth++;
                case '}', ']', ')' -> depth--;
                case '\\' -> {
                    // 转义字符：原样带上下一个字符
                    current.append(c);
                    if (i + 1 < text.length()) {
                        current.append(text.charAt(++i));
                    }
                    continue;
                }
                default -> {
                }
            }
            if (c == separator && depth == 0) {
                parts.add(current.toString());
                current.setLength(0);
                continue;
            }
            current.append(c);
        }
        parts.add(current.toString());
        return parts;
    }

    /** 去掉 `//` 行注释与 `/* *&#47;` 块注释（字符串内不处理，保持简单与保守）。 */
    static String stripComments(String text) {
        StringBuilder out = new StringBuilder();
        char quote = 0;
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (quote != 0) {
                out.append(c);
                if (c == quote) {
                    quote = 0;
                }
                continue;
            }
            if (c == '"' || c == '\'') {
                quote = c;
                out.append(c);
                continue;
            }
            if (c == '/' && i + 1 < text.length() && text.charAt(i + 1) == '/') {
                while (i < text.length() && text.charAt(i) != '\n') {
                    i++;
                }
                out.append('\n');
                continue;
            }
            if (c == '/' && i + 1 < text.length() && text.charAt(i + 1) == '*') {
                i += 2;
                while (i + 1 < text.length() && !(text.charAt(i) == '*' && text.charAt(i + 1) == '/')) {
                    i++;
                }
                i++;
                continue;
            }
            out.append(c);
        }
        return out.toString();
    }

    private static String brief(String text) {
        String one = text.replaceAll("\\s+", " ").trim();
        return one.length() > 80 ? one.substring(0, 80) + "…" : one;
    }

    private static Main.AgentException error(String message) {
        return new Main.AgentException("DBMIND-QUERY-0001", message, supportedHint());
    }

    private static Main.AgentException unsupported(String kind, String name, List<String> args) {
        return new Main.AgentException(
                "DBMIND-QUERY-0001",
                "暂不支持" + kind + "：" + name,
                supportedHint() + "\n收到的参数：" + args);
    }

    /** 报错时把「能写什么」讲清楚，比只说「不支持」有用得多。 */
    private static String supportedHint() {
        return String.join("\n",
                "支持的写法：",
                "  原生命令      { \"find\": \"users\", \"filter\": { \"age\": 30 } }",
                "  查询          db.users.find({ age: 30 }).sort({ age: 1 }).limit(10)",
                "  聚合          db.users.aggregate([ { $match: {} }, { $group: { _id: \"$x\", n: { $sum: 1 } } } ])",
                "  统计/去重      db.users.countDocuments({})  /  db.users.distinct(\"city\")",
                "  写入          db.users.insertOne({...}) / insertMany([...]) / updateOne/updateMany(filter, update)",
                "  删除          db.users.deleteOne({}) / deleteMany({})",
                "  结构          db.users.drop() / db.users.createIndex({ a: 1 })",
                "  库级          db.runCommand({...}) / db.adminCommand({...}) / use <db>");
    }
}
