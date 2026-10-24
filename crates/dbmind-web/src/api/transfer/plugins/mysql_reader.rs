//! **MySQL 读插件**。
//!
//! keyset 分页（`>` 严格大于 + 多列元组比较），分片按主键值域切；分页语法与字面量
//! 全部来自 `api::dialect`，本文件只回答「MySQL 这一侧怎么取数」。
//!
//! 目前行为与通用 `SqlReader` 一致 —— 独立成文件是为了将来只改这一个文件
//!（换游标/分区消费、改页大小策略）就能换取数方式，注册表与引擎都不用动。

crate::api::transfer::plugins::plugin_reader!(MysqlReader);
