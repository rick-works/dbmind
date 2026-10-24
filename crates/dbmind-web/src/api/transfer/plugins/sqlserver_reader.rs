//! **SQL Server 读插件**。
//!
//! 分页用 `offset … fetch next …`（必须有 `order by`，见 `api::dialect`），
//! 分片按主键值域切；`#temp` 之类会话级对象由同步层保证同连接。
//!
//! 目前行为与通用 `SqlReader` 一致 —— 独立成文件是为了将来只改这一个文件
//!（换游标/分区消费、改页大小策略）就能换取数方式，注册表与引擎都不用动。

crate::api::transfer::plugins::plugin_reader!(SqlServerReader);
