//! **SQL Server 写插件**。
//!
//! 落库方式：多行 `INSERT`（**1000 行是 T-SQL 解析器硬限制**），事务用 `begin transaction`。
//! 批量降级用 `save transaction` 保存点；预编译批对 SQL Server 是安全的（驱动可靠地
//! 提供提交/回滚边界），所以打开它省掉字面量渲染。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::{ExprLimit, Savepoint, Spec};

/// SQL Server 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("sqlserver")
    .with_rows(1_000)
    .with_txn("begin transaction", "commit", "")
    .with_savepoint(Savepoint::MsSql)
    .with_prepared()
    .with_expr_limit(ExprLimit::SqlServer)
    .with_multi_statement();

crate::api::transfer::plugins::plugin_writer!(SqlServerWriter);
