//! 数据传输引擎（DataX 风格的 **Reader / Writer 插件 + 多通道流水线**）。
//!
//! ## 为什么要有这一层
//!
//! 改造前「读一页 → 写一页」的代码全塞在 `sync::sync_table_inner` 里（1000+ 行）：
//! 读的方言细节（分页语法、字面量）、写的行为（批量 INSERT / upsert / Stream Load）、
//! 并发、重试、进度、取消全缠在一起。想给某一种库换一种更快的写法（Doris 走 Stream
//! Load、同实例走 INSERT…SELECT）只能在那口大锅里加 `if`，越加越没人敢动。
//!
//! 这一层把它们拆成三件事：
//!
//! | 角色 | 职责 | 谁实现 |
//! |---|---|---|
//! | [`Reader`] | 把源表切成若干 **range**（DataX 的 split），每片按游标拉页 | `SqlReader`（按方言参数化） |
//! | [`Writer`] | 吃行批次、批量落库、脏行降级、事务提交 | `SqlWriter`（按方言参数化，Doris 走 Stream Load） |
//! | [`engine::run`] | 编排：分片 → N 读 → 有界通道 → M 写 → 计数/取消 | 一份 |
//!
//! 编排层只认 trait，所以「源 MySQL → 目标 Oracle」「源 PG → 目标 ClickHouse」之类的
//! 任意组合都是同一个引擎 + 两个不同的插件实例，不需要在引擎里出现任何数据库名字。
//!
//! ## 效率上的三个硬约束
//!
//! 1. **读写都并发**：源端按主键值域切 N 片并发拉（远端库单连接常被带宽/延迟卡死，
//!    N 条连接能吃到链路上限）；目标端 N 条独立连接并发写（`internal:sync-w{i}`，
//!    每条连接自带事务）。两侧并发数都由连接类型声明（见 `sync::write_parallelism`），
//!    单连接池的库（SQLite/H2/Derby/SQL Server）自动退回 1，不做无意义的排队。
//! 2. **有界通道 + 背压**：读快了会阻塞在通道上，内存峰值只与「在途页数 × 页大小」有关，
//!    与表大小无关。分片时页大小自动降到 5000 行（多片同时在途，避免几十页堆内存）。
//! 3. **单语句行数按目标方言定**（`sync::per_statement` 那套），引擎不掺和。
//!
//! ## 容错
//!
//! - **读**：单页失败退避重试（3 次），仍失败才判该表失败，错误原样上抛；
//!   读协程 panic 也会被兜成失败信号 —— 不允许把「少读了一部分」当成「读完了」。
//! - **写**：一条语句失败 → 降级为该语句块逐行重试（坏行单独计失败 + 记样例错误），
//!   好行照常落库；一批里过半失败判系统性问题（连接断/权限/表没了）立即中止。
//! - **取消**：每个页边界检查一次，已写入的部分保留（不回滚）。

pub mod engine;
pub mod reader;
pub mod writer;

pub use engine::run;
pub use reader::{Cursor, Page, Reader, RowRange};
pub use writer::Writer;

use std::sync::Arc;

use crate::api::dialect::Dialect;
use crate::api::tasks::Task;
use crate::AppState;

/// trait 方法里用的装箱 Future（不想为一个 `async fn in trait` 引入 `async-trait` 依赖）。
pub type BF<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Doris Stream Load 的连接参数（只有 doris 目标会带，其余为 `None`）。
#[derive(Clone, Debug, Default)]
pub struct StreamCfg {
    /// FE 的 MySQL 协议端口（Stream Load 的兜底入口，会 307 到 BE）
    pub fe_http_port: u16,
    /// BE 的真实 http 端口（`SHOW BACKENDS` 的 HttpPort，0 = 没探到，走 FE 重定向）
    pub be_http_port: u16,
    pub host: String,
    pub user: String,
    pub pass: String,
}

/// 一张表的**传输计划**：DDL（建表/索引/注释）由 sync 层做完，这里只管搬行。
///
/// 字段全是「已经解析好的值」——没有 `SyncRequest`、没有 `Opts`，插件不需要知道
/// 用户界面上勾了什么，只按这份计划执行。
pub struct TablePlan {
    pub state: AppState,
    pub task: Option<Arc<Task>>,

    // ---- 源 ----
    pub src_conn: String,
    pub src_scope: String,
    pub source_kind: Dialect,
    /// 源表名（已 scoped 的 `库.表`，直接进 FROM；也用于日志）
    pub src_from: String,
    /// 日志里显示的表名（与 `src_from` 可能不同：跨类型会加引号）
    pub table: String,
    pub columns: Vec<String>,

    // ---- 目标 ----
    pub tgt_conn: String,
    pub tgt_scope: String,
    /// 已解析出的目标连接 id（见 `scope::resolve`）
    pub tgt_target: String,
    pub target_kind: Dialect,
    /// 已 quote 的目标表名
    pub target_name: String,
    /// 与 `columns` 一一对应的目标列类型（用于字面量规范化，如日期补 `:00`）
    pub target_types: Vec<Option<String>>,

    // ---- 分页 / 过滤 ----
    pub key_columns: Vec<String>,
    pub key_indexes: Vec<usize>,
    /// 用于 keyset 游标的列（= 真实取到的键列；空 ⇒ 退回 OFFSET 分页）
    pub paging_keys: Vec<String>,
    pub where_clause: String,
    pub row_limit: u64,

    // ---- 写入行为 ----
    pub upsert: bool,
    /// upsert 预读出来的目标已有键（`key_of` 的产物）
    pub existing: Arc<std::collections::HashSet<String>>,
    /// 单条 INSERT 的目标行数（方言差异很大，见 sync 里的注释）
    pub per_statement: u64,
    pub identity_on: bool,
    pub stream: Option<StreamCfg>,
}

/// 一次表传输的结果。
#[derive(Default, Debug)]
pub struct Stats {
    pub inserted: u64,
    pub updated: u64,
    /// 用户取消（已写入的部分保留）
    pub canceled: bool,
    /// 收尾备注（追加到结果消息里）
    pub note: String,
}

/// 按源连接类型取 Reader 插件。
///
/// 现在所有关系型/列式库共用 `SqlReader`（分页语法与字面量由 `Dialect` 参数化）——
/// 它们「怎么翻页」的差别真的只在这两件事上。将来某个源需要完全不同的取数方式
/// （比如 MongoDB 的游标批量、Kafka 的分区消费），在这里按类型返回另一个实现即可，
/// 引擎和 Writer 一行都不用改。
pub fn reader_for(plan: &TablePlan) -> Box<dyn Reader> {
    reader::SqlReader::new(plan)
}

/// 按目标连接类型取 Writer 插件。
///
/// Doris 仍是 `SqlWriter`，但它会把 `stream` 配置用起来（HTTP 直推 BE）；其余目标走
/// JDBC 批量 INSERT。要加一种「完全不同的落库方式」（文件直写、对象存储、消息队列），
/// 在这里返回新实现即可。
pub fn writer_for(plan: &TablePlan) -> Arc<dyn Writer> {
    Arc::new(writer::SqlWriter::new(plan))
}
