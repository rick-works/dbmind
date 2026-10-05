//! agent 驱动的单元测试（**不启动真实 agent 进程**的部分）。
//!
//! 需要真实宿主与真实数据库的验证放在端到端脚本里：
//! - `scripts/smoke.ps1` 的 agent 段落（H2 / DuckDB，真 JVM + 真驱动）
//! - 库本身的行为（连接形状、协议分派、目录补齐）在这里用纯逻辑覆盖。

use super::agent_driver::*;
use super::session_pool::SessionBudget;
use crate::agent::{AgentHost, AgentHostSpec};
use crate::types::ConnectionConfig;
use crate::{ConnectionKind, RuntimeProtocol};
use std::sync::Arc;

fn temp_base(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("dbmind-agent-{tag}-{}", std::process::id()))
}

/// 默认配额（不限制）。
fn driver(kind: ConnectionKind, spec: AgentHostSpec) -> AgentDriver {
    driver_with_cap(kind, spec, 0)
}

/// 带全局配额的驱动（0 = 不限制）。
fn driver_with_cap(kind: ConnectionKind, spec: AgentHostSpec, cap: usize) -> AgentDriver {
    AgentDriver::new(
        kind,
        Arc::new(AgentHost::new(spec)),
        Arc::new(SessionBudget::new(cap)),
    )
}

#[test]
fn 文件型连接会补齐父目录() {
    let base = temp_base("dir");
    let _ = std::fs::remove_dir_all(&base);
    let file = base.join("nested").join("demo");
    let config =
        ConnectionConfig::new("h2", ConnectionKind::H2).with_file(file.to_string_lossy().to_string());

    ensure_file_parent(&config).expect("应能创建父目录");
    assert!(file.parent().unwrap().is_dir(), "父目录应被创建");
    // 幂等：重复调用不应报错
    ensure_file_parent(&config).expect("重复调用应成功");

    let _ = std::fs::remove_dir_all(&base);
}

/// 元数据会话的作用域由类型声明决定。
///
/// 两种情况必须与数据会话**同一条物理连接**：
/// - `metadataConnectionScoped`（如 SQL Server 的 `#temp`）：对象是**会话级**的，
///   换了会话就看不到；
/// - `singleConnectionPool`（嵌入式/文件引擎）：那条声明就是在说「只允许一条物理连接」，
///   再开一条可能落到**另一个引擎实例**上（DuckDB 这类会对数据文件加锁）。
///
/// 其余类型用一条独立的只读会话 —— 浏览结构不该要求写权限。
#[test]
fn 元数据会话按类型声明决定作用域() {
    let config = ConnectionConfig::new("sv", ConnectionKind::Sqlserver).with_host("127.0.0.1");

    // SQL Server：与数据会话同作用域（连接只读时也用只读会话）
    let scoped = driver(ConnectionKind::Sqlserver, AgentHostSpec::jdbc());
    assert!(ConnectionKind::Sqlserver.metadata_connection_scoped());
    assert_eq!(
        scoped.metadata_session_key(&config, false).0,
        scoped.session_key(&config, false)
    );
    assert_eq!(
        scoped.metadata_session_key(&config, true).0,
        scoped.session_key(&config, true)
    );

    // 单连接类型（H2 `:memory:`）：也必须搭同一条，否则看到的是另一个库
    let single = driver(ConnectionKind::H2, AgentHostSpec::jdbc());
    assert!(ConnectionKind::H2.single_connection_pool());
    let (lane, lane_read_only) = single.metadata_session_key(&config, false);
    assert_eq!(lane, single.session_key(&config, false));
    assert!(
        !lane_read_only,
        "搭同一条会话时，只读性必须跟数据会话一致（否则会去开第二条）"
    );

    // 其余 JDBC 类型：独立的只读会话
    let plain = driver(ConnectionKind::Postgresql, AgentHostSpec::jdbc());
    assert!(!ConnectionKind::Postgresql.metadata_connection_scoped());
    assert!(!ConnectionKind::Postgresql.single_connection_pool());
    let (key, read_only) = plain.metadata_session_key(&config, false);
    assert!(read_only, "非作用域类型应用只读会话");
    assert_eq!(key, plain.session_key(&config, true));
    assert_ne!(
        key,
        plain.session_key(&config, false),
        "不该与数据会话混用同一条连接"
    );
}

