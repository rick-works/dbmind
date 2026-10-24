//! **SQLite 写插件**。
//!
//! 落库方式：多行 `INSERT`，5000 行 / 8MB；事务用标准 `begin` / `commit`。
//! 没有 `TRUNCATE`，清表走 `delete from`（见 `api::dialect`）。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::Spec;

/// SQLite 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("sqlite")
    .with_rows(5_000)
    .with_cap(8 * 1024 * 1024)
    .with_txn("begin", "commit", "");

crate::api::transfer::plugins::plugin_writer!(SqliteWriter);
