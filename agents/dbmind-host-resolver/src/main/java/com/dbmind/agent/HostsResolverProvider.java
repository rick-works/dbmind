package com.dbmind.agent;

import java.net.InetAddress;
import java.net.spi.InetAddressResolver;
import java.net.spi.InetAddressResolverProvider;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.attribute.FileTime;
import java.util.HashMap;
import java.util.Locale;
import java.util.Map;
import java.util.stream.Stream;

/**
 * 自有 hosts 解析器（JDK 18+ 的 {@code java.net.spi.InetAddressResolverProvider}）。
 *
 * <p>为什么需要它：MySQL Connector/J 在 {@code ConnectionImpl} 构造里会把服务端 IP
 * <b>反解成主机名</b>（{@code InetSocketAddress.getHostName()}）。企业 DNS 一般没有
 * PTR 记录，Java 对失败的解析又不缓存，于是每新建一条 JDBC 会话都白等一次 DNS 超时
 * （真机实测 4573ms；给一份「IP → 占位名」的映射后 16ms）。</p>
 *
 * <p>为什么不用 {@code -Djdk.net.hosts.file}：那条属性是<b>独占式</b>的 —— 指定文件后
 * JVM 只在这份文件里查名字，文件里没有的名字直接 {@code UnknownHostException}，
 * 所有「用域名连」的库（云 RDS / 代理转发 / K8s Service）全废（2026-11 实测）。
 * 这条 SPI 不一样：先查内核写好的映射，<b>未命中回退平台内置解析</b>（系统 hosts + DNS），
 * 域名连接完全不受影响 —— 所以可以默认启用。</p>
 *
 * <p>映射文件路径：系统属性 {@code dbmind.agent.hosts}（内核 spawn 时传入），
 * 未设置时回退 {@code ~/.dbmind/agent-hosts}。文件格式与系统 hosts 相同
 * （{@code IP 名字}），内核每次连接清单变化时重写；这里按文件 mtime 缓存，
 * 文件变了自动重读。</p>
 *
 * <p>线程安全：InetAddress 内部可能在多线程并发首次解析，表格重建用锁串行，
 * 读取用不可变快照。</p>
 */
public final class HostsResolverProvider extends InetAddressResolverProvider {

    /** 内核 spawn 宿主时传的映射文件路径（crates/dbmind-core/src/agent.rs）。 */
    static final String FILE_PROP = "dbmind.agent.hosts";

    @Override
    public InetAddressResolver get(Configuration configuration) {
        // 打一行到 stderr：宿主的 stderr 落 agent-jdbc.err.log，排障时能确认它真的加载了。
        System.err.println("[dbmind] agent hosts resolver enabled: " + Resolver.file());
        return new Resolver(configuration.builtinResolver());
    }

    @Override
    public String name() {
        return "dbmind-agent-hosts";
    }

    private static final class Resolver implements InetAddressResolver {

        private final InetAddressResolver builtin;
        private final Object lock = new Object();

        // 不可变快照：名字 -> 地址（正向），IP 文本 -> 名字（反向）。
        private volatile Map<String, InetAddress> byName = Map.of();
        private volatile Map<String, String> byAddress = Map.of();
        private volatile FileTime stamp = null;

        Resolver(InetAddressResolver builtin) {
            this.builtin = builtin;
        }

        @Override
        public Stream<InetAddress> lookupByName(String host, LookupPolicy policy)
                throws java.net.UnknownHostException {
            ensureLoaded();
            InetAddress hit = byName.get(host.toLowerCase(Locale.ROOT));
            if (hit != null) {
                return Stream.of(hit);
            }
            return builtin.lookupByName(host, policy);
        }

        @Override
        public String lookupByAddress(byte[] address) throws java.net.UnknownHostException {
            ensureLoaded();
            String name = byAddress.get(formatAddress(address));
            if (name != null) {
                return name;
            }
            return builtin.lookupByAddress(address);
        }

        private void ensureLoaded() {
            reloadIfChanged();
        }

