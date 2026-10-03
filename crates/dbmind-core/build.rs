//! 连接类型的**编译期代码生成**。
//!
//! 单一真源是 `plugins/connection-types/*.yaml`。本脚本在构建时把它们编译成
//! 1) `ConnectionKind` 枚举 + 全部查询方法（`OUT_DIR/connection_types.rs`）
//! 2) 前端可直接消费的 manifest（`assets/database-drivers.manifest.json`）
//!
//! 于是「新增一种数据库」= 新增一个 YAML，内核代码零改动；而 YAML 写错会在
//! **编译期**直接失败（而不是运行期才炸）。

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- YAML 模型

#[derive(serde::Deserialize)]
struct TypeYaml {
    #[serde(rename = "schemaVersion")]
    schema_version: i32,
    order: i32,
    #[serde(rename = "dbType")]
    db_type: String,
    #[serde(rename = "rustVariant")]
    rust_variant: String,
    label: String,
    #[serde(default = "default_dialect")]
    dialect: String,
    #[serde(rename = "runtimeMode")]
    runtime_mode: String,
    /// 语句协议（默认 sql）：决定安全闸门用哪套「只读判定」规则。
    /// 只有非 SQL 数据库才需要写（如 MongoDB）。
    #[serde(default)]
    protocol: Option<String>,
    #[serde(rename = "mcpMode", default)]
    mcp_mode: Option<String>,
    #[serde(rename = "agentKey", default)]
    agent_key: Option<String>,
    #[serde(rename = "driverStoreVisible", default)]
    driver_store_visible: bool,
    #[serde(rename = "driverStoreOrder", default)]
    driver_store_order: i32,
    #[serde(rename = "singleConnectionPool", default)]
    single_connection_pool: bool,
    #[serde(rename = "metadataConnectionScoped", default)]
    metadata_connection_scoped: bool,
    /// 并发会话上限（`maxConnections`）。省略时用 `DEFAULT_MAX_CONNECTIONS`；
    /// 声明了 `singleConnectionPool` 的类型**只能是 1**（写了别的值会在编译期报错）。
    #[serde(rename = "maxConnections", default)]
    max_connections: Option<usize>,
    #[serde(rename = "skipTcpProbe", default)]
    skip_tcp_probe: bool,
    #[serde(rename = "localFile", default)]
    local_file: bool,
    #[serde(rename = "defaultPort", default)]
    default_port: Option<u16>,
    /// agent 运行时的 JDBC 元数据。放在 YAML 而不是写进 Java 宿主，
    /// 是为了让「怎么连这个库」只有一个出处：宿主保持通用，类型保持数据。
    #[serde(default)]
    jdbc: Option<JdbcYaml>,
    #[serde(default)]
    traits: TraitsYaml,
    #[serde(rename = "supportLevel", default = "default_support")]
    support_level: String,
    #[serde(default)]
    capabilities: CapabilitiesYaml,
}

fn default_dialect() -> String {
    "Generic".to_string()
}

fn default_support() -> String {
    "operate".to_string()
}

/// 非单连接类型的并发会话默认上限（YAML 省略 `maxConnections` 时用它）。
///
/// 取 4 的理由：够覆盖「几个标签页 + 结构浏览」这种真实用法，又不至于让一条连接在
/// 数据库侧悄悄占掉一堆连接 —— 很多 JDBC 授权是按连接数算的。
const DEFAULT_MAX_CONNECTIONS: usize = 4;

/// 生效的并发上限：单连接类型恒为 1；其余取声明值（省略则用默认）。
fn effective_max_connections(t: &TypeYaml) -> usize {
    if t.single_connection_pool {
        1
    } else {
        t.max_connections.unwrap_or(DEFAULT_MAX_CONNECTIONS)
    }
}

#[derive(serde::Deserialize, Default)]
struct TraitsYaml {
    #[serde(rename = "singleDatabase", default)]
    single_database: bool,
    #[serde(rename = "schemaAware", default)]
    schema_aware: bool,
}

