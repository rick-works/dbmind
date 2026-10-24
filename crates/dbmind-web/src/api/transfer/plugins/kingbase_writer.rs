//! **KingbaseES 写插件**。
//!
//! 与 PostgreSQL 同源：多行 `INSERT` + `begin`/`commit`，保存点用于批量降级逐行重试。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::{Savepoint, Spec};

/// KingbaseES 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("kingbase")
    .with_rows(2_000)
    .with_txn("begin", "commit", "")
    .with_savepoint(Savepoint::Pg)
    .with_multi_statement();

crate::api::transfer::plugins::plugin_writer!(KingbaseWriter);
