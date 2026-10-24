//! **DuckDB 写插件**。
//!
//! 落库方式：多行 `INSERT`，用**保守默认值**（1000 行 / 4MB / 无事务语句）——
//! 它还没有真机调优数据，宁可慢一点也不要先撞协议上限。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::Spec;

/// DuckDB 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("duckdb");

crate::api::transfer::plugins::plugin_writer!(DuckdbWriter);