#[derive(serde::Deserialize)]
struct JdbcYaml {
    #[serde(rename = "driverClass")]
    driver_class: String,
    /// 支持占位符：{host} {port} {database} {filePath} {username}
    #[serde(rename = "urlTemplate")]
    url_template: String,
    /// Maven 坐标 `group:artifact:version[:classifier]`，供驱动商店下载与就绪检查
    artifact: String,
    /// **额外**依赖（同目录 jar 会一起进宿主 classpath，见 agent 的 driver_dirs）。
    ///
    /// 存在的理由：有些驱动的"完整包"也不含全部依赖 —— ClickHouse 的 `all` 包把
    /// slf4j 当 provided，而驱动类静态块第一句就是 LoggerFactory.getLogger，
    /// 少了它装载即失败（实测 ClassNotFoundException: org/slf4j/LoggerFactory），
    /// 界面却因为"目录里有 jar"显示已就绪。
    #[serde(default)]
    artifacts: Vec<String>,
}

impl JdbcYaml {
    /// 该类型需要的全部坐标：`artifact` 打头，后接 `artifacts` 里去重后的额外项。
    fn all_artifacts(&self) -> Vec<&str> {
        let mut out: Vec<&str> = vec![self.artifact.trim()];
        for extra in &self.artifacts {
            let trimmed = extra.trim();
            if !trimmed.is_empty() && !out.contains(&trimmed) {
                out.push(trimmed);
            }
        }
        out
    }
}

#[derive(serde::Deserialize, Default)]
struct CapabilitiesYaml {
    #[serde(rename = "queryExecution", default)]
    query_execution: bool,
    #[serde(rename = "metadataBrowse", default)]
    metadata_browse: bool,
    #[serde(rename = "objectBrowser", default)]
    object_browser: bool,
    #[serde(rename = "objectSource", default)]
    object_source: bool,
    #[serde(rename = "schemaSearch", default)]
    schema_search: bool,
    #[serde(default)]
    diagram: bool,
    #[serde(rename = "tableDataEdit", default)]
    table_data_edit: bool,
    #[serde(rename = "tableStructureEdit", default)]
    table_structure_edit: bool,
    #[serde(rename = "tableImport", default)]
    table_import: bool,
    #[serde(rename = "dataTransfer", default)]
    data_transfer: bool,
    #[serde(rename = "sqlFileExecution", default)]
    sql_file_execution: bool,
    #[serde(rename = "databaseCreate", default)]
    database_create: bool,
    #[serde(rename = "fieldLineage", default)]
    field_lineage: bool,
    #[serde(rename = "sqlExplain", default)]
    sql_explain: bool,
    #[serde(rename = "userAdmin", default)]
    user_admin: bool,
    #[serde(rename = "driverManagement", default)]
    driver_management: bool,
}

/// 能力清单（YAML key ↔ Rust 字段 ↔ manifest key），新增能力只改这一处。
const CAPABILITIES: &[(&str, &str)] = &[
    ("queryExecution", "query_execution"),
    ("metadataBrowse", "metadata_browse"),
    ("objectBrowser", "object_browser"),
    ("objectSource", "object_source"),
    ("schemaSearch", "schema_search"),
    ("diagram", "diagram"),
    ("tableDataEdit", "table_data_edit"),
    ("tableStructureEdit", "table_structure_edit"),
    ("tableImport", "table_import"),
    ("dataTransfer", "data_transfer"),
    ("sqlFileExecution", "sql_file_execution"),
    ("databaseCreate", "database_create"),
    ("fieldLineage", "field_lineage"),
    ("sqlExplain", "sql_explain"),
    ("userAdmin", "user_admin"),
    ("driverManagement", "driver_management"),
];