        /** 按 mtime 缓存：文件没变不重读，变了（内核刷新了连接清单）自动重读。 */
        private void reloadIfChanged() {
            Path file = file();
            FileTime now;
            try {
                now = Files.getLastModifiedTime(file);
            } catch (Exception e) {
                now = null; // 文件不存在
            }
            if (now == null && stamp == null) {
                return;
            }
            if (now != null && now.equals(stamp)) {
                return;
            }
            synchronized (lock) {
                // 双检：拿锁后再看一眼，避免并发重读
                if (now == null && stamp == null) {
                    return;
                }
                if (now != null && now.equals(stamp)) {
                    return;
                }
                Map<String, InetAddress> names = new HashMap<>();
                Map<String, String> addresses = new HashMap<>();
                if (now != null) {
                    parse(file, names, addresses);
                }
                byName = Map.copyOf(names);
                byAddress = Map.copyOf(addresses);
                stamp = now;
            }
        }

        static Path file() {
            String prop = System.getProperty(FILE_PROP);
            if (prop != null && !prop.isBlank()) {
                return Paths.get(prop);
            }
            return Paths.get(System.getProperty("user.home"), ".dbmind", "agent-hosts");
        }

        /**
         * 解析 hosts 形状的文件。每行 {@code IP 名字...}：第一列必须是 IP 字面量，
         * 其余列注册为这个名字的正向映射（占位名 dbmind-host-N 或 localhost）。
         * 行内 {@code #} 之后当注释。解析不了的行直接跳过 —— 绝不能让一份坏文件
         * 把整个解析器带崩。
         */
        private static void parse(Path file, Map<String, InetAddress> names, Map<String, String> addresses) {
            try {
                for (String raw : Files.readAllLines(file, StandardCharsets.UTF_8)) {
                    String line = raw;
                    int hash = line.indexOf('#');
                    if (hash >= 0) {
                        line = line.substring(0, hash);
                    }
                    line = line.trim();
                    if (line.isEmpty()) {
                        continue;
                    }
                    String[] tokens = line.split("\\s+");
                    byte[] ip = parseLiteralIp(tokens[0]);
                    if (ip == null || tokens.length < 2) {
                        continue;
                    }
                    InetAddress address;
                    try {
                        // IP 字面量走 JDK 的短路路径（不触发 resolver，不会递归）
                        address = InetAddress.getByAddress(ip);
                    } catch (Exception e) {
                        continue;
                    }
                    for (int i = 1; i < tokens.length; i++) {
                        String name = tokens[i].toLowerCase(Locale.ROOT);
                        if (!names.containsKey(name)) {
                            names.put(name, address);
                        }
                        addresses.putIfAbsent(formatAddress(ip), name);
                    }
                }
            } catch (Exception e) {
                // 读不出就当空表：全部走 builtin，行为退回优化前
                names.clear();
                addresses.clear();
            }
        }

        /** 手写 IPv4 解析；IPv6 只认 hex/冒号/点（绝不能把域名交给 getByName —— 会走 resolver 递归）。 */
        private static byte[] parseLiteralIp(String text) {
            String[] parts = text.split("\\.");
            if (parts.length == 4) {
                byte[] out = new byte[4];
                for (int i = 0; i < 4; i++) {
                    int v;
                    try {
                        v = Integer.parseInt(parts[i]);
                    } catch (NumberFormatException e) {
                        return null;
                    }
                    if (v < 0 || v > 255) {
                        return null;
                    }
                    out[i] = (byte) v;
                }
                return out;
            }
            boolean looksIpv6 = !text.isEmpty() && text.indexOf(':') >= 0
                    && text.chars().allMatch(c -> (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')
                        || (c >= 'A' && c <= 'F') || c == ':' || c == '.');
            if (!looksIpv6) {
                return null;
            }
            try {
                // IPv6 字面量（如 ::1）：isLiteral 短路，不会查 DNS
                return InetAddress.getByName(text).getAddress();
            } catch (Exception e) {
                return null;
            }
        }

        /** byte[] -> 点分 IPv4 文本（与 hosts 文件里的写法一致）。 */
        private static String formatAddress(byte[] address) {
            if (address.length == 4) {
                StringBuilder sb = new StringBuilder(15);
                for (int i = 0; i < 4; i++) {
                    if (i > 0) {
                        sb.append('.');
                    }
                    sb.append(address[i] & 0xFF);
                }
                return sb.toString();
            }
            // IPv6：交给 JDK 的字面量格式化（getHostName 不参与，纯文本转换）
            try {
                return InetAddress.getByAddress(address).getHostAddress();
            } catch (Exception e) {
                return "";
            }
        }
    }
}
