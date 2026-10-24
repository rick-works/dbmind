//! **Doris 写插件**。
//!
//! 落库方式：**Stream Load 快通道**（HTTP 直推 BE，`/api/<db>/<table>/_stream_load`），
//! 不可用时回退普通 `INSERT` 管道，数据不丢。
//! 普通 INSERT 只给 2000 行 / 512KB：FE 的 `max_allowed_packet` 默认 1MB。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::Spec;

/// Doris 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("doris")
    .with_rows(2_000)
    .with_cap(512 * 1024)
    .with_multi_statement()
    .with_stream_load()
    .with_activity_sql("select count(*) as n, coalesce(max(time), 0) as longest from information_schema.processlist where command = 'Query' and time >= 5");

crate::api::transfer::plugins::plugin_writer!(DorisWriter);
