//! **ClickHouse 写插件**。
//!
//! 落库方式：多行 `INSERT`，**10000 行 / 8MB** —— 一条 INSERT 就是一次 part 写入，
//! 批量越大越省（往返与 part 合并都摊薄）。**没有 SQL 级事务**，失败就是整批失败。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::Spec;

/// ClickHouse 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("clickhouse")
    .with_rows(10_000)
    .with_cap(8 * 1024 * 1024);

crate::api::transfer::plugins::plugin_writer!(ClickhouseWriter);