/// 池化上限由**类型声明**决定（YAML 的 `maxConnections`；单连接类型恒为 1）。
#[test]
fn 会话池上限按类型声明决定() {
    for kind in ConnectionKind::ALL.iter().copied() {
        let config = ConnectionConfig::new("c", kind).with_host("127.0.0.1");
        let limit = driver(kind, AgentHostSpec::jdbc()).lane_limit(&config, None);
        assert_eq!(
            limit,
            kind.max_connections(),
            "{} 的池上限应等于类型声明",
            kind.key()
        );
        if kind.single_connection_pool() {
            assert_eq!(
                limit,
                1,
                "{} 声明了 singleConnectionPool，上限必须是 1（否则那条声明就是空话）",
                kind.key()
            );
        }
        assert!(limit >= 1, "{} 的上限不能是 0", kind.key());
    }
    // 至少有一个类型是「可多连接」的，否则这个特性等于没写
    assert!(
        ConnectionKind::Postgresql.max_connections() > 1,
        "应当存在能并发开多条连接的类型"
    );
}

/// 三层上限：类型声明 → 连接级覆盖 → 全局配额，**取最小**。
#[test]
fn 并发上限三层取最小() {
    let kind = ConnectionKind::Postgresql;
    let base = ConnectionConfig::new("pg", kind).with_host("127.0.0.1");
    let plain = driver(kind, AgentHostSpec::jdbc());
    let declared = kind.max_connections();

    // 1) 没有覆盖、没有配额 ⇒ 类型声明
    assert_eq!(plain.lane_limit(&base, None), declared);

    // 2) 连接级覆盖：调大、调小都听它的
    assert_eq!(
        plain.lane_limit(&base.clone().with_max_connections(Some(2)), None),
        2
    );
    assert_eq!(
        plain.lane_limit(&base.clone().with_max_connections(Some(16)), None),
        16
    );
    // 非法覆盖（0 / 负数）当作没写，不当作「0 条连接」
    assert_eq!(
        plain.lane_limit(&base.clone().with_max_connections(Some(0)), None),
        declared
    );

    // 3) 全局配额把前两层夹住（一条连接不该自己就超掉全局预算）
    let capped = driver_with_cap(kind, AgentHostSpec::jdbc(), 2);
    assert_eq!(capped.lane_limit(&base, None), 2);
    assert_eq!(
        capped.lane_limit(&base.clone().with_max_connections(Some(16)), None),
        2,
        "覆盖也不能突破全局配额"
    );

    // 4) 亲和泳道恒为 1（那条会话是某个页面的，语句要按顺序落上去）
    assert_eq!(capped.lane_limit(&base, Some("ui:query")), 1);
    assert_eq!(plain.lane_limit(&base, Some("ui:query")), 1);

    // 5) 单连接类型：覆盖只能收窄，不能突破类型声明
    //（对它开第二条连接 = 另一个世界：嵌入式引擎会看到空库或加锁失败）
    let h2 = driver_with_cap(ConnectionKind::H2, AgentHostSpec::jdbc(), 0);
    let h2_config = ConnectionConfig::new("h2", ConnectionKind::H2).with_host("127.0.0.1");
    assert!(ConnectionKind::H2.single_connection_pool());
    assert_eq!(h2.lane_limit(&h2_config, None), 1);
    assert_eq!(
        h2.lane_limit(&h2_config.clone().with_max_connections(Some(8)), None),
        1,
        "单连接类型的连接级覆盖不能把它变成多连接"
    );
}

