package com.dbmind.agent.mongo;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import org.bson.Document;
import org.bson.types.Binary;
import org.bson.types.Decimal128;
import org.bson.types.ObjectId;

import java.math.BigDecimal;
import java.time.format.DateTimeFormatter;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Date;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * 命令结果 -> 协议结果（columns / rows / rowCount / truncated / affectedRows）。
 *
 * <p>值的形状与 JDBC 宿主保持一致（内核的 `CellValue`：tag + content）：
 * null / integer / real / text / blob。**文档与数组转成文本**（宽松 JSON）——
 * 结果表格里塞嵌套结构只会让界面更难用，需要细节的用户自己展开看文本。
 */
public final class ResultWriter {

    private static final int BLOB_PREVIEW_LIMIT = 256;

    /** 这些命令的 n / nModified 表示「受影响行数」，其余命令的响应是数据本身。 */
    private static final java.util.Set<String> WRITE_COMMANDS = java.util.Set.of(
            "insert", "update", "delete", "findandmodify", "createindexes", "drop", "dropdatabase",
            "create", "createcollection", "renamecollection", "collmod", "converttoCapped",
            "createuser", "dropuser", "updaterole", "grantrolestouser", "revokerolesfromuser");

    private ResultWriter() {
    }

    public static JsonObject render(
            Document raw,
            CommandParser.Parsed parsed,
            int maxRows,
            AtomicBoolean cancelled,
            JsonArray notices) {
        JsonObject result = new JsonObject();

        // 只有写类命令的 n 才表示「受影响行数」；count/dbStats 的 n 是**结果本身**
        String commandName = commandName(parsed);
        boolean writeCommand = WRITE_COMMANDS.contains(commandName);
        // 结果是不是「集合里的原始文档」——决定要不要给出行标识（见 identityOf）
        boolean documentRows = isDocumentRead(commandName);

        // 游标类命令：firstBatch 里就是数据；cursor.id != 0 说明服务端还有后续批次
        Document cursor = raw.get("cursor", Document.class);
        if (cursor != null) {
            @SuppressWarnings("unchecked")
            List<Document> batch = (List<Document>) cursor.get("firstBatch", List.class);
            if (batch == null) {
                batch = new ArrayList<>();
            }
            boolean more = cursor.get("id") != null && !(cursor.get("id") instanceof Number number && number.longValue() == 0);
            if (cancelled.get()) {
                throw new Main.AgentException("DBMIND-QUERY-0004", "查询已取消", null);
            }
            return fill(result, batch, maxRows, more, null, notices, documentRows);
        }

        Integer affected = null;
        if (writeCommand) {
            if (raw.get("n") instanceof Number number) {
                affected = number.intValue();
            }
            else if (raw.get("nModified") instanceof Number number) {
                affected = number.intValue();
            }
        }

        @SuppressWarnings("unchecked")
        List<Document> batch = (List<Document>) raw.get("values", List.class);
        if (batch == null) {
            batch = new ArrayList<>();
        }
        // findAndModify 把结果文档放在 value 里
        if (batch.isEmpty() && raw.get("value") instanceof Document value) {
            batch.add(value);
        }
        // 结果型命令（count / dbStats / buildInfo / ping …）：响应本身就是一行数据，
        // 直接丢掉会让 count 显示成「0 行」——用户会以为统计坏了
        if (batch.isEmpty() && !writeCommand && !raw.isEmpty()) {
            batch.add(raw);
        }
        return fill(result, batch, maxRows, false, affected, notices, documentRows);
    }

    /**
     * 结果就是**集合里的原始文档**的读命令。
     *
     * <p>`aggregate` 刻意不算：哪怕只写了 $match，$project / $addFields 也会让
     * 结果里的字段不再等于存储的字段，照着它写回就是改错数据。
     */
    private static boolean isDocumentRead(String commandName) {
        return "find".equals(commandName) || "findone".equals(commandName);
    }

    /**
     * 行标识：**只有能证明时才给**。
     *
     * <p>为什么必须由宿主说而不是内核猜：ObjectId 到了协议结果里只是一串十六进制，
     * 与「恰好长得一样的字符串」完全无法区分；写回时按错类型比较会**匹配不到任何行**——
     * 表现出来是「保存成功、0 行受影响」，比直接报错更糟。
     *
     * <p>证明条件：每一行都有非空的 `_id`，且它们的类型一致、且是我们支持当标识的类型
     * （ObjectId / 字符串 / 整数）。任何一条不满足就整个结果都不给 ——
     * 不能只对「一部分行」负责。
     */
    private static JsonObject identityOf(List<Document> visible) {
        if (visible.isEmpty()) {
            return null;
        }
        String type = null;
        for (Document document : visible) {
            String kind = identityType(document.get("_id"));
            if (kind == null) {
                return null;
            }
            if (type == null) {
                type = kind;
            }
            else if (!type.equals(kind)) {
                return null;
            }
        }
        JsonObject identity = new JsonObject();
        identity.addProperty("column", "_id");
        identity.addProperty("valueType", type);
        return identity;
    }

