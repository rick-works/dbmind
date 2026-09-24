//! agent 运行时宿主：与外置驱动进程通信（NDJSON，一行一个 JSON 消息）。
//!
//! 为什么要有这一层：JVM 的 JDBC 驱动覆盖面最广，但把它塞进内核会带来
//! 崩溃连带、依赖膨胀、版本冲突三个问题。于是把驱动放进独立进程，用一条
//! **窄协议**通信：进程死了只影响它自己，内核下一次调用重新拉起即可。
//!
//! 三个刻意的设计：
//! 1. **不重试查询**：连接/元数据这类幂等调用失败可以重建重试，查询**不行** ——
//!    响应丢失时语句可能已经在数据库侧执行，重试等于再跑一遍。
//! 2. **代际失效**：进程重启后 agent 侧会话全部消失，内核用 generation 判断
//!    缓存会话是否还有效，而不是去匹配错误文案。
//! 3. **可硬中断**：取消不是「等下一行结果时检查」，而是直接向 agent 发 cancel，
//!    由 JDBC 的 `Statement.cancel()` 打断正在执行的语句。

use crate::error::{DbMindError, ErrorCode, Result};
use crate::paths;
use crate::types::{ConnectionConfig, TableInfo, TableKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: u32 = 1;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// 宿主定义：**一类 agentKey 由一个可执行包承载**。
///
/// 为什么是静态声明而不是全靠握手：进程启动有成本，内核需要在不启动进程的前提下
/// 就能回答「这个类型该找谁、它备好了没」（UI/CLI 的就绪报告要用）。
/// 真正的权威判定仍来自进程握手（`AgentHost::supports`），这里只是快速索引。
#[derive(Debug, Clone, Copy)]
pub struct AgentHostSpec {
    /// 宿主标识（报告与日志用）
    pub id: &'static str,
    /// 产物名（可执行文件旁 `agents/` 下、以及开发态 target/ 下）
    pub jar_name: &'static str,
    /// 覆盖用的环境变量名
    pub env_override: &'static str,
    /// 开发态仓库路径（相对 dbmind-core crate 根）
    pub dev_dir: &'static str,
    /// 构建目录（缺包时给用户看的指引）
    pub build_dir: &'static str,
    /// 该宿主**专属**承载的 agentKey；留空表示「通用宿主」
    pub agent_keys: &'static [&'static str],
    /// 通用宿主：按内核给的 driverClass / urlTemplate 工作，可承载任意 JDBC 类型
    pub generic: bool,
    /// 运行该宿主所需的最低 Java 主版本（与 `agents/*/pom.xml` 的 maven.compiler.release 一致）。
    ///
    /// 为什么必须有这一项：就绪判断原来只看「java 存在 + jar 存在」，于是 JDK 11 机器上
    /// 会被判定为**就绪**，真去连接时 JVM 一起来就 `UnsupportedClassVersionError` 退出 ——
    /// 用户看到的是「agent 进程已退出」，而根因（版本不匹配）藏在进程的 stderr 里。
    /// 把版本要求写成声明，就能在**连接之前**把话说清楚。
    pub min_java: u32,
}

impl AgentHostSpec {
    /// 通用 JDBC 宿主：承载所有带 `jdbc` 元数据的类型（13 个）。
    pub const fn jdbc() -> Self {
        Self {
            id: "jdbc",
            jar_name: "dbmind-agent-jdbc.jar",
            env_override: "DBMIND_AGENT_JAR",
            dev_dir: "../../agents/dbmind-agent-jdbc/target",
            build_dir: "agents/dbmind-agent-jdbc",
            agent_keys: &[],
            generic: true,
            min_java: 17,
        }
    }

    /// MongoDB 专属宿主：Mongo 不是 JDBC 体系，需要自己的协议实现。
    pub const fn mongodb() -> Self {
        Self {
            id: "mongodb",
            jar_name: "dbmind-agent-mongodb.jar",
            env_override: "DBMIND_AGENT_MONGODB_JAR",
            dev_dir: "../../agents/dbmind-agent-mongodb/target",
            build_dir: "agents/dbmind-agent-mongodb",
            agent_keys: &["mongodb"],
            generic: false,
            min_java: 17,
        }
    }

    /// Redis 专属宿主：RESP 协议（命令 + 回复），既不是 SQL 也不是 JDBC。
    pub const fn redis() -> Self {
        Self {
            id: "redis",
            jar_name: "dbmind-agent-redis.jar",
            env_override: "DBMIND_AGENT_REDIS_JAR",
            dev_dir: "../../agents/dbmind-agent-redis/target",
            build_dir: "agents/dbmind-agent-redis",
            agent_keys: &["redis"],
            generic: false,
            min_java: 17,
        }
    }

    /// Elasticsearch 专属宿主：把 REST 请求直接转给集群（不强绑任何 ES 客户端版本）。
    pub const fn elasticsearch() -> Self {
        Self {
            id: "elasticsearch",
            jar_name: "dbmind-agent-elasticsearch.jar",
            env_override: "DBMIND_AGENT_ELASTICSEARCH_JAR",
            dev_dir: "../../agents/dbmind-agent-elasticsearch/target",
            build_dir: "agents/dbmind-agent-elasticsearch",
            agent_keys: &["elasticsearch"],
            generic: false,
            min_java: 17,
        }
    }

    pub const fn all() -> [AgentHostSpec; 4] {
        [
            AgentHostSpec::jdbc(),
            AgentHostSpec::mongodb(),
            AgentHostSpec::redis(),
            AgentHostSpec::elasticsearch(),
        ]
    }

    /// 缺包时给用户的可执行指引。
    ///
    /// 分两种口气：开发态（debug）说「去构建」；发布态只能要求「重装 / 指路」——
    /// 让终端用户去跑 `mvn package` 是没意义的建议。这条差异只在打包后才看得出来，
    /// 所以按构建类型分。
    pub fn build_hint(&self) -> String {
        if cfg!(debug_assertions) {
            format!("先构建 {}，或设置 {}", self.build_dir, self.env_override)
        } else {
            format!(
                "安装包似乎不完整（程序目录下缺少 agents/{}）：请重新安装完整包，\
                 或用 {} / {} 指定宿主包位置",
                self.jar_name, self.env_override, AGENTS_DIR_ENV
            )
        }
    }

    /// 该宿主是否声明承载某个 agentKey（不含运行时握手）。
    pub fn declares(&self, agent_key: &str) -> bool {
        self.agent_keys.contains(&agent_key)
    }
}

/// agent 宿主就绪状态（不启动进程即可查询，供 UI/CLI 显示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAvailability {
    /// 宿主标识（jdbc / mongodb …）
    pub id: String,
    pub ready: bool,
    pub java: Option<String>,
    pub jar: Option<String>,
    pub reason: Option<String>,
}

/// agent 宿主的 stderr 落盘位置（每次启动覆盖）。
///
/// 覆盖而不是追加是有意的：这里要回答的是「**这一次**为什么起不来」，
/// 历史上积累的报错只会把当前原因淹掉。
pub fn agent_stderr_path(spec: &AgentHostSpec) -> PathBuf {
    crate::paths::home_dir()
        .join("logs")
        .join(format!("agent-{}.err.log", spec.id))
}

/// 把宿主 stderr 接到文件；接不上（目录建不了等）就退回丢弃 —— 诊断能力不该拖垮启动。
pub fn agent_stderr_stdio(spec: &AgentHostSpec) -> Stdio {
    let path = agent_stderr_path(spec);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::File::create(&path) {
        Ok(file) => Stdio::from(file),
        Err(_) => Stdio::null(),
    }
}

/// 读宿主 stderr 的最后 `max_lines` 行（拿不到就给空串，调用方据此换一种说法）。
fn stderr_tail(spec: &AgentHostSpec, max_lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(agent_stderr_path(spec)) else {
        return String::new();
    };
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join(" / ")
}