fn capability_value(c: &CapabilitiesYaml, key: &str) -> bool {
    match key {
        "queryExecution" => c.query_execution,
        "metadataBrowse" => c.metadata_browse,
        "objectBrowser" => c.object_browser,
        "objectSource" => c.object_source,
        "schemaSearch" => c.schema_search,
        "diagram" => c.diagram,
        "tableDataEdit" => c.table_data_edit,
        "tableStructureEdit" => c.table_structure_edit,
        "tableImport" => c.table_import,
        "dataTransfer" => c.data_transfer,
        "sqlFileExecution" => c.sql_file_execution,
        "databaseCreate" => c.database_create,
        "fieldLineage" => c.field_lineage,
        "sqlExplain" => c.sql_explain,
        "userAdmin" => c.user_admin,
        "driverManagement" => c.driver_management,
        other => panic!("build.rs 能力清单缺少 {other}（CAPABILITIES 与 CapabilitiesYaml 不同步）"),
    }
}

fn runtime_variant(mode: &str) -> Result<&'static str, String> {
    match mode {
        "native" => Ok("RuntimeMode::Native"),
        "agent" => Ok("RuntimeMode::Agent"),
        "external" => Ok("RuntimeMode::External"),
        "file" => Ok("RuntimeMode::File"),
        other => Err(format!("未知 runtimeMode: {other}")),
    }
}

/// 协议 -> 生成代码里的变体。默认 `sql`：绝大多数类型都属于 SQL 系。
fn protocol_variant(protocol: Option<&str>) -> Result<&'static str, String> {
    match protocol.unwrap_or("sql") {
        "sql" => Ok("RuntimeProtocol::Sql"),
        "mongodb" => Ok("RuntimeProtocol::Mongodb"),
        "redis" => Ok("RuntimeProtocol::Redis"),
        "elasticsearch" => Ok("RuntimeProtocol::Elasticsearch"),
        other => Err(format!(
            "未知 protocol: {other}（可选 sql / mongodb / redis / elasticsearch）"
        )),
    }
}

fn mcp_variant(mode: Option<&str>) -> Result<&'static str, String> {
    match mode.unwrap_or("none") {
        "bridge" => Ok("McpMode::Bridge"),
        "native" => Ok("McpMode::Native"),
        "none" => Ok("McpMode::None"),
        other => Err(format!("未知 mcpMode: {other}")),
    }
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn opt_str(v: Option<&str>) -> String {
    match v {
        Some(s) => format!("Some({s:?})"),
        None => "None".to_string(),
    }
}

// ---------------------------------------------------------------- 主流程

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let types_dir = manifest_dir.join("../../plugins/connection-types");
    println!("cargo:rerun-if-changed={}", types_dir.display());

    let mut types = load_types(&types_dir).unwrap_or_else(|e| panic!("连接类型 YAML 校验失败：{e}"));
    validate(&types).unwrap_or_else(|e| panic!("连接类型 YAML 校验失败：{e}"));
    types.sort_by_key(|t| t.order);

    let code = generate_rust(&types);
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("connection_types.rs");
    fs::write(&out, code).expect("写 connection_types.rs 失败");

    let manifest = generate_manifest(&types);
    let assets = manifest_dir.join("assets");
    fs::create_dir_all(&assets).expect("创建 assets 目录失败");
    let manifest_path = assets.join("database-drivers.manifest.json");
    let rendered = serde_json::to_string_pretty(&manifest).expect("manifest 序列化失败");
    // 只在内容变化时写入，避免每次构建都改文件时间戳
    let changed = fs::read_to_string(&manifest_path)
        .map(|old| old != rendered)
        .unwrap_or(true);
    if changed {
        fs::write(&manifest_path, format!("{rendered}\n")).expect("写 manifest 失败");
    }

    // 生成情况不再打 cargo:warning（每次构建刷屏），需要排查时看 OUT_DIR 产物
    println!("cargo:rerun-if-changed=connectors");
}

fn load_types(dir: &Path) -> Result<Vec<TypeYaml>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("无法读取 {}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        match serde_yaml::from_str::<TypeYaml>(&text) {
            Ok(t) => out.push(t),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    if out.is_empty() {
        return Err(format!("{} 下没有找到任何连接类型 YAML", dir.display()));
    }
    Ok(out)
}

