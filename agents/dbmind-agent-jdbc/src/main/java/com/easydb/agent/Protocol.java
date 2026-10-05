package com.dbmind.agent;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.stream.JsonWriter;
import java.io.IOException;
import java.io.StringWriter;
import java.util.Map;

/**
 * 线上协议：一行一个 JSON 消息。
 *
 * <p>请求：{@code {"id":"...","method":"...", <params...>}}
 * <br>成功：{@code {"id":"...","ok":true,"result":{...}}}
 * <br>失败：{@code {"id":"...","ok":false,"code":"DBMIND-...","message":"...","detail":"..."}}
 *
 * <p>错误码沿用内核的码表（见 docs/error-codes.md）：这样前端与用户看到的是同一套
 * 语义，而不是「agent 内部错误」这种什么都说明不了的东西。
 */
public final class Protocol {

    public static final Gson GSON = new GsonBuilder().serializeNulls().create();

    private Protocol() {
    }

    /**
     * 手写信封：{@code result} 的字段照常序列化，{@code rows} 段用 {@code jsonValue}
     * 把调用方已经渲染好的文本**原样透传**（不再经过 Gson 的树）。
     */
    public static String ok(String id, JsonElement result, String rowsText) {
        try {
            StringWriter buffer = new StringWriter(1 << 16);
            JsonWriter writer = new JsonWriter(buffer);
            writer.beginObject();
            writer.name("id").value(id);
            writer.name("ok").value(true);
            writer.name("result").beginObject();
            for (Map.Entry<String, JsonElement> entry : ((JsonObject) result).entrySet()) {
                if ("rows".equals(entry.getKey())) {
                    continue;
                }
                writer.name(entry.getKey());
                GSON.toJson(entry.getValue(), writer);
            }
            if (rowsText != null) {
                writer.name("rows");
                writer.jsonValue(rowsText);
            }
            writer.endObject();
            writer.endObject();
            writer.flush();
            return buffer.toString();
        } catch (IOException | RuntimeException e) {
            // 手写信封失败就退回普通路径：绝不能因为优化把响应丢了
            return ok(id, result);
        }
    }

    public static String ok(String id, JsonElement result) {
        JsonObject envelope = new JsonObject();
        envelope.addProperty("id", id);
        envelope.addProperty("ok", true);
        envelope.add("result", result == null ? new JsonObject() : result);
        return GSON.toJson(envelope);
    }

    public static String failure(String id, String code, String message, String detail) {
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

    /** agent 侧的业务异常：带上内核错误码，避免壳层去猜。 */
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