/// 会话亲和：一个键一条会话；单连接类型忽略亲和（它们只有一条物理连接可给）。
#[test]
fn 会话亲和按页面分泳道() {
    let kind = ConnectionKind::Postgresql;
    let config = ConnectionConfig::new("pg", kind).with_host("127.0.0.1");
    let pg = driver(kind, AgentHostSpec::jdbc());

    let plain = pg.session_key(&config, false);
    let page_a = pg.lane_key(&config, false, Some("ui:query"));
    let page_b = pg.lane_key(&config, false, Some("ui:schema"));

    assert_ne!(page_a, plain, "带亲和的查询不该落在公共泳道上");
    assert_ne!(page_a, page_b, "不同页面必须是不同泳道（否则会共用一条会话）");
    assert_eq!(
        page_a,
        pg.lane_key(&config, false, Some("ui:query")),
        "同一个页面必须稳定落在同一条泳道上"
    );
    // 空键等于没给（界面没传时不改变行为）
    assert_eq!(pg.lane_key(&config, false, Some("")), plain);

    // 泳道键会拼进 sessionId 交给宿主，键里的分隔符要被换掉，避免拼出重复的泳道
    let tricky = pg.lane_key(&config, false, Some("a|s=b"));
    let tricky_key = tricky
        .strip_prefix(&format!("{plain}|s="))
        .expect("应带上亲和后缀");
    assert!(!tricky_key.contains('|'), "键里的分隔符应被换掉：{tricky}");
    assert_ne!(
        tricky,
        pg.lane_key(&config, false, Some("b")),
        "去掉分隔符后不能与另一个键撞上"
    );

    // 单连接类型：忽略亲和（H2 / SQLite 这类只有一条物理连接）
    let h2 = driver(ConnectionKind::H2, AgentHostSpec::jdbc());
    let h2_config = ConnectionConfig::new("h2", ConnectionKind::H2).with_host("127.0.0.1");
    assert!(ConnectionKind::H2.single_connection_pool());
    assert_eq!(
        h2.lane_key(&h2_config, false, Some("ui:query")),
        h2.session_key(&h2_config, false),
        "单连接类型必须忽略亲和，否则就开了第二条连接"
    );
}

/// 宿主回执里「被配额淘汰的会话」要被认出来（内核据此清缓存、下次重连）。
#[test]
fn 能读出宿主淘汰的会话() {
    let value = serde_json::json!({
        "serverVersion": "2.3.232",
        "evictedSessions": ["a#0", "b#1"]
    });
    assert_eq!(
        evicted_sessions(&value),
        vec!["a#0".to_string(), "b#1".to_string()]
    );

    // 没淘汰（老宿主 / 未超配额）⇒ 空列表，不该误伤任何会话
    assert!(evicted_sessions(&serde_json::json!({ "serverVersion": "x" })).is_empty());
    assert!(
        evicted_sessions(&serde_json::json!({ "evictedSessions": [] })).is_empty(),
        "空数组等于没淘汰"
    );
}

/// 连接级覆盖存在 `extra.maxConnections`（不新增列：老库不会自动加列）。
#[test]
fn 连接级覆盖的读写与容错() {
    let base = ConnectionConfig::new("pg", ConnectionKind::Postgresql);
    assert_eq!(base.max_connections_override(), None);

    let set = base.clone().with_max_connections(Some(6));
    assert_eq!(set.max_connections_override(), Some(6));
    assert_eq!(
        base.clone().with_max_connections(None).max_connections_override(),
        None
    );

    // 别的 extra 键不能被覆盖时弄丢
    let with_other = ConnectionConfig {
        extra: Some(serde_json::json!({ "whatever": "keep-me" })),
        ..base.clone()
    };
    let both = with_other.with_max_connections(Some(3));
    assert_eq!(both.extra.as_ref().unwrap()["whatever"], "keep-me");
    assert_eq!(both.max_connections_override(), Some(3));

    // 坏值当作没写（不当作 0 条连接）
    let broken = ConnectionConfig {
        extra: Some(serde_json::json!({ "maxConnections": "四" })),
        ..base.clone()
    };
    assert_eq!(broken.max_connections_override(), None);
    let zero = ConnectionConfig {
        extra: Some(serde_json::json!({ "maxConnections": 0 })),
        ..base
    };
    assert_eq!(zero.max_connections_override(), None);
}