fn validate(types: &[TypeYaml]) -> Result<(), String> {
    let mut keys = std::collections::HashSet::new();
    let mut variants = std::collections::HashSet::new();
    for t in types {
        if t.schema_version != 1 {
            return Err(format!(
                "{}: 不支持的 schemaVersion {}",
                t.db_type, t.schema_version
            ));
        }
        if !is_ident(&t.rust_variant) {
            return Err(format!("{}: rustVariant 不是合法标识符", t.db_type));
        }
        if t.db_type != t.db_type.to_lowercase() || !is_ident(&t.db_type) {
            return Err(format!("{}: dbType 必须是 snake_case 标识符", t.db_type));
        }
        if !keys.insert(&t.db_type) {
            return Err(format!("dbType 重复：{}", t.db_type));
        }
        if !variants.insert(&t.rust_variant) {
            return Err(format!("rustVariant 重复：{}", t.rust_variant));
        }
        // 运行时一致性约束：这些能挡住最容易犯的配置错误
        runtime_variant(&t.runtime_mode).map_err(|e| format!("{}: {e}", t.db_type))?;
        protocol_variant(t.protocol.as_deref()).map_err(|e| format!("{}: {e}", t.db_type))?;
        mcp_variant(t.mcp_mode.as_deref()).map_err(|e| format!("{}: {e}", t.db_type))?;
        if t.runtime_mode == "agent" && t.agent_key.is_none() {
            return Err(format!("{}: runtimeMode=agent 必须给 agentKey", t.db_type));
        }
        if t.local_file && t.default_port.is_some() && !t.skip_tcp_probe {
            return Err(format!(
                "{}: 本地文件型不应做 TCP 探测（skipTcpProbe 应为 true）",
                t.db_type
            ));
        }
        // 并发上限：0 没有意义；而「只允许一条物理连接」与「要求 >1 条」是自相矛盾的声明
        match t.max_connections {
            Some(0) => return Err(format!("{}: maxConnections 必须 >= 1", t.db_type)),
            Some(n) if t.single_connection_pool && n != 1 => {
                return Err(format!(
                    "{}: 声明了 singleConnectionPool 就不能要求并发上限 > 1（maxConnections = {n}）\
                     —— 上限由声明派生为 1，YAML 里请省略或写 1",
                    t.db_type
                ));
            }
            _ => {}
        }
        if !t.local_file && t.default_port.is_none() {
            return Err(format!("{}: 网络型必须给 defaultPort", t.db_type));
        }
        // JDBC 元数据只对 agent 运行时成立；写错会在编译期暴露
        if let Some(jdbc) = &t.jdbc {
            if t.runtime_mode != "agent" {
                return Err(format!(
                    "{}: jdbc 元数据只适用于 runtimeMode=agent（当前 {}）",
                    t.db_type, t.runtime_mode
                ));
            }
            if jdbc.driver_class.trim().is_empty() || jdbc.url_template.trim().is_empty() {
                return Err(format!(
                    "{}: jdbc.driverClass / jdbc.urlTemplate 不能为空",
                    t.db_type
                ));
            }
            if !jdbc.url_template.contains('{') {
                return Err(format!(
                    "{}: jdbc.urlTemplate 至少需要一个占位符（如 {{host}}）",
                    t.db_type
                ));
            }
            // `group:artifact:version` 或 `group:artifact:version:classifier`：
            // 有些驱动只有带分类器的包才自带依赖（ClickHouse 的 `all`），
            // 用默认瘦包会以 NoClassDefFoundError 装载失败。校验与
            // `dbmind_core::agent::split_artifact` 保持同一套形状。
            for artifact in jdbc.all_artifacts() {
                let parts: Vec<&str> = artifact.split(':').collect();
                if !matches!(parts.len(), 3 | 4) || parts.iter().any(|p| p.trim().is_empty()) {
                    return Err(format!(
                        "{}: jdbc 坐标必须是 group:artifact:version[:classifier]（当前 {artifact}）",
                        t.db_type
                    ));
                }
            }
        }
    }
    Ok(())
}

