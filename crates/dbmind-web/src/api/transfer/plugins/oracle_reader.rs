//! **Oracle 读插件**。
//!
//! 没有 `LIMIT`：分页统一 `offset … rows fetch next … rows only`（见 `api::dialect`）。
//! 表挂在**用户（schema）**下，分片仍按主键值域切。
//!
//! 目前行为与通用 `SqlReader` 一致 —— 独立成文件是为了将来只改这一个文件
//!（换游标/分区消费、改页大小策略）就能换取数方式，注册表与引擎都不用动。

crate::api::transfer::plugins::plugin_reader!(OracleReader);
