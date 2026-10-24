//! **SQLite 读插件**。
//!
//! 文件型库：`database` 就是那个文件，分片恒为 1（单连接池）。
//!
//! 目前行为与通用 `SqlReader` 一致 —— 独立成文件是为了将来只改这一个文件
//!（换游标/分区消费、改页大小策略）就能换取数方式，注册表与引擎都不用动。

crate::api::transfer::plugins::plugin_reader!(SqliteReader);
