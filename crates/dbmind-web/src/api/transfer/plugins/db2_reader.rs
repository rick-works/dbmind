//! **DB2 读插件**。
//!
//! 分页用 `fetch first … rows only` / `offset … rows`（见 `api::dialect`）。
//!
//! 目前行为与通用 `SqlReader` 一致 —— 独立成文件是为了将来只改这一个文件
//!（换游标/分区消费、改页大小策略）就能换取数方式，注册表与引擎都不用动。

crate::api::transfer::plugins::plugin_reader!(Db2Reader);
