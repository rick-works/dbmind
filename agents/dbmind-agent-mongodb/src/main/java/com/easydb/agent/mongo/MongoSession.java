package com.dbmind.agent.mongo;

import com.mongodb.client.MongoClient;

/** 一个 Mongo 会话 = 一个 MongoClient + 默认库。 */
public final class MongoSession {

    private final String id;
    private final MongoClient client;
    private final String defaultDatabase;
    private final String serverVersion;

    public MongoSession(String id, MongoClient client, String defaultDatabase, String serverVersion) {
        this.id = id;
        this.client = client;
        this.defaultDatabase = defaultDatabase;
        this.serverVersion = serverVersion;
    }

    public String id() {
        return id;
    }

    public MongoClient client() {
        return client;
    }

    public String defaultDatabase() {
        return defaultDatabase;
    }

    public String serverVersion() {
        return serverVersion;
    }

    public void close() {
        try {
            client.close();
        }
        catch (RuntimeException ignored) {
            // 关闭失败无处上报：进程可能马上要退出
        }
    }
}