#[test]
fn 网络型连接不需要建目录() {
    let config = ConnectionConfig::new("pg", ConnectionKind::Postgresql).with_host("127.0.0.1");
    ensure_file_parent(&config).expect("网络型连接应直接通过");
}

#[test]
fn 每个_jdbc_类型都有可渲染的_url() {
    for kind in ConnectionKind::ALL.iter().copied().filter(|k| k.is_jdbc()) {
        let template = kind.jdbc_url_template().expect("JDBC 类型必须有 urlTemplate");
        assert!(template.contains('{'), "{} 的模板应有占位符", kind.key());
        assert!(
            kind.jdbc_driver_class().is_some(),
            "{} 缺少 driverClass",
            kind.key()
        );
        let artifact = kind.jdbc_artifact().expect("JDBC 类型必须有 artifact");
        // 与驱动商店的下载逻辑保持一致：三段坐标，或四段（末段是 classifier）。
        // ClickHouse 必须是四段的 `...:0.7.2:all` —— 默认瘦包缺 slf4j，
        // 装载时以 NoClassDefFoundError 失败，而界面还显示已就绪。
        let parts = artifact.split(':').count();
        assert!(
            parts == 3 || parts == 4,
            "{} 的坐标不合法：{artifact}",
            kind.key()
        );
        // 而且每个坐标都要真的能拼成 jar 地址（包括 YAML 里声明的额外依赖）
        for coordinate in kind.jdbc_artifacts() {
            let url = crate::agent::driver_artifact_url(coordinate).unwrap_or_else(|err| {
                panic!("{} 的坐标无法拼成地址：{coordinate}（{}）", kind.key(), err.message)
            });
            assert!(url.ends_with(".jar"), "{} 的地址不像 jar：{url}", kind.key());
        }
    }
}

#[test]
fn 非_jdbc_类型不应有_jdbc_元数据() {
    for kind in ConnectionKind::ALL.iter().copied().filter(|k| !k.is_jdbc()) {
        assert!(kind.jdbc_driver_class().is_none());
        assert!(kind.jdbc_url_template().is_none());
    }
}

/// JDBC 与原生协议的 connect 参数形状不同，这里把两种形状钉住。
#[test]
fn jdbc_类型传驱动类与_url() {
    let config = ConnectionConfig::new("pg", ConnectionKind::Postgresql)
        .with_host("10.0.0.5")
        .with_database("app");
    let params = driver(ConnectionKind::Postgresql, AgentHostSpec::jdbc())
        .connect_params("s1", &config, false)
        .expect("应能组装参数");

    assert_eq!(params["driverClass"], "org.postgresql.Driver");
    assert_eq!(params["url"], "jdbc:postgresql://10.0.0.5:5432/app");
    assert_eq!(params["protocol"], "sql");
    assert_eq!(params["agentKey"], "postgresql");
    // JDBC 的驱动由驱动商店提供 ⇒ 必须带 jar 列表字段
    assert!(params.contains_key("driverJars"));
    assert!(!params.contains_key("host"), "JDBC 不应把原始字段也传过去");
}

