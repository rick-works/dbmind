//! # dbmind-core —— DBMind 内核
//!
//! 内核**只有逻辑，没有任何传输与界面**：不认识 HTTP、不认识 stdout 协议、不认识窗口。
//! 四个壳（桌面 / Web / CLI / MCP）都只依赖本 crate 的 `DbMindEngine`。
//!
//! ```text
//!  壳：src-tauri / dbmind-cli / dbmind-mcp / dbmind-web
//!        └──────────────┬──────────────┘
//!              DbMindEngine（本 crate）
//!        ┌──────────────┴──────────────┐
//!   safety 闸门   drivers 驱动   storage 元数据库
//!        └──────────────┬──────────────┘
//!   ConnectionKind（由 plugins/connection-types/*.yaml 编译期生成）
//! ```

mod agent;
mod cancel;
mod drivers;
mod engine;
mod bootstrap;
mod error;
mod es;
mod kind;
mod mongo;
// 对外公开：壳层要往同一个应用目录里放自己的产物（导出文件、备份），
// 而 `DBMIND_HOME` 的重定向规则只能有一份 —— 各自再实现一遍，迟早有一处漏了那个环境变量。
pub mod paths;
mod redis;
mod safety;
mod sql;
mod statement;
mod storage;
// SSH 隧道：连接是否要经跳板机，是「连数据库」这件事的一部分，
// 所以放在内核 —— 四个壳共用同一套行为（见模块文档）。
pub mod tunnel;
mod types;

pub use agent::{
    availability_all, bundled_native_auth_dir, driver_artifact_url, driver_artifact_url_with_base,
    driver_dir, driver_dirs, driver_jar_name, driver_upload_target, has_host_for,
    install_driver_jar, installed_driver_jars, driver_mirror_base, native_arch, AgentAvailability, AgentHost, AgentHostSpec, AGENTS_DIR_ENV, MAVEN_CENTRAL,
};
pub use cancel::{CancelRegistry, CancelToken};
pub use drivers::{
    builtin_kinds, unimplemented_driver, Driver, DriverRegistry, QueryCall, TxAction,
};
pub use engine::{DriverEntry, DriverReport, DbMindEngine, RuntimeSummary};
pub use error::{query_error, DbMindError, ErrorCode, ErrorPayload, Result};
pub use kind::{Capabilities, McpMode, RuntimeMode, RuntimeProtocol, TypeDescriptor, TypeTraits};
pub use safety::SafetyPolicy;
pub use sql::StatementKind;
// 语句判定按协议分派：壳层拿到的 statementKind 也是由这里判出来的
pub use statement::{classify, is_read_only, split_statements, statement_count};
// `global_store` / `install_global_store`：AI 设置这类"环境式"读写要按需取主库句柄，
// 句柄由壳层在启动时显式装一次（不在 Store 构造里自动装，否则单测的内存库会互相串）。
pub use bootstrap::InitReport;
pub use storage::{
    global_store, install_global_store, AiExampleRow, AiGlossaryRow, AiModelRow, AiQualityRuleRow,
    AiSettingsRow, AiUsageRow, KbDocRow, KbInfoRow, KbVectorRow, Store,
};
pub use types::{
    AccessContext, CellValue, ColumnDetail, ColumnMeta, ConnectReport, ConnectionConfig, ConnectionRecord,
    HistoryEntry, HistoryStatus, NewHistoryEntry, QueryOptions, QueryRequest, QueryResult, SchemaCacheInfo,
    TableInfo, TableKind, EXTRA_SHADOW_FOR,
};

// `ConnectionKind` 及全部类型元数据由 build.rs 从 YAML 生成，直接展开到 crate 根。
// （用普通注释而非文档注释：rustdoc 不为宏展开生成文档）
include!(concat!(env!("OUT_DIR"), "/connection_types.rs"));

/// 生成一个执行 id（壳层在发起前先拿到，用于取消与前端关联）。
pub fn new_execution_id() -> String {
    storage::new_id("exec")
}

/// 内核版本（与 Cargo 包版本一致，供壳层显示）。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
