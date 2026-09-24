package com.dbmind.agent;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

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