/// Windows：把子进程的控制台窗口藏掉。
///
/// 桌面版是 **GUI 子系统**程序（自身没有控制台），此时未经 `CREATE_NO_WINDOW` 启动的
/// `java` 会**自己弹出一个控制台窗口** —— 点连接时突然冒出来的那个黑窗就是这么来的
/// （长驻的宿主一个；就绪检查里的 `java -version` 还会再闪几下）。
///
/// 以前用 `dbmind-web.exe`（控制台程序）启动时看不出来：子进程**继承**了它的控制台。
/// 换句话说这是"桌面化"才暴露的问题，不是新引入的。
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// Windows 的 `CREATE_NO_WINDOW`：不为子进程创建控制台。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

/// 解析 Java 运行时：`DBMIND_JAVA` > `JAVA_HOME/bin/java` > PATH 上的 `java`。
pub fn resolve_java() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("DBMIND_JAVA") {
        let path = PathBuf::from(custom.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(home) = std::env::var("JAVA_HOME") {
        let bin = if cfg!(windows) { "java.exe" } else { "java" };
        let candidate = Path::new(home.trim()).join("bin").join(bin);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // 交给 PATH 解析（只检查存在性无法判断，这里返回名字由 Command 去解析）
    Some(PathBuf::from("java"))
}

/// 打包产物里 agent 包的目录（壳层用它把自己的资源目录告诉内核）。
///
/// 为什么需要这一层：Tauri/Electron 在 macOS 上把资源放在 `Contents/Resources`，
/// 而可执行文件在 `Contents/MacOS` —— 只靠「exe 旁边」会找不到宿主。
/// 壳层拿到自己的资源目录后设进这个变量，内核就不必知道打包器的目录布局。
pub const AGENTS_DIR_ENV: &str = "DBMIND_AGENTS_DIR";

/// 让**单测**加载真实 agent 宿主（会起 JVM）的开关：`DBMIND_TEST_AGENTS=1`。
pub const AGENTS_TEST_ENV: &str = "DBMIND_TEST_AGENTS";

/// 测试构建里是否允许看到真实宿主包。
///
/// 默认 **不允许**。这条默认值是踩出来的：debug 构建有「开发态兜底」
/// （见 `resolve_agent_jar`），于是开发机上跑一次全量单测就会找到
/// `agents/*/target/*.jar` 并起一堆 JVM —— 一次留下 45 个 `hs_err_pid*.log`
/// 和一个 235MB 的孤儿宿主。更坏的是它让**结果取决于本机**：装了 jar 走一条路、
/// 没装走另一条，于是「通过」可能只是资源紧张下的假通过（两条测试我都踩到过）。
///
/// 单测应当快、可复现；**真正验证宿主交互**的是 CLI 冒烟 / E2E / MCP / HTTP ——
/// 那些是独立进程，不受这里影响。需要让单测也真的连一次时显式打开：
/// `DBMIND_TEST_AGENTS=1 cargo test -p dbmind-core`。
#[cfg(test)]
pub(crate) fn agent_hosts_visible() -> bool {
    std::env::var(AGENTS_TEST_ENV)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// 非测试构建恒可见（两种构建共用同一段判定逻辑，避免分支漂移）。
#[cfg(not(test))]
pub(crate) fn agent_hosts_visible() -> bool {
    true
}

/// 解析某宿主的可执行包，顺序即优先级：
///
/// 1. 该宿主专属环境变量（`DBMIND_AGENT_JAR` / `DBMIND_AGENT_MONGODB_JAR` …）
/// 2. `DBMIND_AGENTS_DIR`（打包器给的资源目录）
/// 3. 可执行文件旁的 `agents/`
/// 4. **仅 debug 构建**：开发态仓库路径
pub fn resolve_agent_jar(spec: &AgentHostSpec) -> Option<PathBuf> {
    // 测试构建默认看不到宿主包（见 `agent_hosts_visible`）：否则开发机上跑单测
    // 会顺手起一堆 JVM。显式设了开关就照常解析。
    if !agent_hosts_visible() {
        return None;
    }
    if let Ok(custom) = std::env::var(spec.env_override) {
        let path = PathBuf::from(custom.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(dir) = std::env::var(AGENTS_DIR_ENV) {
        let candidate = Path::new(dir.trim()).join(spec.jar_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("agents").join(spec.jar_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // 开发态兜底：**只在 debug 构建里启用**。
    //
    // 发布包里没有源码树，而这条路径是编译期写死的绝对路径 —— 若它在 release 里生效，
    // 发布的二进制可能「在开发机上恰好能用」，到用户机器上却找不到任何宿主；
    // 更糟的是它可能悄悄用开发树里那个版本不明的 jar。
    #[cfg(debug_assertions)]
    {
        let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(spec.dev_dir)
            .join(spec.jar_name);
        if dev.is_file() {
            return Some(dev);
        }
    }
    None
}

/// 为 Java 宿主准备一份**自有 hosts 文件**，返回它的路径（写不成功返回 `None`）。
///
/// 解决的问题（实测数据）：MySQL 驱动在 `ConnectionImpl` 里会调
/// `InetSocketAddress.getHostName()` —— **把服务端 IP 反解成主机名**。企业 DNS 一般没有
/// 这些 PTR 记录，而 Java 对**失败**的解析结果不缓存，于是**每新建一条会话就白等一次
/// DNS 超时**：
///
/// | 项目 | 实测 |
/// |---|---|
/// | 纯 TCP 建连（不经驱动） | 29~47 ms |
/// | `nslookup <IP>`（回 NXDOMAIN） | 143 ms |
/// | Java `InetAddress.getHostName()` | **4573 ms**（且第二次仍 4579ms） |
/// | 同上，但给 JVM 一份自有 hosts | **16 ms** |
///
/// 于是「展开连接要等 5 秒、点开列表要等 10 秒（两条会话）」与数据库、网络都无关。
/// 宿主是独立 JVM ⇒ 用 `-Djdk.net.hosts.file` 指到这里生成的文件即可，
/// **不需要管理员权限，也不改系统配置**。
///
/// 生成内容 = 系统 hosts 的全部内容（否则 `localhost` 之类的解析会丢）+ 每个连接主机一行。
pub fn prepare_agent_hosts(hosts: &[String]) -> Option<PathBuf> {
    let target = paths::agent_hosts_path();
    // 注释一律用 ASCII：hosts 文件由系统解析器读取，编码不由我们决定，
    // 中文注释在某些代码页下会被读坏（而且注释行一旦被读坏就可能破坏整行解析）。
    let mut text = String::from(
        "# Generated by DBMind. Do not edit by hand.\n\
         # Used as the host JVM's -Djdk.net.hosts.file.\n\
         # Why: the MySQL driver resolves the server's IP back to a host name while\n\
         # opening a connection; without a PTR record that costs ~4.6s per new session.\n",
    );
    // 系统 hosts 的内容原样带上：这份文件会**取代**系统 hosts，
    // 少了 localhost 之类的既有映射，用户那些连本机数据库的连接反而会坏。
    for candidate in [
        std::env::var("SystemRoot").ok().map(|root| {
            Path::new(&root)
                .join("System32")
                .join("drivers")
                .join("etc")
                .join("hosts")
        }),
        Some(PathBuf::from("/etc/hosts")),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(existing) = std::fs::read_to_string(&candidate) {
            text.push_str("\n# ---- verbatim from the system hosts file ----\n");
            text.push_str(&existing);
            text.push('\n');
            break;
        }
    }
    text.push_str("\n# ---- generated by DBMind ----\n");
    text.push_str("127.0.0.1 localhost\n::1 localhost\n");
    // 占位名带序号：反解只需要「有个名字」，具体叫什么无所谓。
    // 去重后排序，保证同一批连接生成出来的文件是稳定的。
    let mut unique: Vec<String> = hosts
        .iter()
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty() && host != "localhost" && host != "127.0.0.1")
        .collect();
    unique.sort();
    unique.dedup();
    for (index, host) in unique.iter().enumerate() {
        text.push_str(&format!("{host} dbmind-host-{index}\n"));
    }

    if let Err(err) = paths::ensure_parent(&target) {
        eprintln!("[dbmind] 准备宿主 hosts 失败（将退回系统解析）: {err}");
        return None;
    }
    if let Err(err) = std::fs::write(&target, text) {
        eprintln!("[dbmind] 写入宿主 hosts 失败（将退回系统解析）: {err}");
        return None;
    }
    Some(target)
}

/// java 主版本的缓存。
///
/// 就绪报告会被界面反复调用（`availability_all` 一次问 4 个宿主），
/// 而每次问都要跑一次 `java -version`（一次 JVM 启动 ~100ms）。版本不会变，缓存住即可。
fn java_major_cache() -> &'static Mutex<HashMap<PathBuf, u32>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<PathBuf, u32>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 某个 java 可执行文件的主版本号（带缓存）。
pub fn java_major_version(java: &Path) -> Option<u32> {
    {
        let cache = java_major_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(version) = cache.get(java) {
            return Some(*version);
        }
    }
    // `-version` 把版本写到 **stderr**（`--version` 才写 stdout），两个流都收
    let mut probe = Command::new(java);
    hide_console(&mut probe); // 否则这次探测会在桌面版里闪一下黑窗（一次就绪检查会跑好几遍）
    let output = probe.arg("-version").output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    let version = parse_java_major(&text)?;
    java_major_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(java.to_path_buf(), version);
    Some(version)
}

/// 从 `java -version` 的输出里取主版本号。
///
/// 两种写法都要认：`openjdk version "17.0.20.1"`（现代）与 `java version "1.8.0_381"`（旧格式，1.x = x）。
pub fn parse_java_major(text: &str) -> Option<u32> {
    let start = text.find('"')? + 1;
    let end = text[start..].find('"')? + start;
    let mut parts = text[start..end].split(['.', '_', '-']);
    let first = parts.next()?.trim().parse::<u32>().ok()?;
    if first == 1 {
        // 1.8.0_381 → 8
        parts.next()?.trim().parse::<u32>().ok()
    } else {
        Some(first)
    }
}

/// 单个宿主的就绪状态。`java` 与 `jar` 任一缺失都算不就绪，并给出**可执行**的原因。
pub fn availability_for(spec: &AgentHostSpec) -> AgentAvailability {
    let java = resolve_java();
    let jar = resolve_agent_jar(spec);
    let java_ok = java
        .as_deref()
        .map(|p| p.is_file() || p == Path::new("java"))
        .unwrap_or(false);
    // 测试构建下「不就绪」是**刻意**的，原因要说清是哪个开关 —— 否则会被误读成
    // 「驱动真没装」（我上一轮就是被这类误判绊了两次）。
    let reason = if !agent_hosts_visible() {
        Some(format!(
            "测试构建默认不加载 agent 宿主，故看不到 {}（设 {} = 1 可开启真实宿主）",
            spec.jar_name, AGENTS_TEST_ENV
        ))
    } else if !java_ok {
        Some("找不到 Java 运行时：设置 DBMIND_JAVA 或 JAVA_HOME，或把 java 放进 PATH".to_string())
    } else if jar.is_none() {
        Some(format!(
            "找不到 agent 包（{}）：{}",
            spec.jar_name,
            spec.build_hint()
        ))
    } else {
        // java 与 jar 都在，最后再看**版本**：宿主编译到 17，JDK 11 上跑不起来。
        // 这一步必须在连接之前做 —— 否则用户看到的是「agent 进程已退出」，
        // 而真正的原因（版本不匹配）在进程 stderr 里，得翻日志才知道。
        // 版本读不出来时**不**判不就绪：无法证明它不行，就不该编一个失败出来，
        // 真出问题还有启动后的 stderr 兜底。
        match java.as_deref().and_then(java_major_version) {
            Some(runtime) if runtime < spec.min_java => Some(format!(
                "Java 版本过低：宿主 {} 需要 Java {} 及以上，当前的 java（{}）是 {}。\
                 请安装 JDK {} 并设置 DBMIND_JAVA 指向它（或改 JAVA_HOME；注意 JAVA_HOME 会影响别的工具）",
                spec.jar_name,
                spec.min_java,
                java.as_deref().unwrap_or(Path::new("java")).display(),
                runtime,
                spec.min_java
            )),
            _ => None,
        }
    };
    AgentAvailability {
        id: spec.id.to_string(),
        ready: reason.is_none(),
        java: java.map(|p| p.display().to_string()),
        jar: jar.map(|p| p.display().to_string()),
        reason,
    }
}

/// 所有宿主的就绪状态（按声明顺序）。
pub fn availability_all() -> Vec<AgentAvailability> {
    AgentHostSpec::all().iter().map(availability_for).collect()
}

/// 静态判断「是否有宿主能承载该类型」——不启动进程，供类型目录与就绪报告使用。
///
/// 与运行期握手（`AgentHost::supports`）配合：这里决定「类型目录里是否算已接入」，
/// 握手决定「真正连接时该宿主认不认」。前者要快、后者要准。
pub fn has_host_for(kind: crate::ConnectionKind) -> bool {
    let specs = AgentHostSpec::all();
    if let Some(key) = kind.agent_key() {
        if specs.iter().any(|spec| spec.declares(key)) {
            return true;
        }
    }
    if kind.is_jdbc() {
        return specs.iter().any(|spec| spec.generic);
    }
    false
}

/// 某 agentKey 的驱动 jar 目录：`~/.dbmind/drivers/<agentKey>/`。
pub fn driver_dir(agent_key: &str) -> PathBuf {
    paths::home_dir().join("drivers").join(agent_key)
}

/// 驱动 jar 的搜索路径：类型专属目录 + `DBMIND_DRIVER_DIRS`（用系统分隔符分隔）。
pub fn driver_dirs(agent_key: &str) -> Vec<PathBuf> {
    let mut dirs = vec![driver_dir(agent_key)];
    if let Ok(extra) = std::env::var("DBMIND_DRIVER_DIRS") {
        for part in std::env::split_paths(&extra) {
            if !part.as_os_str().is_empty() {
                dirs.push(part);
            }
        }
    }
    dirs
}

/// Maven Central 的 jar 下载地址：坐标 `group:artifact:version[:classifier]` ⇒ URL。
///
/// **只做「坐标 → 地址」这一步，不含任何网络**：内核保持零网络依赖（下载由壳层做，
/// 见 CLI 里那段说明）。但因为 CLI / 桌面 / Web 都要下载，**地址的拼法必须有且只有一份** ——
/// 三处各拼一遍，早晚有一处先漂。
pub fn driver_artifact_url(artifact: &str) -> Result<String> {
    driver_artifact_url_with_base(artifact, None)
}

/// Maven Central 的仓库根（默认下载源）。
pub const MAVEN_CENTRAL: &str = "https://repo.maven.apache.org/maven2";

/// 同上，但可指定**镜像根**（设置里的「驱动下载镜像」，如公司内网 Nexus / 阿里云镜像）。
///
/// 只替换仓库根、保留内核拼好的那段路径：镜像站都是按 Maven 仓库布局放的，
/// 而"坐标 → 路径"的拼法必须只有一份（三处各拼一遍，早晚有一处先漂）。
/// 传 `None` / 空串就是 Maven Central —— 与不带镜像时**逐字节一致**。
pub fn driver_artifact_url_with_base(artifact: &str, mirror: Option<&str>) -> Result<String> {
    let (group, name, version, classifier) = split_artifact(artifact)?;
    let suffix = classifier.map(|c| format!("-{c}")).unwrap_or_default();
    let base = mirror
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .map(|base| base.trim_end_matches('/').to_string())
        .unwrap_or_else(|| MAVEN_CENTRAL.to_string());
    Ok(format!(
        "{base}/{}/{}/{}/{}-{}{}.jar",
        group.replace('.', "/"),
        name,
        version,
        name,
        version,
        suffix
    ))
}

/// 驱动 jar 的落地文件名（与坐标一致，一眼能看出它来自哪个版本）。
pub fn driver_jar_name(artifact: &str) -> Result<String> {
    let (_, name, version, classifier) = split_artifact(artifact)?;
    Ok(match classifier {
        Some(classifier) => format!("{name}-{version}-{classifier}.jar"),
        None => format!("{name}-{version}.jar"),
    })
}

/// 解析 `group:artifact:version[:classifier]`。
///
/// 需要 classifier，是因为有些驱动**必须**用带分类器的包才能跑：ClickHouse 的
/// `clickhouse-jdbc` 默认包是瘦包（1 MB），缺 slf4j / clickhouse-http-client 等
/// 传递依赖，装载时以 `NoClassDefFoundError: org/slf4j/LoggerFactory` 失败；
/// 而 `driver-installed` 只看 jar 在不在，于是界面显示「驱动已就绪」、连的时候才炸。
/// 官方可用的完整包是 `...:0.7.2:all`（8.4 MB）。
fn split_artifact(artifact: &str) -> Result<(&str, &str, &str, Option<&str>)> {
    let parts: Vec<&str> = artifact.split(':').collect();
    let shape_ok = matches!(parts.len(), 3 | 4);
    if !shape_ok || parts.iter().any(|part| part.trim().is_empty()) {
        return Err(DbMindError::new(
            ErrorCode::DriverNotReady,
            format!("坐标格式应为 group:artifact:version[:classifier]，实际为 {artifact}"),
        ));
    }
    let classifier = parts.get(3).map(|value| value.trim());
    Ok((parts[0].trim(), parts[1].trim(), parts[2].trim(), classifier))
}

/// 校验用户上传的驱动文件名，并给出它该落地的绝对路径。
///
/// 校验**不是可选项**：这个名字来自 HTTP 请求，而它要落进用户的文件系统 ——
/// 不挡住 `../`、路径分隔符、非 `.jar` 扩展名，就等于把「往任意路径写文件」的自由
/// 交给了调用方。
pub fn driver_upload_target(agent_key: &str, filename: &str) -> Result<PathBuf> {
    let name = filename.trim();
    let reject = |why: &str| {
        DbMindError::new(
            ErrorCode::DriverNotReady,
            format!("驱动文件名不合法（{why}）：{filename}"),
        )
        .with_detail("只接受形如 h2-2.3.232.jar 的 .jar 文件名，不能带目录".to_string())
    };
    if name.is_empty() {
        return Err(reject("空文件名"));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(reject("含路径"));
    }
    if !name.to_ascii_lowercase().ends_with(".jar") {
        return Err(reject("不是 .jar"));
    }
    Ok(driver_dir(agent_key).join(name))
}

/// 把驱动 jar 写进该 agentKey 的驱动目录（目录不存在就建）。返回落地的绝对路径。
///
/// 这是「上传驱动」落地的那一步：写完**无需别的配置** —— 内核按 `driver_dirs` 找 jar，
/// 宿主拿到的就是这些文件。
pub fn install_driver_jar(agent_key: &str, filename: &str, bytes: &[u8]) -> Result<PathBuf> {
    if bytes.is_empty() {
        return Err(DbMindError::new(ErrorCode::DriverNotReady, "驱动文件是空的"));
    }
    let target = driver_upload_target(agent_key, filename)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, bytes)?;
    Ok(target)
}

/// 已安装的驱动 jar（目录不存在即视为缺失）。
pub fn installed_driver_jars(agent_key: &str) -> Vec<PathBuf> {
    let mut jars = Vec::new();
    for dir in driver_dirs(agent_key) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("jar") {
                jars.push(path);
            }
        }
    }
    jars.sort();
    jars
}

// ---------------------------------------------------------------- 协议响应

#[derive(Debug, Deserialize)]
struct AgentResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

/// agent 侧返回的错误码 -> 内核错误码。未知码归到内部错误但保留原文，避免丢信息。
fn map_agent_code(code: &str) -> ErrorCode {
    match code {
        "DBMIND-CONN-0001" => ErrorCode::ConnNotFound,
        "DBMIND-CONN-0002" => ErrorCode::ConnInvalid,
        "DBMIND-CONN-0003" => ErrorCode::ConnConnectFailed,
        "DBMIND-QUERY-0001" => ErrorCode::QueryInvalid,
        "DBMIND-QUERY-0002" => ErrorCode::QueryFailed,
        "DBMIND-QUERY-0003" => ErrorCode::QueryTimeout,
        "DBMIND-QUERY-0004" => ErrorCode::QueryCanceled,
        "DBMIND-DRV-0002" => ErrorCode::DriverNotReady,
        _ => ErrorCode::QueryFailed,
    }
}

/// 会话是否已因进程重启而失效——用代际判断，不匹配文案。
#[derive(Debug, Clone)]
struct CachedSession {
    id: String,
    server_version: Option<String>,
    generation: u64,
}

// ---------------------------------------------------------------- 宿主

pub struct AgentHost {
    /// 本宿主承载哪一类 agentKey（决定用哪个可执行包、以及就绪判定）
    spec: AgentHostSpec,
    process: Mutex<Option<AgentProcess>>,
    sessions: Mutex<HashMap<String, CachedSession>>,
    generation: AtomicU64,
    counter: AtomicU64,
    /// 单次调用默认超时（可被调用方覆盖）。
    default_timeout: Duration,
    /// 测试专用：`ensure_process` 被调用的次数。
    ///
    /// 存在的唯一理由是钉住「收掉没起来的宿主时不该去启动它」这条性质 ——
    /// 在默认（宿主不可见）的环境里，走弯路与不走弯路**结果一样**，没有别的痕迹可断言。
    #[cfg(test)]
    ensure_calls: AtomicU64,
}

struct AgentProcess {
    child: Child,
    stdin: ChildStdin,
    pending: Arc<Mutex<HashMap<String, Sender<AgentResponse>>>>,
    agent_keys: Vec<String>,
    /// 是否为通用宿主（能承载任意 JDBC 类型，凭 YAML 里的 driverClass/urlTemplate 接）。
    /// 专属宿主（如未来的 MongoDB/Redis）会声明自己支持的 agentKey 列表。
    generic: bool,
}

impl Default for AgentHost {
    fn default() -> Self {
        Self::new(AgentHostSpec::jdbc())
    }
}

impl AgentHost {
    pub fn new(spec: AgentHostSpec) -> Self {
        Self {
            spec,
            process: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            counter: AtomicU64::new(0),
            default_timeout: DEFAULT_TIMEOUT,
            #[cfg(test)]
            ensure_calls: AtomicU64::new(0),
        }
    }

    /// 测试专用：`ensure_process` 的调用次数（见字段文档）。
    #[cfg(test)]
    pub(crate) fn ensure_calls(&self) -> u64 {
        self.ensure_calls.load(Ordering::Relaxed)
    }

    pub fn spec(&self) -> &AgentHostSpec {
        &self.spec
    }

    /// 本宿主的就绪状态（不启动进程）。
    pub fn availability(&self) -> AgentAvailability {
        availability_for(&self.spec)
    }

    /// agent 是否支持某个 agentKey（必要时启动进程并握手）。
    ///
    /// 通用宿主（JDBC）声明 `generic`：它按内核给的 driverClass/urlTemplate 工作，
    /// 因此天然承载所有 JDBC 类型；专属宿主则声明自己支持的 key 列表。
    pub fn supports(&self, agent_key: &str) -> Result<bool> {
        self.ensure_process()?;
        let guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        Ok(match guard.as_ref() {
            Some(process) => {
                process.generic
                    || process.agent_keys.iter().any(|k| k == agent_key)
                    || self.spec.declares(agent_key)
            }
            None => false,
        })
    }

    /// 握手：返回 agent 支持的 agentKey 列表。
    pub fn handshake(&self) -> Result<Vec<String>> {
        self.ensure_process()?;
        let guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        Ok(guard.as_ref().map(|p| p.agent_keys.clone()).unwrap_or_default())
    }

    /// 当前代际（每次拉起进程自增）。会话缓存据此判断有效性。
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn cached_session(&self, key: &str) -> Option<(String, Option<String>)> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let entry = sessions.get(key)?;
        if entry.generation == self.generation() {
            Some((entry.id.clone(), entry.server_version.clone()))
        } else {
            None
        }
    }

    pub fn cache_session(&self, key: &str, id: &str, server_version: Option<String>) {
        let generation = self.generation();
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.insert(
            key.to_string(),
            CachedSession {
                id: id.to_string(),
                server_version,
                generation,
            },
        );
    }

    pub fn forget_session(&self, key: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    /// 发起一次调用。`id` 用于匹配响应与取消：查询传 executionId，其它可传 None。
    pub fn call(
        &self,
        id: Option<&str>,
        method: &str,
        params: Map<String, Value>,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        self.ensure_process()?;
        let request_id = id
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{method}-{}", self.counter.fetch_add(1, Ordering::Relaxed)));

        let (sender, receiver) = mpsc::channel();
        {
            let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
            let Some(process) = guard.as_mut() else {
                return Err(DbMindError::new(ErrorCode::DriverNotReady, "agent 进程不可用"));
            };
            process
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(request_id.clone(), sender);

            let mut envelope = Map::new();
            envelope.insert("id".to_string(), Value::String(request_id.clone()));
            envelope.insert("method".to_string(), Value::String(method.to_string()));
            envelope.insert("protocolVersion".to_string(), Value::from(PROTOCOL_VERSION));
            for (k, v) in params {
                envelope.insert(k, v);
            }
            let line = serde_json::to_string(&Value::Object(envelope))?;

            if let Err(err) = writeln!(process.stdin, "{line}").and_then(|_| process.stdin.flush()) {
                drop(guard);
                self.kill();
                return Err(
                    DbMindError::new(ErrorCode::ConnConnectFailed, "向 agent 写入请求失败")
                        .with_detail(err.to_string()),
                );
            }
        }

        match receiver.recv_timeout(timeout.unwrap_or(self.default_timeout)) {
            Ok(response) if response.ok => Ok(response.result),
            Ok(response) => Err(DbMindError::new(
                map_agent_code(response.code.as_deref().unwrap_or_default()),
                response.message.unwrap_or_else(|| "agent 返回失败".to_string()),
            )
            .with_detail(response.detail.unwrap_or_default())),
            Err(RecvTimeoutError::Timeout) => {
                self.drop_pending(&request_id);
                Err(DbMindError::new(
                    ErrorCode::QueryTimeout,
                    format!(
                        "agent 调用 {method} 超过 {} ms 未返回",
                        timeout.unwrap_or(self.default_timeout).as_millis()
                    ),
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.kill();
                // 带上宿主最后几行 stderr：没有它，「进程已退出」是一句无法行动的错误
                let tail = stderr_tail(&self.spec, 6);
                let path = agent_stderr_path(&self.spec);
                // 「宿主需要的 Java 比当前运行时新」是最常见的一种「一起来就死」，
                // 而且从这一行 stderr 就能判定。这里补一句可行动的指引 ——
                // 不写死版本号：报错原文里已经有 61.0 / 55.0，说死了反而会在升级后过期。
                let hint = if tail.contains("UnsupportedClassVersionError") {
                    "\n可行动建议：宿主编译目标高于本机 Java 运行时（上面两行分别是所需与当前版本）。\
                     安装匹配的 JDK 后，用 DBMIND_JAVA 指向它的 java 可执行文件；\
                     或把 agents/*/pom.xml 的 maven.compiler.release 降到本机 JDK 能跑的版本并重新打包。"
                } else {
                    ""
                };
                let detail = if tail.is_empty() {
                    format!("宿主没有任何输出；stderr 文件：{}", path.display())
                } else {
                    format!("宿主最后输出：{tail}（完整 stderr：{}）{hint}", path.display())
                };
                Err(
                    DbMindError::new(ErrorCode::ConnConnectFailed, "agent 进程已退出")
                        .with_detail(detail),
                )
            }
        }
    }

    /// 请求 agent 中断某个正在执行的请求（JDBC `Statement.cancel()`）。
    pub fn cancel(&self, request_id: &str) -> Result<bool> {
        let mut params = Map::new();
        params.insert("requestId".to_string(), Value::String(request_id.to_string()));
        match self.call(None, "cancel", params, Some(Duration::from_secs(5))) {
            Ok(value) => Ok(value.get("cancelled").and_then(|v| v.as_bool()).unwrap_or(false)),
            Err(err) => {
                // 取消失败不应把「取消」变成错误：记录后返回 false，调用方继续走超时逻辑
                tracing::warn!(target: "dbmind::agent", error = %err, "请求 agent 取消失败");
                Ok(false)
            }
        }
    }

    /// 关掉一条会话：**真的让宿主 close**（不只是内核忘掉它）。
    ///
    /// 用途：全局会话配额回收空闲会话。内核才知道「这条会话此刻空闲」（池的槽位状态），
    /// 但关连接的动作只能由宿主做（句柄在它手里）。
    ///
    /// 无论成败都清掉内核的会话缓存：既然决定不要它了，就不该再被复用。
    pub fn disconnect(&self, session_id: &str) -> Result<()> {
        let mut params = Map::new();
        params.insert("sessionId".to_string(), Value::String(session_id.to_string()));
        let outcome = self.call(None, "disconnect", params, Some(Duration::from_secs(10)));
        self.forget_session(session_id);
        outcome.map(|_| ())
    }

    pub fn forget_all_sessions(&self) {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// 关闭 agent 进程（内核退出或显式释放时调用）。
    ///
    /// 协议约定：`shutdown` 的**回执意味着会话已全部关闭并落盘**（agent 侧先落盘再回执）。
    /// 因此收到回执后先等它自己退出，再兜底强杀 —— 直接强杀会让嵌入式引擎
    /// （H2 / Derby 等）丢失最后一次写，表现为「建表成功但表不见了」。
    /// 收掉宿主进程；**没有进程就什么都不做**。
    ///
    /// 这里不能直接 `call`：`call` 会先 `ensure_process()` —— 于是「从未启动过」的宿主
    /// 也会为了说一声再见而被**拉起来一个 JVM**，再立刻杀掉。实测（打开测试宿主开关时）：
    /// 内核实例析构时每个宿主各起一个 4 个 JVM，21 个引擎 fixture 就是 84 个进程 ——
    /// 纯浪费，而且是**内存压力、孤儿进程与那些崩溃转储的来源之一**。
    /// 没起来的宿主，本来就没有什么可收的。
    pub fn shutdown(&self) {
        let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        let started = match guard.as_mut() {
            Some(process) => matches!(process.child.try_wait(), Ok(None)),
            None => false,
        };
        if !started {
            *guard = None;
            drop(guard);
            self.forget_all_sessions();
            return;
        }
        drop(guard);

        // 已经起来了：先好好道别（让它自己关连接、清资源），再兜底杀
        let _ = self.call(None, "shutdown", Map::new(), Some(Duration::from_secs(3)));
        self.wait_for_exit(Duration::from_millis(1_500));
        self.kill();
    }

    fn wait_for_exit(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let alive = {
                let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
                match guard.as_mut() {
                    Some(process) => matches!(process.child.try_wait(), Ok(None)),
                    None => false,
                }
            };
            if !alive {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn drop_pending(&self, request_id: &str) {
        let guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(process) = guard.as_ref() {
            process
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(request_id);
        }
    }

    /// 强杀进程并清空会话。**不做重试**：重试策略由调用方按「调用是否幂等」决定。
    fn kill(&self) {
        let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut process) = guard.take() {
            let _ = process.child.kill();
            let _ = process.child.wait();
        }
        drop(guard);
        self.forget_all_sessions();
    }

    fn ensure_process(&self) -> Result<()> {
        #[cfg(test)]
        self.ensure_calls.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        let alive = match guard.as_mut() {
            Some(process) => matches!(process.child.try_wait(), Ok(None)),
            None => false,
        };
        if alive {
            return Ok(());
        }
        if guard.is_some() {
            // 进程已退出：清掉会话（代际变化会让缓存失效）
            *guard = None;
            self.sessions.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }

        let availability = availability_for(&self.spec);
        if !availability.ready {
            return Err(DbMindError::new(
                ErrorCode::DriverNotReady,
                availability
                    .reason
                    .unwrap_or_else(|| "agent 运行时未就绪".to_string()),
            )
            .with_detail(format!("宿主 {}", self.spec.id)));
        }
        let java = availability.java.unwrap_or_else(|| "java".to_string());
        let jar = availability.jar.unwrap_or_default();

        let mut command = Command::new(&java);
        // 宿主是长驻 JVM：不藏窗口的话，桌面版里从连上那一刻起就一直挂着一个黑窗
        hide_console(&mut command);
        command.arg("-Dfile.encoding=UTF-8");
        // 让宿主用一个**自有 hosts 文件**（内容见 prepare_agent_hosts）：
        // 否则 MySQL 驱动每次建连接反解服务端 IP 都要白等约 4.6 秒。
        // 文件不存在就不加这个参数 —— 行为与加之前完全一样，纯回退。
        let agent_hosts = paths::agent_hosts_path();
        if agent_hosts.is_file() {
            command.arg(format!("-Djdk.net.hosts.file={}", agent_hosts.display()));
        }
        command
            .arg("-jar")
            .arg(&jar)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr 落盘而不是丢掉：宿主是独立 JVM，jar 不可执行 / 版本不匹配 / 缺依赖时
            // 它会**立刻退出**，而界面上只会看到「agent 进程已退出」——没有原因，日志里也查不到。
            .stderr(agent_stderr_stdio(&self.spec));
        let mut child = command.spawn().map_err(|err| {
            DbMindError::new(ErrorCode::DriverNotReady, "启动 agent 进程失败")
                .with_detail(format!("{java} -jar {jar}: {err}"))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| DbMindError::new(ErrorCode::DriverNotReady, "无法获取 agent stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| DbMindError::new(ErrorCode::DriverNotReady, "无法获取 agent stdout"))?;

        let pending: Arc<Mutex<HashMap<String, Sender<AgentResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = pending.clone();
        std::thread::Builder::new()
            .name("dbmind-agent-reader".to_string())
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let value: Value = match serde_json::from_str(&line) {
                        Ok(value) => value,
                        Err(err) => {
                            tracing::warn!(target: "dbmind::agent", error = %err, "agent 输出不是合法 JSON");
                            continue;
                        }
                    };
                    let id = value
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    match serde_json::from_value::<AgentResponse>(value) {
                        Ok(response) => {
                            let sender = reader_pending
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&id);
                            if let Some(sender) = sender {
                                let _ = sender.send(response);
                            }
                        }
                        Err(err) => {
                            tracing::warn!(target: "dbmind::agent", error = %err, "agent 响应结构不符");
                        }
                    }
                }
                // 读到 EOF：进程结束，唤醒所有等待者（它们会得到 Disconnected）
                reader_pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
            })
            .map_err(|err| {
                DbMindError::new(ErrorCode::DriverNotReady, "启动 agent 读取线程失败")
                    .with_detail(err.to_string())
            })?;

        *guard = Some(AgentProcess {
            child,
            stdin,
            pending,
            agent_keys: Vec::new(),
            generic: false,
        });
        self.generation.fetch_add(1, Ordering::SeqCst);
        drop(guard);

        // 握手：拿到宿主类型与支持的 agentKey（同时验证协议可用）
        let result = self.call(None, "handshake", Map::new(), Some(Duration::from_secs(20)))?;
        let keys: Vec<String> = result
            .get("agentKeys")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        // 宿主没自报 generic 时以静态声明为准（专属宿主忘了声明 generic 也不会误判）
        let generic = result
            .get("generic")
            .and_then(|v| v.as_bool())
            .unwrap_or(self.spec.generic);
        let mut guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(process) = guard.as_mut() {
            process.agent_keys = keys;
            process.generic = generic;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 行/结构映射

/// 行反序列化：同时吃两种格式。
///
/// 旧格式是**每格一个带标签对象**（`{"t":"integer","v":1}`），新格式是**裸值**
/// （`[1,"a",null]`，由宿主在收到请求里的 `compact:true` 时给出）。裸值让类型由 JSON
/// 自身表达即可：整数与小数天然区分（`1` 与 `1.0`），null 就是 null，字符串就是字符串 ——
/// 语义完全等价，报文却小 5~8 倍，导出的逐行开销大头就在这里。
///
/// 按形状自适应（而不是看 `compactRows` 标记）：这样内核与宿主可以各自独立升级或回退，
/// 不会出现"必须同时换，否则崩"的部署约束。
fn de_rows<'de, D>(deserializer: D) -> std::result::Result<Vec<Vec<crate::types::CellValue>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_seq(RowsVisitor)
}

/// 行的 Visitor：让解析器**直接把标量喂进来**，不再每个格子先造一个 serde_json::Value。
///
/// 为什么要自己写：Vec<Vec<Value>> 那条路每格都要构造一个 Value（20 万行 × 12 列
/// = 240 万次分配），随后再遍历一遍转成 CellValue。JSON 里本来只有那几种标量，
/// 交给 visit_null / visit_i64 / visit_f64 / visit_str 就够 —— 少一层分配、少一趟遍历。
///
/// 两种格式都吃（见下面的 Cell）：裸值（紧凑格式）与带标签对象（旧格式 / blob）。
/// 按形状自适应而不是看 compactRows 标记：内核与宿主可以各自独立升级或回退。
struct RowsVisitor;

impl<'de> serde::de::Visitor<'de> for RowsVisitor {
    type Value = Vec<Vec<crate::types::CellValue>>;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("行数组（数组的数组）")
    }

    fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut rows = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(RowCells(row)) = seq.next_element::<RowCells>()? {
            rows.push(row);
        }
        Ok(rows)
    }
}

/// 一行（若干 Cell）
struct RowCells(Vec<crate::types::CellValue>);

impl<'de> serde::Deserialize<'de> for RowCells {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct RowVisitor;

        impl<'de> serde::de::Visitor<'de> for RowVisitor {
            type Value = RowCells;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("一行（单元格数组）")
            }

            fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut cells = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(Cell(cell)) = seq.next_element::<Cell>()? {
                    cells.push(cell);
                }
                Ok(RowCells(cells))
            }
        }

        deserializer.deserialize_seq(RowVisitor)
    }
}

/// 一个单元格：裸值（紧凑格式）与带标签对象（旧格式 / blob）都吃。
struct Cell(crate::types::CellValue);

impl<'de> serde::Deserialize<'de> for Cell {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct CellVisitor;

        impl<'de> serde::de::Visitor<'de> for CellVisitor {
            type Value = Cell;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("单元格（裸值或带标签对象）")
            }

            fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(Cell(crate::types::CellValue::Null))
            }

            fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(Cell(crate::types::CellValue::Null))
            }

            fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                deserializer.deserialize_any(CellVisitor)
            }

            fn visit_bool<E>(self, flag: bool) -> std::result::Result<Self::Value, E> {
                // 布尔归 0/1：与宿主侧的约定一致（跨语言表示布尔最省事）
                Ok(Cell(crate::types::CellValue::Integer(if flag { 1 } else { 0 })))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(Cell(crate::types::CellValue::Integer(value)))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(Cell(crate::types::CellValue::Integer(
                    value.min(i64::MAX as u64) as i64,
                )))
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E> {
                Ok(Cell(crate::types::CellValue::Real(value)))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Cell(crate::types::CellValue::Text(value.to_owned())))
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
                Ok(Cell(crate::types::CellValue::Text(value)))
            }

            fn visit_map<A>(self, map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                // 带标签对象：旧格式的每一格，以及紧凑格式里唯一的例外 blob
                //（它要带长度与预览字节，裸值表达不了）。这条不在热路径上。
                let object = serde_json::Map::<String, Value>::deserialize(
                    serde::de::value::MapAccessDeserializer::new(map),
                )?;
                Ok(Cell(serde_json::from_value(Value::Object(object))
                    .unwrap_or(crate::types::CellValue::Null)))
            }
        }

        deserializer.deserialize_any(CellVisitor)
    }
}

