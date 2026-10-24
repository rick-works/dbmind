//! **Derby 写插件**。
//!
//! 落库方式：多行 `INSERT`，5000 行 / 8MB（嵌入式没有网络往返成本）。
//! 不声明事务语句与保存点（Derby 的批事务在宿主侧不可靠，逐条提交更稳）。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::Spec;

/// Derby 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("derby")
    .with_rows(5_000)
    .with_cap(8 * 1024 * 1024);

crate::api::transfer::plugins::plugin_writer!(DerbyWriter);
