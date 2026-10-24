//! **MariaDB 读插件**。
//!
//! 与 MySQL 同源：keyset 分页 + 主键值域分片，分页语法由 `api::dialect` 提供。
//!
//! 目前行为与通用 `SqlReader` 一致 —— 独立成文件是为了将来只改这一个文件
//!（换游标/分区消费、改页大小策略）就能换取数方式，注册表与引擎都不用动。

crate::api::transfer::plugins::plugin_reader!(MariadbReader);