    /** 支持当行标识的类型；其它（Date / 二进制 / 嵌套 / 小数…）返回 null（不给编辑，而不是猜）。 */
    private static String identityType(Object value) {
        if (value instanceof ObjectId) {
            return "objectId";
        }
        if (value instanceof String) {
            return "string";
        }
        if (value instanceof Byte || value instanceof Short
                || value instanceof Integer || value instanceof Long) {
            return "integer";
        }
        return null;
    }

    private static JsonObject fill(
            JsonObject result,
            List<Document> batch,
            int maxRows,
            boolean more,
            Integer affected,
            JsonArray notices,
            boolean documentRows) {
        int limit = maxRows > 0 ? maxRows : Integer.MAX_VALUE;
        boolean truncated = more || batch.size() > limit;
        List<Document> visible = batch.size() > limit ? batch.subList(0, limit) : batch;

        // 列 = 各行键的并集（保持首次出现顺序）：文档型数据每行字段都可能不同
        Map<String, Boolean> names = new LinkedHashMap<>();
        for (Document document : visible) {
            for (String key : document.keySet()) {
                names.putIfAbsent(key, Boolean.TRUE);
            }
        }

        JsonArray columns = new JsonArray();
        for (String name : names.keySet()) {
            JsonObject column = new JsonObject();
            column.addProperty("name", name);
            // 文档型数据每行字段不同、类型也可能不同 ⇒ 这里如实留空，不编一个类型
            column.add("typeName", com.google.gson.JsonNull.INSTANCE);
            columns.add(column);
        }

        JsonArray rows = new JsonArray();
        for (Document document : visible) {
            JsonArray row = new JsonArray();
            for (String name : names.keySet()) {
                row.add(cell(document.get(name)));
            }
            rows.add(row);
        }

        if (truncated && notices != null) {
            notices.add("结果已被截断（服务端还有后续数据），建议加 limit 或缩小范围");
        }

        result.add("columns", columns);
        result.add("rows", rows);
        result.addProperty("rowCount", rows.size());
        result.addProperty("truncated", truncated);
        if (affected == null) {
            result.add("affectedRows", null);
        }
        else {
            result.addProperty("affectedRows", affected);
        }
        result.add("notices", notices == null ? new JsonArray() : notices);
        result.add("identity", documentRows ? identityOf(visible) : null);
        return result;
    }

    static JsonObject cell(Object value) {
        if (value == null) {
            return tagged("null");
        }
        if (value instanceof Boolean flag) {
            // 与 JDBC 宿主一致：布尔统一成 0/1，避免引入第四种值类型
            return integer(flag ? 1 : 0);
        }
        if (value instanceof Byte || value instanceof Short || value instanceof Integer || value instanceof Long) {
            return integer(((Number) value).longValue());
        }
        if (value instanceof Double || value instanceof Float) {
            return real(((Number) value).doubleValue());
        }
        if (value instanceof BigDecimal decimal) {
            return decimal.scale() <= 0 ? integer(decimal.longValue()) : real(decimal.doubleValue());
        }
        if (value instanceof Decimal128 decimal) {
            BigDecimal asDecimal = decimal.bigDecimalValue();
            return asDecimal.scale() <= 0 ? integer(asDecimal.longValue()) : real(asDecimal.doubleValue());
        }
        if (value instanceof String text) {
            return text(text);
        }
        if (value instanceof ObjectId id) {
            return text(id.toHexString());
        }
        if (value instanceof Date date) {
            return text(DateTimeFormatter.ISO_INSTANT.format(date.toInstant()));
        }
        if (value instanceof Binary binary) {
            byte[] data = binary.getData();
            return blob(data.length, data);
        }
        if (value instanceof Document || value instanceof Map || value instanceof List
                || value instanceof Object[]) {
            return text(relaxed(value));
        }
        return text(String.valueOf(value));
    }

    /** 嵌套结构用宽松 JSON 呈现（界面上按文本显示，需要时再展开）。 */
    private static String relaxed(Object value) {
        try {
            if (value instanceof Document document) {
                return document.toJson();
            }
            if (value instanceof List<?> list) {
                Document wrapper = new Document("v", list);
                String json = wrapper.toJson();
                // 去掉 {"v": ...} 外壳
                int start = json.indexOf('[');
                int end = json.lastIndexOf(']');
                return start >= 0 && end > start ? json.substring(start, end + 1) : json;
            }
            return String.valueOf(value);
        }
        catch (RuntimeException e) {
            return String.valueOf(value);
        }
    }

    /** 命令名 = 命令文档的第一个键（Mongo 命令的约定）。 */
    private static String commandName(CommandParser.Parsed parsed) {
        if (parsed == null || parsed.command() == null || parsed.command().isEmpty()) {
            return "";
        }
        String name = parsed.command().keySet().iterator().next();
        return name == null ? "" : name.toLowerCase();
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
            int take = Math.min(preview.length, BLOB_PREVIEW_LIMIT);
            byte[] slice = new byte[take];
            System.arraycopy(preview, 0, slice, 0, take);
            payload.addProperty("previewBase64", Base64.getEncoder().encodeToString(slice));
        }
        JsonObject cell = tagged("blob");
        cell.add("v", payload);
        return cell;
    }
}
