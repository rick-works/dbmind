//! **PostgreSQL 写插件**。
//!
//! 落库方式：多行 `INSERT`（PG 的多行 VALUES 效率好），2000 行 / 4MB 一条语句。
//! 事务用标准 `begin` / `commit`；一条语句失败会把整个事务标记为 aborted，
//! 所以批量降级逐行重试前必须先打保存点（见 [`Savepoint::Pg`]）。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::{Savepoint, Spec};

/// PostgreSQL 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("postgresql")
    .with_rows(2_000)
    .with_txn("begin", "commit", "")
    .with_savepoint(Savepoint::Pg)
    .with_multi_statement();

crate::api::transfer::plugins::plugin_writer!(PostgresqlWriter);