/// JDBC 的连接请求必须带**账号、口令与驱动参数**。
///
/// 这三样曾经都没发：JDBC 的 URL 模板里不含账号（各家驱动都从连接属性取），
/// 而内核只发了 driverClass/url/driverJars ⇒ **所有需要认证的 JDBC 类型都登不上去**。
/// 宿主层测试一直没暴露它，因为它只覆盖 H2 文件库 —— 那玩意儿不需要认证。
///
/// 也要说清上面那条 `jdbc_类型传驱动类与_url` 的教训：它当时只钉了 URL 与 jar 列表，
/// 于是**错的形状被钉住了**（测试全绿，产品不可用）。所以这条把三样都点名。
#[test]
fn jdbc_连接请求要带账号口令与驱动参数() {
    let mut config = ConnectionConfig::new("mssql", ConnectionKind::Sqlserver)
        .with_host("10.0.0.9")
        .with_port(1433)
        .with_database("app")
        .with_credentials("sa", "s3cret");
    config.extra = Some(serde_json::json!({
        "params": { "trustServerCertificate": "true", "encrypt": "false" }
    }));

    let params = driver(ConnectionKind::Sqlserver, AgentHostSpec::jdbc())
        .connect_params("s1", &config, false)
        .expect("应能组装参数");

    assert_eq!(params["username"], "sa", "JDBC 必须带账号（URL 模板里没有）");
    assert_eq!(params["password"], "s3cret", "JDBC 必须带口令");
    assert_eq!(
        params["params"]["trustServerCertificate"], "true",
        "驱动参数要透传（自签证书的 SQL Server 就靠它）"
    );

    // 没填参数时不要塞一个空的 params —— 宿主侧会多绕一圈、日志里也吵
    let bare = ConnectionConfig::new("mssql2", ConnectionKind::Sqlserver)
        .with_host("10.0.0.9")
        .with_port(1433)
        .with_database("app");
    let p2 = driver(ConnectionKind::Sqlserver, AgentHostSpec::jdbc())
        .connect_params("s1", &bare, false)
        .expect("应能组装参数");
    assert!(!p2.contains_key("params"), "没填参数就不该出现 params 字段");
    assert!(p2["username"].is_null(), "没填账号时为 null（宿主会跳过它）");
}

#[test]
fn mongodb_类型传规范化连接字段而不传_url() {
    let config = ConnectionConfig::new("mongo", ConnectionKind::Mongodb)
        .with_host("10.0.0.8")
        .with_database("app")
        .with_credentials("root", "secret");
    let params = driver(ConnectionKind::Mongodb, AgentHostSpec::mongodb())
        .connect_params("s1", &config, true)
        .expect("应能组装参数");

    assert_eq!(params["protocol"], "mongodb");
    assert_eq!(params["host"], "10.0.0.8");
    assert_eq!(params["port"], 27017, "应补上 YAML 里的默认端口");
    assert_eq!(params["database"], "app");
    assert_eq!(params["username"], "root");
    assert_eq!(params["readOnly"], true);
    // 原生协议的连接串由宿主自己拼：不该出现 JDBC 的字段
    assert!(!params.contains_key("url"));
    assert!(!params.contains_key("driverClass"));
    assert!(!params.contains_key("driverJars"));
}

#[test]
fn 协议与宿主声明保持一致() {
    assert_eq!(ConnectionKind::Mongodb.protocol(), RuntimeProtocol::Mongodb);
    assert_eq!(ConnectionKind::Postgresql.protocol(), RuntimeProtocol::Sql);
    assert_eq!(ConnectionKind::Sqlite.protocol(), RuntimeProtocol::Sql);
    // MongoDB 必须由专属宿主承载，且宿主声明里要认领它
    assert!(AgentHostSpec::mongodb().declares(ConnectionKind::Mongodb.agent_key().unwrap()));
    assert!(!AgentHostSpec::jdbc().declares("mongodb"));
}

#[test]
fn 会话键区分只读与可写() {
    let config = ConnectionConfig::new("m", ConnectionKind::Mongodb).with_host("h");
    let mongo = driver(ConnectionKind::Mongodb, AgentHostSpec::mongodb());
    let read = mongo.session_key(&config, true);
    let write = mongo.session_key(&config, false);
    assert_ne!(read, write, "只读会话与可写会话不能共用一个键");
}
