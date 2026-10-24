//! **PostgreSQL 读插件**。
//!
//! keyset 分页走 `(a, b) > (…, …)` 多列比较；无键表退回 `limit/offset`。
//! 分片按主键值域等分，区间连续不重不漏 —— 切错的表现是「某段数据压根没搬」或「搬了两遍」。
//!
//! 目前行为与通用 `SqlReader` 一致 —— 独立成文件是为了将来只改这一个文件
//!（换游标/分区消费、改页大小策略）就能换取数方式，注册表与引擎都不用动。

crate::api::transfer::plugins::plugin_reader!(PostgresqlReader);
