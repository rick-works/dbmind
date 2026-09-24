package com.dbmind.agent;

import java.sql.Connection;
import java.sql.SQLException;

/** 一个 agent 侧会话 = 一条物理连接。会话 id 由内核指定（内核据此做进程代际判断）。 */
public final class Session {

    private final String id;
    private final String agentKey;
    private final String url;
    private final Connection connection;
    private volatile long lastUsedAt;

    public Session(String id, String agentKey, String url, Connection connection) {
        this.id = id;
        this.agentKey = agentKey;
        this.url = url;
        this.connection = connection;
        this.lastUsedAt = System.currentTimeMillis();
    }

    public String id() {
        return id;
    }

    public String agentKey() {
        return agentKey;
    }

    public String url() {
        return url;
    }

    public Connection connection() {
        return connection;
    }

    public long lastUsedAt() {
        return lastUsedAt;
    }

    public void touch() {
        this.lastUsedAt = System.currentTimeMillis();
    }

    public void close() {
        try {
            if (!connection.isClosed()) {
                connection.close();
            }
        } catch (SQLException ignored) {
            // 关闭失败无处上报：进程可能马上就要退出了
        }
    }
}
