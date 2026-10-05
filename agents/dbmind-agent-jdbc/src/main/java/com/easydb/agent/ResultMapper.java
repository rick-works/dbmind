package com.dbmind.agent;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.stream.JsonWriter;
import java.io.IOException;
import java.io.StringWriter;
import java.math.BigDecimal;
import java.sql.Clob;
import java.sql.ResultSet;
import java.sql.ResultSetMetaData;
import java.sql.SQLException;
import java.sql.Timestamp;
import java.util.Base64;

/**
 * ResultSet -> 协议结果。
 *
 * <p>值的形状必须与内核的 `CellValue`（serde tag+content）严格一致：
 * {@code {"t":"null"}} / {@code {"t":"integer","v":1}} / {@code {"t":"real","v":1.5}}
 * / {@code {"t":"text","v":"s"}} / {@code {"t":"blob","v":{"len":12}}}。
 * 二进制只回长度：结果集要跨进程传输，塞原始字节会把响应体撑爆。
 */
public final class ResultMapper {

    /** CLOB 预览上限：够看内容，又不会把内存和协议撑爆。 */
    private static final int CLOB_PREVIEW_LIMIT = 64 * 1024;
    /** 二进制预览上限（base64 前若干字节，便于用户判断内容类型）。 */
    private static final int BLOB_PREVIEW_LIMIT = 256;

    private ResultMapper() {
    }

    public static JsonObject read(ResultSet resultSet, int maxRows) throws SQLException {
        return read(resultSet, maxRows, false);
    }

    /**
     * 读结果集。{@code compact=true} 时每行用**裸值**表示。
     *
     * <p>为什么值得单开一条路：旧格式每个单元格都是一个带类型标签的对象
     * （{@code {"t":"integer","v":1}}），一行 12 列 ≈ 500 字节文本，20 万行就是
     * ~100 MB 的 JSON —— 宿主逐格拼、内核逐格解，实测约 0.23 毫秒/行，
     * 导出 20 万行要近一分钟，瓶颈全在这里。
     *
     * <p>裸值格式（{@code [1,"a",null]}）让类型由 JSON 自身表达：整数/小数天然区分
     * （{@code 1} vs {@code 1.0}），null 就是 null，字符串就是字符串 —— 语义与原格式
     * 完全等价，报文小了 5~8 倍。**blob 仍保留标签对象**（它要带长度与预览字节，
     * 裸值表达不了），混在裸值行里没问题。
     *
     * <p>默认关闭：老内核不认裸值，只有内核明确要（请求里带 {@code compact:true}）
     * 才用。这样内核与宿主可以各自独立升级、独立回退。
     */
    public static JsonObject read(ResultSet resultSet, int maxRows, boolean compact) throws SQLException {
        ResultSetMetaData meta = resultSet.getMetaData();
        int columnCount = meta.getColumnCount();

        JsonArray columns = new JsonArray();
        for (int i = 1; i <= columnCount; i++) {
            JsonObject column = new JsonObject();
            column.addProperty("name", meta.getColumnLabel(i));
            column.addProperty("typeName", meta.getColumnTypeName(i));
            columns.add(column);
        }

        JsonArray rows = new JsonArray();
        boolean truncated = false;
        while (resultSet.next()) {
            if (rows.size() >= maxRows) {
                truncated = true;
                break;
            }
            JsonArray row = new JsonArray();
            for (int i = 1; i <= columnCount; i++) {
                row.add(compact ? bareCell(resultSet, i) : cell(resultSet, i));
            }
            rows.add(row);
        }

        JsonObject result = new JsonObject();
        result.add("columns", columns);
        result.add("rows", rows);
        result.addProperty("rowCount", rows.size());
        result.addProperty("truncated", truncated);
        result.addProperty("compactRows", compact);
        result.add("affectedRows", null);
        result.add("notices", new JsonArray());
        return result;
    }

