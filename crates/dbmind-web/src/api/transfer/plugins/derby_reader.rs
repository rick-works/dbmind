//! **Derby 读插件**。
//!
//! 内嵌库：不认 `limit`，分页走 `offset … fetch next …`（见 `api::dialect`）。
//!
//! 目前行为与通用 `SqlReader` 一致 —— 独立成文件是为了将来只改这一个文件
//!（换游标/分区消费、改页大小策略）就能换取数方式，注册表与引擎都不用动。

crate::api::transfer::plugins::plugin_reader!(DerbyReader);
