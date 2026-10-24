//! **DuckDB 读插件**。
//!
//! 内嵌分析库：分页与字面量走 `api::dialect`，分片受单连接池限制恒为 1。
//!
//! 目前行为与通用 `SqlReader` 一致 —— 独立成文件是为了将来只改这一个文件
//!（换游标/分区消费、改页大小策略）就能换取数方式，注册表与引擎都不用动。

crate::api::transfer::plugins::plugin_reader!(DuckdbReader);
