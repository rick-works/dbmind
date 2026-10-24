//! **H2 写插件**。
//!
//! 落库方式：多行 `INSERT`，5000 行 / 8MB；预编译批在 H2 上是安全的（驱动可靠地
//! 提供提交/回滚边界），打开它省掉字面量渲染。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::Spec;

/// H2 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("h2")
    .with_rows(5_000)
    .with_cap(8 * 1024 * 1024)
    .with_prepared();

crate::api::transfer::plugins::plugin_writer!(H2Writer);
