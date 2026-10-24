//! **达梦 DM 写插件**。
//!
//! 与 Oracle 同源：1000 行 / 预编译批 / 65535 表达式上限 / 无 SQL 级事务。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::{ExprLimit, Spec};

/// 达梦 DM 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("dm")
    .with_rows(1_000)
    .with_prepared()
    .with_expr_limit(ExprLimit::OracleLike);

crate::api::transfer::plugins::plugin_writer!(DmWriter);