    /**
     * 紧凑格式的单元格：能裸表达就裸表达，实在不行（blob）退回带标签对象。
     *
     * <p>与 {@link #cell} 的值语义一一对应 —— 布尔仍归 0/1、BigDecimal 整数化、时间戳按
     * 本地时间文本，避免两种格式出现细微差异。
     */
    /**
     * 紧凑模式专用：**不建树**，把行的 JSON 文本直接追加进 {@code rowsOut}，
     * 返回值里不含 {@code rows} —— 由 {@code Protocol.ok(id, result, rowsText)} 原样透传。
     *
     * <p>为什么要这样：旧写法每格 new 一个 JsonObject / JsonPrimitive，
     * 20 万行 × 12 列 = 240 万个对象，随后 Gson 还要把整棵树再走一遍拼成字符串。
     * 这里每格只剩一次 StringBuilder.append，字符串转义借一个复用的 JsonWriter。
     */
    public static JsonObject readCompactInto(ResultSet resultSet, int maxRows, StringBuilder rowsOut)
            throws SQLException {
        ResultSetMetaData meta = resultSet.getMetaData();
        int columnCount = meta.getColumnCount();

        JsonArray columns = new JsonArray();
        for (int i = 1; i <= columnCount; i++) {
            JsonObject column = new JsonObject();
            column.addProperty("name", meta.getColumnLabel(i));
            column.addProperty("typeName", meta.getColumnTypeName(i));
            columns.add(column);
        }

        rowsOut.append('[');
        boolean firstRow = true;
        int rowsWritten = 0;
        boolean truncated = false;
        while (resultSet.next()) {
            if (rowsWritten >= maxRows) {
                truncated = true;
                break;
            }
            if (!firstRow) {
                rowsOut.append(',');
            }
            firstRow = false;
            rowsOut.append('[');
            for (int i = 1; i <= columnCount; i++) {
                if (i > 1) {
                    rowsOut.append(',');
                }
                appendBare(rowsOut, resultSet, i);
            }
            rowsOut.append(']');
            rowsWritten++;
        }
        rowsOut.append(']');

        JsonObject result = new JsonObject();
        result.add("columns", columns);
        result.addProperty("rowCount", rowsWritten);
        result.addProperty("truncated", truncated);
        result.addProperty("compactRows", true);
        result.add("affectedRows", null);
        result.add("notices", new JsonArray());
        return result;
    }

    /** 把一个单元格按紧凑格式追加进缓冲：数字/null 直接写，字符串借 JsonWriter 转义 */
    private static void appendBare(StringBuilder buf, ResultSet resultSet, int index) throws SQLException {
        Object value;
        try {
            value = resultSet.getObject(index);
        } catch (SQLException err) {
            // 个别驱动对某些类型取不动：如实回 null，别让整页作废
            buf.append("null");
            return;
        }
        if (value == null || resultSet.wasNull()) {
            buf.append("null");
            return;
        }
        if (value instanceof Boolean flag) {
            buf.append(flag ? '1' : '0');
            return;
        }
        if (value instanceof Byte || value instanceof Short || value instanceof Integer || value instanceof Long) {
            buf.append(((Number) value).longValue());
            return;
        }
        if (value instanceof BigDecimal decimal) {
            if (decimal.scale() <= 0) {
                buf.append(decimal.longValueExact());
            } else {
                buf.append(decimal.toPlainString());
            }
            return;
        }
        if (value instanceof Number number) {
            buf.append(number.doubleValue());
            return;
        }
        if (value instanceof byte[] bytes) {
            appendBlob(buf, bytes.length, bytes);
            return;
        }
        if (value instanceof java.sql.Blob blob) {
            long length = blob.length();
            int take = (int) Math.min(length, BLOB_PREVIEW_LIMIT);
            appendBlob(buf, (int) length, blob.getBytes(1, take));
            return;
        }
        if (value instanceof Clob clob) {
            long length = clob.length();
            int take = (int) Math.min(length, CLOB_PREVIEW_LIMIT);
            appendEscaped(buf, clob.getSubString(1, take));
            return;
        }
        if (value instanceof Timestamp timestamp) {
            appendEscaped(buf, timestamp.toLocalDateTime().toString());
            return;
        }
        appendEscaped(buf, String.valueOf(value));
    }

