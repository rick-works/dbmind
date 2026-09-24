package com.dbmind.agent.es;

import java.util.List;
import java.util.Locale;

/**
 * ES 语句 = 一条 REST 请求：
 *
 * <pre>
 *   GET  /logs/_search
 *   POST /logs/_doc  { "level": "info" }
 * </pre>
 *
 * 首行是 `方法 路径`，其余是请求体。这里只做「拆开」，不做判定 ——
 * 只读判定在内核的安全闸门里（<code>es.rs</code>），两处规则绝不能各写一份。
 */
public final class Statement {

    private static final List<String> METHODS = List.of("GET", "POST", "PUT", "DELETE", "HEAD", "PATCH");

    public final String method;
    /** 路径（含查询串，例如 `/_cat/indices?format=json`） */
    public final String path;
    public final String body;

    private Statement(String method, String path, String body) {
        this.method = method;
        this.path = path;
        this.body = body;
    }

    public static boolean looksLikeRequest(String text) {
        String first = firstLine(text);
        for (String method : METHODS) {
            if (first.toUpperCase(Locale.ROOT).startsWith(method + " ")) {
                return true;
            }
        }
        return false;
    }

    public static Statement parse(String text) {
        String first = firstLine(text);
        int space = first.indexOf(' ');
        if (space < 0) {
            throw error("语句需要写成「方法 路径」，例如：GET /logs/_search");
        }
        String method = first.substring(0, space).trim().toUpperCase(Locale.ROOT);
        if (!METHODS.contains(method)) {
            throw error("不支持的方法：" + method + "（支持 " + String.join(" / ", METHODS) + "）");
        }
        String rest = first.substring(space + 1).trim();
        if (rest.isEmpty()) {
            throw error("缺少路径，例如：GET /logs/_search");
        }
        // 路径与请求体写在同一行是常见写法（`POST /logs/_search { ... }`），
        // 按空白切开：第一段是路径，其余是请求体
        String path = rest;
        String inlineBody = null;
        int whitespace = firstWhitespace(rest);
        if (whitespace > 0) {
            path = rest.substring(0, whitespace);
            inlineBody = rest.substring(whitespace).trim();
        }
        if (!path.startsWith("/")) {
            path = "/" + path;
        }
        String tail = bodyAfterFirstLine(text);
        String body = inlineBody == null ? tail
                : (tail == null ? inlineBody : inlineBody + "\n" + tail);
        return new Statement(method, path, body);
    }

    /** 第一个空白字符的位置；没有则返回 -1。 */
    private static int firstWhitespace(String text) {
        for (int i = 0; i < text.length(); i++) {
            if (Character.isWhitespace(text.charAt(i))) {
                return i;
            }
        }
        return -1;
    }

    private static String firstLine(String text) {
        for (String line : text.split("\n")) {
            if (!line.trim().isEmpty()) {
                return line.trim();
            }
        }
        return "";
    }

    private static String bodyAfterFirstLine(String text) {
        int newline = text.indexOf('\n');
        if (newline < 0) {
            return null;
        }
        String body = text.substring(newline + 1).trim();
        return body.isEmpty() ? null : body;
    }

    private static Main.AgentException error(String message) {
        return new Main.AgentException("DBMIND-QUERY-0001", message,
                "支持的写法：\n"
                        + "  GET  /logs/_search\n"
                        + "  POST /logs/_search   { \"query\": { \"match_all\": {} } }\n"
                        + "  POST /logs/_doc      { \"level\": \"info\" }\n"
                        + "  PUT  /logs\n"
                        + "  DELETE /logs/_doc/1\n"
                        + "  GET  /_cat/indices?format=json");
    }
}