fn generate_rust(types: &[TypeYaml]) -> String {
    let mut c = String::new();
    c.push_str(
        "// @generated by crates/dbmind-core/build.rs —— 请勿手改，改 plugins/connection-types/*.yaml\n\n",
    );

    // 枚举
    writeln!(c, "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]").unwrap();
    writeln!(c, "pub enum ConnectionKind {{").unwrap();
    for t in types {
        writeln!(c, "    {},", t.rust_variant).unwrap();
    }
    writeln!(c, "}}\n").unwrap();

    writeln!(c, "impl ConnectionKind {{").unwrap();
    let all = types
        .iter()
        .map(|t| format!("ConnectionKind::{}", t.rust_variant))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(c, "    pub const ALL: &'static [ConnectionKind] = &[{all}];\n").unwrap();

    // 单一真源表：每个变体一行，所有 getter 都由它派生
    writeln!(c, "    pub fn key(self) -> &'static str {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        writeln!(
            c,
            "            ConnectionKind::{} => {:?},",
            t.rust_variant, t.db_type
        )
        .unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    for (name, ty, expr) in [
        ("label", "&'static str", "label"),
        ("dialect", "&'static str", "dialect"),
        ("support_level", "&'static str", "support_level"),
    ] {
        writeln!(c, "    pub fn {name}(self) -> {ty} {{").unwrap();
        writeln!(c, "        match self {{").unwrap();
        for t in types {
            let v = match expr {
                "label" => t.label.as_str(),
                "dialect" => t.dialect.as_str(),
                _ => t.support_level.as_str(),
            };
            writeln!(c, "            ConnectionKind::{} => {v:?},", t.rust_variant).unwrap();
        }
        writeln!(c, "        }}\n    }}\n").unwrap();
    }

    for (name, ty, expr) in [
        ("order", "i32", "order"),
        ("driver_store_order", "i32", "driver_store_order"),
    ] {
        writeln!(c, "    pub fn {name}(self) -> {ty} {{").unwrap();
        writeln!(c, "        match self {{").unwrap();
        for t in types {
            let v = match expr {
                "order" => t.order,
                _ => t.driver_store_order,
            };
            writeln!(c, "            ConnectionKind::{} => {v},", t.rust_variant).unwrap();
        }
        writeln!(c, "        }}\n    }}\n").unwrap();
    }

    // 并发会话上限：**单连接类型恒为 1**（那条声明就是这个意思），其余取 YAML 的
    // `maxConnections`（省略则用默认值）。派生放在这里而不是运行期：这样
    // 「YAML 声明矛盾」是编译期错误，运行期只有一种真相。
    writeln!(
        c,
        "    /// 该类型允许的并发会话上限（YAML 的 `maxConnections`）。"
    )
    .unwrap();
    writeln!(c, "    ///").unwrap();
    writeln!(
        c,
        "    /// 声明 `singleConnectionPool` 的类型恒为 **1** —— 那条声明就是在说"
    )
    .unwrap();
    writeln!(
        c,
        "    /// 「只允许一条物理连接」，所以并发请求只能排队（见 drivers/session_pool.rs）。"
    )
    .unwrap();
    writeln!(c, "    pub fn max_connections(self) -> usize {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        let v = effective_max_connections(t);
        writeln!(c, "            ConnectionKind::{} => {v},", t.rust_variant).unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    for (name, ty, expr) in [
        ("local_file", "bool", "local_file"),
        ("skip_tcp_probe", "bool", "skip_tcp_probe"),
        ("single_connection_pool", "bool", "single_connection_pool"),
        ("metadata_connection_scoped", "bool", "metadata_connection_scoped"),
        ("driver_store_visible", "bool", "driver_store_visible"),
    ] {
        writeln!(c, "    pub fn {name}(self) -> {ty} {{").unwrap();
        writeln!(c, "        match self {{").unwrap();
        for t in types {
            let v = match expr {
                "local_file" => t.local_file,
                "skip_tcp_probe" => t.skip_tcp_probe,
                "single_connection_pool" => t.single_connection_pool,
                "metadata_connection_scoped" => t.metadata_connection_scoped,
                _ => t.driver_store_visible,
            };
            writeln!(c, "            ConnectionKind::{} => {v},", t.rust_variant).unwrap();
        }
        writeln!(c, "        }}\n    }}\n").unwrap();
    }

    writeln!(c, "    pub fn runtime_mode(self) -> RuntimeMode {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        let v = runtime_variant(&t.runtime_mode).expect("validated");
        writeln!(c, "            ConnectionKind::{} => {v},", t.rust_variant).unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    // 语句协议：安全闸门据此选择「只读判定」规则
    writeln!(c, "    /// 语句协议（YAML 的 `protocol`，默认 sql）。").unwrap();
    writeln!(c, "    pub fn protocol(self) -> RuntimeProtocol {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        let v = protocol_variant(t.protocol.as_deref()).expect("validated");
        writeln!(c, "            ConnectionKind::{} => {v},", t.rust_variant).unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    writeln!(c, "    pub fn mcp_mode(self) -> McpMode {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        let v = mcp_variant(t.mcp_mode.as_deref()).expect("validated");
        writeln!(c, "            ConnectionKind::{} => {v},", t.rust_variant).unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    writeln!(c, "    pub fn agent_key(self) -> Option<&'static str> {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        writeln!(
            c,
            "            ConnectionKind::{} => {},",
            t.rust_variant,
            opt_str(t.agent_key.as_deref())
        )
        .unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    writeln!(c, "    pub fn default_port(self) -> Option<u16> {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        let v = match t.default_port {
            Some(p) => format!("Some({p})"),
            None => "None".to_string(),
        };
        writeln!(c, "            ConnectionKind::{} => {v},", t.rust_variant).unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    writeln!(c, "    pub fn traits(self) -> TypeTraits {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        writeln!(
            c,
            "            ConnectionKind::{} => TypeTraits {{ single_database: {}, schema_aware: {} }},",
            t.rust_variant, t.traits.single_database, t.traits.schema_aware
        )
        .unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    writeln!(c, "    pub fn capabilities(self) -> Capabilities {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        let fields = CAPABILITIES
            .iter()
            .map(|(json_key, rust_field)| {
                format!("{rust_field}: {}", capability_value(&t.capabilities, json_key))
            })
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            c,
            "            ConnectionKind::{} => Capabilities {{ {fields} }},",
            t.rust_variant
        )
        .unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    writeln!(c, "    pub fn from_key(key: &str) -> Option<ConnectionKind> {{").unwrap();
    writeln!(c, "        match key {{").unwrap();
    for t in types {
        writeln!(
            c,
            "            {:?} => Some(ConnectionKind::{}),",
            t.db_type, t.rust_variant
        )
        .unwrap();
    }
    writeln!(c, "            _ => None,\n        }}\n    }}\n").unwrap();

    writeln!(c, "    pub fn rust_variant(self) -> &'static str {{").unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        writeln!(
            c,
            "            ConnectionKind::{} => {:?},",
            t.rust_variant, t.rust_variant
        )
        .unwrap();
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    // JDBC 元数据（agent 运行时用；非 JDBC 类型为 None）
    for (name, pick) in [
        ("jdbc_driver_class", "driver"),
        ("jdbc_url_template", "url"),
        ("jdbc_artifact", "artifact"),
    ] {
        writeln!(c, "    pub fn {name}(self) -> Option<&'static str> {{").unwrap();
        writeln!(c, "        match self {{").unwrap();
        for t in types {
            let value = t.jdbc.as_ref().map(|j| match pick {
                "driver" => j.driver_class.as_str(),
                "url" => j.url_template.as_str(),
                _ => j.artifact.as_str(),
            });
            writeln!(
                c,
                "            ConnectionKind::{} => {},",
                t.rust_variant,
                opt_str(value)
            )
            .unwrap();
        }
        writeln!(c, "        }}\n    }}\n").unwrap();
    }

    // 该类型需要的**全部**驱动坐标（含额外依赖）：就绪检查要"齐了才算就绪"，
    // 下载要逐个补齐 —— 只看第一个会让"缺 slf4j"这种状态显示成已就绪。
    writeln!(
        c,
        "    /// 该类型需要的全部驱动坐标（第一个是主驱动，其余是额外依赖）。"
    )
    .unwrap();
    writeln!(
        c,
        "    pub fn jdbc_artifacts(self) -> &'static [&'static str] {{"
    )
    .unwrap();
    writeln!(c, "        match self {{").unwrap();
    for t in types {
        match t.jdbc.as_ref() {
            Some(jdbc) => {
                let items: Vec<String> = jdbc
                    .all_artifacts()
                    .iter()
                    .map(|a| format!("{a:?}"))
                    .collect();
                writeln!(
                    c,
                    "            ConnectionKind::{} => &[{}],",
                    t.rust_variant,
                    items.join(", ")
                )
                .unwrap();
            }
            None => {
                writeln!(c, "            ConnectionKind::{} => &[],", t.rust_variant).unwrap();
            }
        }
    }
    writeln!(c, "        }}\n    }}\n").unwrap();

    writeln!(c, "    /// 是否由 JDBC agent 宿主承载。").unwrap();
    writeln!(c, "    pub fn is_jdbc(self) -> bool {{").unwrap();
    writeln!(c, "        self.jdbc_driver_class().is_some()").unwrap();
    writeln!(c, "    }}\n").unwrap();

    writeln!(c, "    pub fn requires_file(self) -> bool {{").unwrap();
    writeln!(c, "        self.local_file()").unwrap();
    writeln!(c, "    }}\n").unwrap();

    writeln!(c, "    pub fn requires_host(self) -> bool {{").unwrap();
    writeln!(c, "        !self.local_file()").unwrap();
    writeln!(c, "    }}\n").unwrap();

    // 该类型当前是否有可用驱动（原生已实现 / 外部运行时已接入）
    writeln!(c, "    pub fn implemented(self) -> bool {{").unwrap();
    writeln!(c, "        crate::drivers::builtin_kinds().contains(&self)").unwrap();
    writeln!(c, "    }}\n").unwrap();

    writeln!(c, "    pub fn descriptor(self) -> TypeDescriptor {{").unwrap();
    writeln!(c, "        TypeDescriptor {{").unwrap();
    writeln!(c, "            kind: self,").unwrap();
    writeln!(c, "            key: self.key(),").unwrap();
    writeln!(c, "            label: self.label(),").unwrap();
    writeln!(c, "            dialect: self.dialect(),").unwrap();
    writeln!(
        c,
        "            runtime_mode: self.runtime_mode(),
            protocol: self.protocol(),"
    )
    .unwrap();
    writeln!(c, "            mcp_mode: self.mcp_mode(),").unwrap();
    writeln!(c, "            agent_key: self.agent_key(),").unwrap();
    writeln!(c, "            default_port: self.default_port(),").unwrap();
    writeln!(c, "            local_file: self.local_file(),").unwrap();
    writeln!(c, "            skip_tcp_probe: self.skip_tcp_probe(),").unwrap();
    writeln!(
        c,
        "            single_connection_pool: self.single_connection_pool(),"
    )
    .unwrap();
    writeln!(
        c,
        "            metadata_connection_scoped: self.metadata_connection_scoped(),"
    )
    .unwrap();
    writeln!(c, "            max_connections: self.max_connections(),").unwrap();
    writeln!(c, "            driver_store_order: self.driver_store_order(),").unwrap();
    writeln!(c, "            support_level: self.support_level(),").unwrap();
    writeln!(c, "            traits: self.traits(),").unwrap();
    writeln!(c, "            capabilities: self.capabilities(),").unwrap();
    writeln!(c, "            implemented: self.implemented(),").unwrap();
    writeln!(c, "        }}").unwrap();
    writeln!(c, "    }}").unwrap();
    writeln!(c, "}}\n").unwrap();

    writeln!(c, "pub fn all_descriptors() -> Vec<TypeDescriptor> {{").unwrap();
    writeln!(
        c,
        "    ConnectionKind::ALL.iter().copied().map(|k| k.descriptor()).collect()"
    )
    .unwrap();
    writeln!(c, "}}\n").unwrap();

    // 序列化按 key 字符串（对外契约稳定，不随 Rust 变体名变化）
    // 注意：这里必须写全 `std::result::Result`——生成代码落在 crate 根，
    // 而根上有一个 `pub type Result<T> = ...`，写裸 `Result` 会被解析成它。
    writeln!(c, "impl serde::Serialize for ConnectionKind {{").unwrap();
    writeln!(
        c,
        "    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {{"
    )
    .unwrap();
    writeln!(c, "        serde::Serializer::serialize_str(s, self.key())").unwrap();
    writeln!(c, "    }}").unwrap();
    writeln!(c, "}}\n").unwrap();

    writeln!(c, "impl<'de> serde::Deserialize<'de> for ConnectionKind {{").unwrap();
    writeln!(
        c,
        "    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {{"
    )
    .unwrap();
    writeln!(
        c,
        "        let key = <String as serde::Deserialize>::deserialize(d)?;"
    )
    .unwrap();
    writeln!(
        c,
        "        ConnectionKind::from_key(&key).ok_or_else(|| <D::Error as serde::de::Error>::custom(format!(\"未知连接类型: {{key}}\")))"
    )
    .unwrap();
    writeln!(c, "    }}").unwrap();
    writeln!(c, "}}").unwrap();

    c
}

fn generate_manifest(types: &[TypeYaml]) -> serde_json::Value {
    let list = types
        .iter()
        .map(|t| {
            let mut capabilities = serde_json::Map::new();
            for (json_key, _) in CAPABILITIES {
                capabilities.insert(
                    (*json_key).to_string(),
                    serde_json::Value::Bool(capability_value(&t.capabilities, json_key)),
                );
            }
            serde_json::json!({
                "key": t.db_type,
                "rustVariant": t.rust_variant,
                "label": t.label,
                "dialect": t.dialect,
                "order": t.order,
                "runtimeMode": t.runtime_mode,
                "protocol": t.protocol.clone().unwrap_or_else(|| "sql".to_string()),
                "mcpMode": t.mcp_mode.clone().unwrap_or_else(|| "none".to_string()),
                "agentKey": t.agent_key,
                "defaultPort": t.default_port,
                "localFile": t.local_file,
                "skipTcpProbe": t.skip_tcp_probe,
                "singleConnectionPool": t.single_connection_pool,
                "metadataConnectionScoped": t.metadata_connection_scoped,
                "maxConnections": effective_max_connections(t),
                "driverStoreVisible": t.driver_store_visible,
                "driverStoreOrder": t.driver_store_order,
                "supportLevel": t.support_level,
                "jdbc": t.jdbc.as_ref().map(|j| serde_json::json!({
                    "driverClass": j.driver_class,
                    "urlTemplate": j.url_template,
                    "artifact": j.artifact,
                    "artifacts": j.all_artifacts(),
                })),
                "traits": {
                    "singleDatabase": t.traits.single_database,
                    "schemaAware": t.traits.schema_aware,
                },
                "capabilities": capabilities,
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "schemaVersion": 1,
        "generatedBy": "crates/dbmind-core/build.rs",
        "source": "plugins/connection-types/*.yaml",
        "types": list,
    })
}