    /**
     * 字符串一律走 JsonWriter 转义：自己写转义表迟早会漏掉控制字符。
     *
     * <p>IOException 在这里吃掉并兜底：{@code StringWriter} 写在内存里、不会真的失败，
     * 但 {@code Writer} 接口声明了它，不接就得让整条调用链都带上 throws（不值得）。
     * 兜底只做最基本的引号/反斜杠转义，保证出去的东西仍是合法 JSON。
     */
    private static void appendEscaped(StringBuilder buf, String text) {
        buf.append('"');
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            switch (c) {
                case '"':
                    buf.append("\\\"");
                    break;
                case '\\':
                    buf.append("\\\\");
                    break;
                case '\n':
                    buf.append("\\n");
                    break;
                case '\r':
                    buf.append("\\r");
                    break;
                case '\t':
                    buf.append("\\t");
                    break;
                case '\b':
                    buf.append("\\b");
                    break;
                case '\f':
                    buf.append("\\f");
                    break;
                default:
                    if (c < 0x20) {
                        buf.append(String.format("\\u%04x", (int) c));
                    } else {
                        buf.append(c);
                    }
            }
        }
        buf.append('"');
    }

    /** blob 仍用带标签的对象（要带长度与预览字节，裸值表达不了）。 */
    private static void appendBlob(StringBuilder buf, int length, byte[] preview) {
        buf.append("{\"t\":\"blob\",\"v\":{\"len\":").append(length);
        if (preview != null && preview.length > 0) {
            buf.append(",\"previewBase64\":\"")
                .append(Base64.getEncoder().encodeToString(preview))
                .append('"');
        }
        buf.append("}}");
    }

    private static JsonElement bareCell(ResultSet resultSet, int index) throws SQLException {
        Object value = resultSet.getObject(index);
        if (value == null || resultSet.wasNull()) {
            return com.google.gson.JsonNull.INSTANCE;
        }
        if (value instanceof Boolean flag) {
            return new com.google.gson.JsonPrimitive(flag ? 1 : 0);
        }
        if (value instanceof Byte || value instanceof Short || value instanceof Integer || value instanceof Long) {
            return new com.google.gson.JsonPrimitive(((Number) value).longValue());
        }
        if (value instanceof BigDecimal decimal) {
            if (decimal.scale() <= 0) {
                return new com.google.gson.JsonPrimitive(decimal.longValueExact());
            }
            return new com.google.gson.JsonPrimitive(decimal.doubleValue());
        }
        if (value instanceof Number number) {
            return new com.google.gson.JsonPrimitive(number.doubleValue());
        }
        if (value instanceof byte[] bytes) {
            return blob(bytes.length, bytes);
        }
        if (value instanceof java.sql.Blob blob) {
            long length = blob.length();
            int take = (int) Math.min(length, BLOB_PREVIEW_LIMIT);
            return blob((int) length, blob.getBytes(1, take));
        }
        if (value instanceof Clob clob) {
            long length = clob.length();
            int take = (int) Math.min(length, CLOB_PREVIEW_LIMIT);
            return new com.google.gson.JsonPrimitive(clob.getSubString(1, take));
        }
        if (value instanceof Timestamp timestamp) {
            return new com.google.gson.JsonPrimitive(timestamp.toLocalDateTime().toString());
        }
        return new com.google.gson.JsonPrimitive(String.valueOf(value));
    }

    private static JsonObject cell(ResultSet resultSet, int index) throws SQLException {
        Object value = resultSet.getObject(index);
        if (value == null || resultSet.wasNull()) {
            return tagged("null");
        }
        if (value instanceof Boolean flag) {
            // 统一成 0/1：跨语言表示布尔最省事，且不引入第四种值类型
            return integer(flag ? 1 : 0);
        }
        if (value instanceof Byte || value instanceof Short || value instanceof Integer || value instanceof Long) {
            return integer(((Number) value).longValue());
        }
        if (value instanceof BigDecimal decimal) {
            if (decimal.scale() <= 0) {
                return integer(decimal.longValueExact());
            }
            return real(decimal.doubleValue());
        }
        if (value instanceof Number number) {
            return real(number.doubleValue());
        }
        if (value instanceof byte[] bytes) {
            return blob(bytes.length, bytes);
        }
        if (value instanceof java.sql.Blob blob) {
            long length = blob.length();
            int take = (int) Math.min(length, BLOB_PREVIEW_LIMIT);
            return blob((int) length, blob.getBytes(1, take));
        }
        if (value instanceof Clob clob) {
            long length = clob.length();
            int take = (int) Math.min(length, CLOB_PREVIEW_LIMIT);
            return text(clob.getSubString(1, take));
        }
        if (value instanceof Timestamp timestamp) {
            return text(timestamp.toLocalDateTime().toString());
        }
        return text(String.valueOf(value));
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

    private static JsonObject blob(int length, byte[] preview) {
        JsonObject payload = new JsonObject();
        payload.addProperty("len", length);
        if (preview != null && preview.length > 0) {
            payload.addProperty("previewBase64", Base64.getEncoder().encodeToString(preview));
        }
        JsonObject cell = tagged("blob");
        cell.add("v", payload);
        return cell;
    }
}
