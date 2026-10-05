package com.dbmind.deploy;

import de.bwaldvogel.mongo.MongoServer;
import de.bwaldvogel.mongo.backend.memory.MemoryBackend;

/**
 * 本地 Mongo 服务（内存后端）：监听 127.0.0.1:&lt;port&gt;，供端到端验证使用。
 *
 * <p>为什么需要它：验证「非 JDBC 协议宿主」必须有真库可连，但要求每个开发者
 * 先装一个 MongoDB 不可持续。mongo-java-server 实现了 Mongo 线协议，
 * 真驱动（mongodb-driver-sync）能正常连上 —— 链路是真的，数据是临时的。
 *
 * <p>用法：{@code java -jar dbmind-mongo-test-server.jar [port]}
 */
public final class MongoTestServer {

    private MongoTestServer() {
    }

    public static void main(String[] args) throws Exception {
        int port = args.length > 0 ? Integer.parseInt(args[0]) : 27017;
        MongoServer server = new MongoServer(new MemoryBackend());
        server.bind(new java.net.InetSocketAddress("127.0.0.1", port));
        Runtime.getRuntime().addShutdownHook(new Thread(server::shutdown));
        System.out.println("mongo-java-server listening on 127.0.0.1:" + port);
        System.out.flush();
        // 主线程挂住：由外部进程终止
        Thread.currentThread().join();
    }
}
