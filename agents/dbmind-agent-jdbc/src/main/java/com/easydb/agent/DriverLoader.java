package com.dbmind.agent;

import java.io.File;
import java.net.URL;
import java.net.URLClassLoader;
import java.sql.Driver;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * 驱动类加载。
 *
 * <p>要点：JDBC 驱动必须来自 URLClassLoader（用户目录里的 jar），且**不能**用
 * 应用类加载器当父加载器 —— 否则同一个驱动类可能被加载两次，出现
 * 「驱动不认识自己创建的连接」这类诡异问题。这里统一用 platform classloader 作父级。
 *
 * <p>每个 agentKey 复用一份加载器（驱动 jar 是进程内共享的），并记录已加载的 jar 集合；
 * jar 列表变化时重建加载器，便于「装完驱动不重启」。
 */
public final class DriverLoader {

    private static final Map<String, Loaded> CACHE = new ConcurrentHashMap<>();

    private DriverLoader() {
    }

    private record Loaded(List<String> jars, URLClassLoader loader) {
    }

    public static synchronized Driver instantiate(String agentKey, String className, List<String> jarPaths) {
        if (jarPaths == null || jarPaths.isEmpty()) {
            throw new Protocol.AgentException(
                    "DBMIND-DRV-0002",
                    "没有可用的驱动 jar",
                    "请把驱动 jar 放入 ~/.dbmind/drivers/" + agentKey + "/ 后重试");
        }

        Loaded loaded = CACHE.get(agentKey);
        if (loaded == null || !loaded.jars().equals(jarPaths)) {
            loaded = load(agentKey, jarPaths);
            CACHE.put(agentKey, loaded);
        }

        try {
            Class<?> clazz = Class.forName(className, true, loaded.loader());
            Object instance = clazz.getDeclaredConstructor().newInstance();
            if (!(instance instanceof Driver driver)) {
                throw new Protocol.AgentException(
                        "DBMIND-DRV-0002",
                        className + " 不是 java.sql.Driver 实现",
                        "检查 YAML 里该类型的 jdbc.driverClass 是否写对");
            }
            return driver;
        } catch (ClassNotFoundException e) {
            throw new Protocol.AgentException(
                    "DBMIND-DRV-0002",
                    "驱动类不存在：" + className,
                    "已加载 jar：" + String.join(", ", jarPaths));
        } catch (ReflectiveOperationException e) {
            throw new Protocol.AgentException(
                    "DBMIND-DRV-0002",
                    "驱动类无法实例化：" + className,
                    e.toString());
        }
    }

    private static Loaded load(String agentKey, List<String> jarPaths) {
        List<URL> urls = new ArrayList<>();
        for (String path : jarPaths) {
            File file = new File(path);
            if (!file.isFile()) {
                continue;
            }
            try {
                urls.add(file.toURI().toURL());
            } catch (Exception e) {
                throw new Protocol.AgentException("DBMIND-DRV-0002", "驱动 jar 路径非法：" + path, e.toString());
            }
        }
        if (urls.isEmpty()) {
            throw new Protocol.AgentException(
                    "DBMIND-DRV-0002",
                    "驱动 jar 不存在（agentKey=" + agentKey + "）",
                    String.join(", ", jarPaths));
        }
        URLClassLoader loader = new URLClassLoader(urls.toArray(new URL[0]), ClassLoader.getPlatformClassLoader());
        return new Loaded(List.copyOf(jarPaths), loader);
    }
}
