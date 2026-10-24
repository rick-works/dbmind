//! **DB2 写插件**。
//!
//! 落库方式：字面量多行 `INSERT`，**不用预编译批**：
//! jcc 的 `executeBatch` 一次 200 行就会整批通信中断（SQLState 08001），之后连接已关、
//! `rollback` 报 08003 ⇒ 无法确认回滚，只能整表失败。批量仍受 65535 表达式上限约束。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::{ExprLimit, Spec};

/// DB2 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("db2")
    .with_rows(1_000)
    .with_expr_limit(ExprLimit::OracleLike);

crate::api::transfer::plugins::plugin_writer!(Db2Writer);