/// agent 返回的结果集（与内核 `QueryResult` 的差别：不带连接与执行标识）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentQueryResult {
    #[serde(default)]
    pub columns: Vec<crate::types::ColumnMeta>,
    // 行的格式由请求里的 `compact` 决定，这里按形状自适应（见 de_rows）
    #[serde(default, deserialize_with = "de_rows")]
    pub rows: Vec<Vec<crate::types::CellValue>>,
    #[serde(default)]
    pub row_count: usize,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub affected_rows: Option<i64>,
    #[serde(default)]
    pub notices: Vec<String>,
    /// 宿主声明的行标识，**只有它能证明时才给**。
    ///
    /// Mongo 的 `_id` 到了结果里只是一串十六进制：它原本是 ObjectId 还是字符串，
    /// 内核看不见。写回时按错类型比较会**匹配不到任何行**（「保存成功、0 行受影响」），
    /// 所以这件事由宿主说，内核不猜。
    #[serde(default)]
    pub identity: Option<crate::types::RowIdentity>,
}

/// agent 返回的表清单 -> 内核结构。
pub fn map_tables(value: Value) -> Vec<TableInfo> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let name = item.get("name")?.as_str()?.to_string();
                    let kind = match item.get("kind").and_then(|v| v.as_str()) {
                        Some("view") => TableKind::View,
                        _ => TableKind::Table,
                    };
                    Some(TableInfo {
                        name,
                        kind,
                        row_estimate: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 渲染 JDBC URL：把 YAML 模板里的占位符替换成连接字段。
///
/// 缺字段时给出**明确**的错误（哪个字段缺），而不是拼出一个必然失败 URL 再去猜。
///
/// 另外把文件路径里的反斜杠统一成 `/`：JDBC URL 是 URL，不是 Windows 路径 ——
/// H2 之类的驱动拿到 `C:\a\b` 会解析失败甚至静默退化成内存库（实测踩过：
/// 建表「成功」但那是个内存库，下一个连接当然是空的）。
pub fn render_jdbc_url(template: &str, config: &ConnectionConfig) -> Result<String> {
    let mut url = template.to_string();
    let replacements: [(&str, Option<String>); 5] = [
        ("{host}", config.host.clone()),
        ("{port}", config.resolved_port().map(|p| p.to_string())),
        ("{database}", config.database.clone()),
        ("{filePath}", config.resolved_file().map(|s| s.replace('\\', "/"))),
        ("{username}", config.username.clone()),
    ];
    for (token, value) in replacements {
        if url.contains(token) {
            let value = value.ok_or_else(|| {
                DbMindError::new(
                    ErrorCode::ConnInvalid,
                    format!("连接「{}」缺少 {token} 对应的字段", config.name),
                )
            })?;
            url = url.replace(token, &value);
        }
    }
    if url.contains('{') {
        return Err(DbMindError::new(
            ErrorCode::ConnInvalid,
            format!("JDBC URL 模板中存在无法识别的占位符：{url}"),
        ));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConnectionKind;

    /// 驱动镜像：**只替换仓库根**，路径部分与 Maven Central 那份逐字节一致。
    ///
    /// 这条性质就是「设置里的驱动镜像真的生效」的全部内容 —— 内网 Nexus / 阿里云镜像
    /// 都按 Maven 仓库布局存放，路径拼错一段就是 404，而用户看到的只会是"驱动下载失败"。
    #[test]
    fn 驱动镜像替换仓库根而路径不变() {
        let artifact = "org.postgresql:postgresql:42.7.3";
        let central = driver_artifact_url(artifact).unwrap();
        assert_eq!(
            central,
            "https://repo.maven.apache.org/maven2/org/postgresql/postgresql/42.7.3/postgresql-42.7.3.jar"
        );
        // 镜像：尾斜杠要被吃掉（否则拼出 `//org/...`）
        let mirrored = driver_artifact_url_with_base(
            artifact,
            Some("https://maven.aliyun.com/repository/public/"),
        )
        .unwrap();
        assert_eq!(
            mirrored,
            "https://maven.aliyun.com/repository/public/org/postgresql/postgresql/42.7.3/postgresql-42.7.3.jar"
        );
        // None / 空串 / 纯空白都等于"没设镜像"：清空镜像输入框不该拼出坏地址
        for empty in [None, Some(""), Some("   ")] {
            assert_eq!(
                driver_artifact_url_with_base(artifact, empty).unwrap(),
                central
            );
        }
        // 带 classifier 的坐标（如 mssql 的 jre11）也要跟着走同一个仓库根
        let classified = driver_artifact_url_with_base(
            "com.microsoft.sqlserver:mssql-jdbc:12.8.1:jre11",
            Some("https://mirror.example/maven2"),
        )
        .unwrap();
        assert_eq!(
            classified,
            "https://mirror.example/maven2/com/microsoft/sqlserver/mssql-jdbc/12.8.1/mssql-jdbc-12.8.1-jre11.jar"
        );
    }

    #[test]
    fn url_模板按连接字段渲染() {
        let config = ConnectionConfig::new("pg", ConnectionKind::Postgresql)
            .with_host("10.0.0.5")
            .with_database("app");
        let url = render_jdbc_url(ConnectionKind::Postgresql.jdbc_url_template().unwrap(), &config).unwrap();
        assert_eq!(url, "jdbc:postgresql://10.0.0.5:5432/app");
    }

    #[test]
    fn url_模板缺字段时明确指出缺哪个() {
        let config = ConnectionConfig::new("pg", ConnectionKind::Postgresql).with_host("h");
        let err =
            render_jdbc_url(ConnectionKind::Postgresql.jdbc_url_template().unwrap(), &config).unwrap_err();
        assert_eq!(err.code, ErrorCode::ConnInvalid);
        assert!(
            err.message.contains("{database}"),
            "应指出缺哪个字段：{}",
            err.message
        );
    }

    #[test]
    fn 文件型类型用文件路径建_url() {
        let config = ConnectionConfig::new("h2", ConnectionKind::H2).with_file("./data/app");
        let url = render_jdbc_url(ConnectionKind::H2.jdbc_url_template().unwrap(), &config).unwrap();
        assert_eq!(url, "jdbc:h2:file:./data/app");
    }

    #[test]
    fn windows_路径的反斜杠被规整成_url_分隔符() {
        let config =
            ConnectionConfig::new("h2", ConnectionKind::H2).with_file(r"C:\Users\me\.dbmind\data\demo");
        let url = render_jdbc_url(ConnectionKind::H2.jdbc_url_template().unwrap(), &config).unwrap();
        assert_eq!(url, "jdbc:h2:file:C:/Users/me/.dbmind/data/demo");
        assert!(!url.contains('\\'), "JDBC URL 里不应出现反斜杠：{url}");
    }

    #[test]
    fn jdbc_元数据只对_agent_类型存在() {
        assert!(ConnectionKind::Postgresql.is_jdbc());
        assert!(ConnectionKind::H2.is_jdbc());
        assert!(
            !ConnectionKind::Sqlite.is_jdbc(),
            "SQLite 走原生驱动，不该有 JDBC 元数据"
        );
        assert!(!ConnectionKind::Mongodb.is_jdbc(), "MongoDB 不是 JDBC 体系");
        assert!(ConnectionKind::Postgresql
            .jdbc_artifact()
            .unwrap()
            .starts_with("org.postgresql:"));
    }

    #[test]
    fn 驱动目录按_agent_键隔离() {
        let dir = driver_dir("h2");
        assert!(dir.ends_with(Path::new("drivers").join("h2")));
        // 未安装时不应 panic，只返回空
        assert!(installed_driver_jars("definitely-not-installed").is_empty());
    }

    #[test]
    fn 就绪探测给出原因而非直接失败() {
        for spec in AgentHostSpec::all() {
            let availability = availability_for(&spec);
            assert_eq!(availability.id, spec.id);
            if !availability.ready {
                assert!(availability.reason.is_some(), "不就绪时必须说明原因");
                let reason = availability.reason.unwrap();
                // 原因里要能看出「缺什么、怎么补」，而不是一句「未就绪」
                assert!(
                    reason.contains(spec.jar_name) || reason.contains("Java"),
                    "{} 的原因应指出缺什么：{reason}",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn 宿主声明能承载哪些_agent_键() {
        let jdbc = AgentHostSpec::jdbc();
        assert!(jdbc.generic, "JDBC 宿主是通用宿主");
        assert!(jdbc.agent_keys.is_empty(), "通用宿主不枚举 agentKey");

        let mongo = AgentHostSpec::mongodb();
        assert!(!mongo.generic);
        assert!(mongo.declares("mongodb"));
        assert!(!mongo.declares("postgresql"), "专属宿主不应声称承载 JDBC 类型");

        // 宿主 id 与产物名各自唯一
        let all = AgentHostSpec::all();
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a.id, b.id);
                assert_ne!(a.jar_name, b.jar_name);
            }
        }
    }

    /// 收掉「从没起来过」的宿主时，**不能**为了说一声再见把它拉起来。
    ///
    /// 这条性质在默认环境里没有别的痕迹可断言（宿主不可见时那条弯路只是静悄悄地返回
    /// 一个错误），所以用 `ensure_process` 的调用计数钉住。它值一个测试：实测每次内核
    /// 析构都会给 4 个宿主各起一个 JVM 再立刻杀掉（21 个 fixture = 84 个进程）。
    #[test]
    fn 收掉没起来的宿主不会去启动它() {
        let host = AgentHost::new(AgentHostSpec::jdbc());
        host.shutdown();
        assert_eq!(
            host.ensure_calls(),
            0,
            "shutdown 不该调用 ensure_process（那会把没起来的宿主拉起来）"
        );
    }

    /// **宿主层**（默认不跑）：四个宿主都真的起得来、握得上手。
    ///
    /// 为什么需要这一层：「默认单测看不到宿主」换来了速度与可复现，代价是
    /// **「内核 ↔ 宿主」那条链路平时没有任何单测覆盖**。这里把它补回来，而且
    /// **要求宿主真的可用** —— 环境不合就失败并说清怎么补，绝不静默降级
    /// （这一层是被显式点名运行的，含含糊糊地过去等于没跑）。
    ///
    /// 跑法见 `scripts/host-tests.ps1`。
    #[test]
    #[ignore = "宿主层：需要真实 JVM（scripts/host-tests.ps1，或 DBMIND_TEST_AGENTS=1 cargo test -p dbmind-core -- --ignored）"]
    fn 宿主层_四个宿主都能起来并握手() {
        assert!(
            agent_hosts_visible(),
            "这一层要求真实宿主：请设 {AGENTS_TEST_ENV}=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        for spec in AgentHostSpec::all() {
            let id = spec.id.to_string();
            let generic = spec.generic;
            let declared = spec.agent_keys;
            let availability = availability_for(&spec);
            assert!(
                availability.ready,
                "宿主 {id} 应当可用，否则这一层没在验证任何东西：{:?} —— 先跑 scripts/build-agents.ps1",
                availability.reason
            );

            let host = AgentHost::new(spec);
            // 起进程 + 握手：这一步过了才叫「宿主可用」
            host.handshake()
                .unwrap_or_else(|e| panic!("宿主 {id} 握手失败：{}（{}）", e.message, e.code_str()));

            // 握完手要真的认自己该认的 key
            if generic {
                assert!(
                    host.supports("postgresql").unwrap_or(false),
                    "通用宿主（JDBC）应当承载任意 JDBC 类型"
                );
            } else {
                assert!(!declared.is_empty(), "专属宿主 {id} 应当声明自己承载哪一类");
                for key in declared.iter().copied() {
                    assert!(
                        host.supports(key).unwrap_or(false),
                        "宿主 {id} 不认自己声明的 agentKey={key}"
                    );
                }
            }

            // 起了就要收得掉：这一层同时验证「不留孤儿宿主」
            host.shutdown();
        }
    }

    /// 单测**默认不加载真实宿主**（见 `agent_hosts_visible`）—— 这条测试既是守卫，
    /// 也是那个开关的说明书。
    #[test]
    fn 测试构建默认不加载真实宿主() {
        if agent_hosts_visible() {
            // 显式打开了开关：此时**应当**真能看到宿主包，否则说明开关没生效
            // 或宿主的 jar 没构建 —— 那要立刻说出来，而不是让别的测试去猜。
            assert!(
                AgentHostSpec::all()
                    .iter()
                    .any(|spec| resolve_agent_jar(spec).is_some()),
                "设了 {AGENTS_TEST_ENV} 却一个宿主包都解析不出来：开关没生效，或宿主未构建（先跑 scripts/build-agents.ps1）"
            );
            return;
        }

        for spec in AgentHostSpec::all() {
            assert!(
                resolve_agent_jar(&spec).is_none(),
                "{} 在单测里默认不该解析出宿主包（那会起 JVM）",
                spec.id
            );
            let availability = availability_for(&spec);
            assert!(!availability.ready, "{} 默认不该就绪", spec.id);
            let reason = availability.reason.unwrap_or_default();
            assert!(
                reason.contains(AGENTS_TEST_ENV),
                "{} 的就绪原因要告诉用户怎么打开真实宿主：{reason}",
                spec.id
            );
        }

        // 静态的「有没有宿主能承载」看的是**声明**，不受这个开关影响：
        // 类型目录仍应显示这些类型已接入（否则开关会把功能显示成未接入）。
        assert!(has_host_for(ConnectionKind::Postgresql));
        assert!(has_host_for(ConnectionKind::Mongodb));
        assert!(has_host_for(ConnectionKind::Redis));
    }

    /// 驱动下载地址：坐标 → Maven Central 地址。三个壳共用这一个拼法。
    #[test]
    fn 驱动坐标能拼出_maven_地址() {
        assert_eq!(
            driver_artifact_url("com.h2database:h2:2.3.232").unwrap(),
            "https://repo.maven.apache.org/maven2/com/h2database/h2/2.3.232/h2-2.3.232.jar"
        );
        // 组名里的点要变成路径分隔符
        assert_eq!(
            driver_artifact_url("org.postgresql:postgresql:42.7.4").unwrap(),
            "https://repo.maven.apache.org/maven2/org/postgresql/postgresql/42.7.4/postgresql-42.7.4.jar"
        );
        assert_eq!(
            driver_jar_name("com.h2database:h2:2.3.232").unwrap(),
            "h2-2.3.232.jar"
        );
    }

    #[test]
    fn 坐标格式不对要明确报错() {
        // 注意 `a:b:c:d` 是**合法**的（第四段是 classifier，ClickHouse 用它）；
        // 五段才是格式错
        for bad in ["h2", "a:b", "a:b:c:d:e", ":h2:1", "a::1", ""] {
            let err = driver_artifact_url(bad).unwrap_err();
            assert!(
                err.message.contains("group:artifact:version"),
                "「{bad}」的报错要说清格式：{}",
                err.message
            );
        }
    }

    /// classifier 要真的进 URL 与文件名（ClickHouse 的 `all` 靠它拿到带依赖的包）。
    #[test]
    fn classifier_进地址与文件名() {
        assert_eq!(
            driver_artifact_url("com.clickhouse:clickhouse-jdbc:0.7.2:all").unwrap(),
            "https://repo.maven.apache.org/maven2/com/clickhouse/clickhouse-jdbc/0.7.2/clickhouse-jdbc-0.7.2-all.jar"
        );
        assert_eq!(
            driver_jar_name("com.clickhouse:clickhouse-jdbc:0.7.2:all").unwrap(),
            "clickhouse-jdbc-0.7.2-all.jar"
        );
    }

    /// 上传驱动的落点：**必须**挡掉路径穿越与非 jar —— 这个名字来自 HTTP 请求。
    #[test]
    fn 上传驱动文件名要挡住路径与非_jar() {
        let ok = driver_upload_target("h2", "h2-2.3.232.jar").unwrap();
        assert!(
            ok.ends_with(Path::new("h2").join("h2-2.3.232.jar")),
            "落点应当在 <home>/drivers/h2/ 下：{}",
            ok.display()
        );

        for bad in [
            "../evil.jar",
            "sub/dir.jar",
            "sub\\dir.jar",
            "..",
            "jdbc.txt",
            "",
            "   ",
        ] {
            assert!(driver_upload_target("h2", bad).is_err(), "「{bad}」不该被接受");
        }
        // 大写的扩展名照收（Windows 上很常见）
        assert!(driver_upload_target("h2", "H2.JAR").is_ok());
    }

    /// 空文件不算驱动：它既不是「装上了」，也不该在磁盘上留一个 0 字节的假驱动。
    #[test]
    fn 空文件不算驱动() {
        assert!(install_driver_jar("h2", "h2-2.3.232.jar", &[]).is_err());
    }
}
