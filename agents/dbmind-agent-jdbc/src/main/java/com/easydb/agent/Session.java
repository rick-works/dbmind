package com.dbmind.agent;

import java.sql.Connection;
import java.sql.SQLException;
import java.util.concurrent.locks.ReentrantLock;

/** 一个 agent 侧会话 = 一条物理连接。会话 id 由内核指定（内核据此做进程代际判断）。 */
public final class Session {

    private final String id;
    private final String agentKey;
    private final String url;
    private final Connection connection;
    private volatile long lastUsedAt;

    /**
     * 串行锁：**一条物理连接同一时刻只允许一个请求在用**。
     *
     * <p>JDBC 的 Connection 不是线程安全的。agent 的每个请求都跑在线程池里（为的是 cancel 能插队），
     * 于是「界面同时要字段 + 索引」这种平常操作就会让两个线程共用一条连接 —— Oracle 实测会直接搅坏
     * 驱动内部状态（ORA-17027 流已被关闭 / ORA-17401 违反协议），而且**这条连接此后永久不可用**。
     * 加锁口径见 Main.dispatch：除 cancel 外，凡碰连接的请求都排队。
     */
    private final ReentrantLock lock = new ReentrantLock();

    public void lock() {
        lock.lock();
    }

    public void unlock() {
        lock.unlock();
    }

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
