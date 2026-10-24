//! **Oracle 写插件**。
//!
//! 落库方式：多行 `INSERT`，每批 1000 行；**没有 SQL 级事务**（DDL/DML 前后自动提交），
//! 所以 `SPEC` 不声明事务语句，原子性靠前端把变更合并成一条 ALTER。
//! 派生表的 `(值1, 值2, …)` 有 65535 个表达式上限 ⇒ 宽表按列数收窄每批行数。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::{ExprLimit, Spec};

/// Oracle 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("oracle")
    .with_rows(1_000)
    .with_prepared()
    .with_expr_limit(ExprLimit::OracleLike);

crate::api::transfer::plugins::plugin_writer!(OracleWriter);
