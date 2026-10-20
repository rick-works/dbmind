//! Reader 插件：把源表切成若干 range，每片按游标一页页拉。
//!
//! `SqlReader` 是**通用实现**：翻页语法（keyset / OFFSET / `limit`/`top`/`offset fetch`）
//! 与字面量（日期、二进制、转义）都由 `Dialect` 负责，所以 MySQL / PG / Oracle /
//! SQL Server / ClickHouse / Doris / DB2 / H2 / Derby / SQLite 共用这一份 —— 它们
//! 「怎么把数据取出来」的实际差别真的只在这两件事上。
//!
//! 需要完全不同的取数方式时（MongoDB 的游标批读、Kafka 的分区消费、文件的按块读），
//! 实现 [`Reader`] 再在 `transfer::reader_for` 里按连接类型返回即可，引擎不用动。

use std::sync::atomic::{AtomicUsize, Ordering};

use dbmind_core::CellValue;

use super::{BF, TablePlan};
use crate::api::dialect::Dialect;
use crate::api::meta::run_sql_in;
use crate::api::sync::{estimate_row_bytes, fetch_page, literal_for, write_parallelism};
use crate::AppState;

/// 单流初始页 2 万行；拿到第一页后会按实际行宽自动升到最多 10 万行。
pub const PAGE_WIDE: usize = 20_000;
/// 分片并发初始页 1 万行：多片同时在途，先控制内存，再按实际行宽自适应 ——
/// 页码是「一次往返」的成本，5 千行/页会让 20 万行变成 40 次往返，在延迟高的链路上
/// 反而比单流慢（真机实测 4 片 5 千行/页 11s vs 单流 5s）。
pub const PAGE_SPLIT: usize = 10_000;
const PAGE_BYTES_TARGET: usize = 32 * 1024 * 1024;
const PAGE_ROWS_MIN: usize = 1_000;
const PAGE_ROWS_MAX: usize = 100_000;

/// 读页的会话键前缀（分片时后面拼片号，整表唯一后缀拼 `sid`）
const SESSION_READ: &str = "internal:sync-r";

/// **每片至少要有的行数**（5 页），低于这个量级就不分片。
///
/// 分片不是白给的：每片一条新连接（远端库的建连 + 认证在跨网链路上要几百毫秒到一两秒），
/// 页小了还要多打几次往返。真机实测（远端 MySQL 20 万行 → 本地 H2）：
/// 单流 4.8~6.8s，4 片 6.3~7.9s —— 链路是带宽瓶颈时，分片只摊到了「多建 3 条连接」的成本。
/// 大表（每片 10 万行以上）才是分片的战场：那时建连成本被摊薄，服务端并行扫描才真正生效。
const MIN_ROWS_PER_SLICE: i64 = 100_000;

/// 分页游标：上一页停在哪。
#[derive(Clone, Debug, Default)]
pub struct Cursor {
    /// keyset 游标的键值（`paging_keys` 顺序）；无键表为 `None`
    pub key: Option<Vec<CellValue>>,
    /// 无键表用的 OFFSET
    pub offset: u64,
}

/// 一页数据。
pub struct Page {
    pub rows: Vec<Vec<CellValue>>,
    /// `None` = 这一片读完了
    pub next: Option<Cursor>,
}

/// 一片：`[low, high)` 的键值区间，两侧都可以是开区间。
#[derive(Clone, Debug, Default)]
pub struct RowRange {
    /// 片序号：每片要占一条**独立的物理会话**（会话键不同才会分到不同连接），
    /// 否则几片会挤在一条连接上排队，分片就退化成「多打几次往返」。
    pub index: usize,
    pub low: Option<CellValue>,
    pub high: Option<CellValue>,
}

impl RowRange {
    pub fn is_whole(&self) -> bool {
        self.low.is_none() && self.high.is_none()
    }
}

/// 取数插件。
pub trait Reader: Send + Sync {
    /// 插件名（进日志/诊断）
    fn name(&self) -> &'static str;
    /// 当前一页的行数
    fn page_rows(&self) -> usize;
    /// 这个源最多能并发开几片（由连接类型声明决定）
    fn max_parallel(&self) -> usize;
    /// 切分。返回 1 片 = 不分片（无键 / 有行数上限 / 单连接池的库）。
    fn split<'a>(&'a mut self, want: usize) -> BF<'a, Result<Vec<RowRange>, String>>;
    /// 拉一片的下一页
    fn page<'a>(&'a self, range: &'a RowRange, cursor: Option<Cursor>) -> BF<'a, Result<Page, String>>;
}

/// 通用 SQL 取数插件。
pub struct SqlReader {
    state: AppState,
    conn: String,
    scope: String,
    /// `select 列 from 表`（不含 where）
    base: String,
    /// 表表达式（分片探测 min/max/count 用）
    from: String,
    dialect: Dialect,
    paging_keys: Vec<String>,
    key_positions: Vec<usize>,
    where_clause: String,
    row_limit: u64,
    page_rows: AtomicUsize,
    /// 本表的会话后缀（见 `TablePlan::sid`）：并发整库同步时，不同表的同号分片
    /// 若用同一个会话键就会落到同一条物理连接上，分片退化成排队
    sid: String,
    /// 本表最多几片并发读（0 = 方言默认），见 `TablePlan::slices_cap`
    slices_cap: usize,
}

impl SqlReader {
    pub fn new(plan: &TablePlan) -> Box<dyn Reader> {
        let columns = plan
            .columns
            .iter()
            .map(|name| plan.source_kind.quote(name))
            .collect::<Vec<_>>()
            .join(", ");
        Box::new(Self {
            state: plan.state.clone(),
            conn: plan.src_conn.clone(),
            scope: plan.src_scope.clone(),
            base: format!("select {columns} from {}", plan.src_from),
            from: plan.src_from.clone(),
            dialect: plan.source_kind,
            paging_keys: plan.paging_keys.clone(),
            key_positions: plan.key_indexes.clone(),
            where_clause: plan.where_clause.clone(),
            row_limit: plan.row_limit,
            page_rows: AtomicUsize::new(PAGE_WIDE),
            sid: plan.sid.clone(),
            slices_cap: plan.slices_cap,
        })
    }

    fn page_rows_now(&self) -> usize {
        self.page_rows.load(Ordering::SeqCst)
    }

    fn filter_sql(&self) -> String {
        let trimmed = self.where_clause.trim();
        if trimmed.is_empty() {
            String::new()
        } else {
            format!(" where {trimmed}")
        }
    }

    /// 取一行里的键值（游标用）
    fn key_of(&self, row: &[CellValue]) -> Option<Vec<CellValue>> {
        if self.paging_keys.is_empty() {
            return None;
        }
        Some(
            self.key_positions
                .iter()
                .filter_map(|index| row.get(*index).cloned())
                .collect(),
        )
    }

    /// `min/max` 一次拿到范围：整数/浮点键按值域等分，最省（不用数行、不用扫表）。
    async fn numeric_ranges(&self, want: usize) -> Option<Vec<RowRange>> {
        let key = self.dialect.quote(&self.paging_keys[0]);
        let probe = format!(
            "select min({key}) as mn, max({key}) as mx from {}{}",
            self.from,
            self.filter_sql()
        );
        // 极值查询很轻（有索引直接走两端），失败就退回单流
        let result = run_sql_in(&self.state, &self.conn, &self.scope, probe, 1).await.ok()?;
        let row = result.rows.first()?;
        if row.len() < 2 {
            return None;
        }
        match (&row[0], &row[1]) {
            (CellValue::Integer(lo), CellValue::Integer(hi)) => split_integer(*lo, *hi, want),
            (CellValue::Real(lo), CellValue::Real(hi)) => split_real(*lo, *hi, want),
            _ => None,
        }
    }

    /// 文本/日期/二进制键：没有值域可算，只能「数行 + 按占比采样切点」。
    ///
    /// 采样代价是 `want-1` 次带索引的 `offset` 定位（不是全表扫），对几十万行以上的表
    /// 完全可以接受；行数少（< want*2）时直接不分片。
    async fn sampled_ranges(&self, want: usize) -> Option<Vec<RowRange>> {
        let key = self.dialect.quote(&self.paging_keys[0]);
        let count_sql = format!("select count(*) as n from {}{}", self.from, self.filter_sql());
        let result = run_sql_in(&self.state, &self.conn, &self.scope, count_sql, 1).await.ok()?;
        let total = match result.rows.first()?.first()? {
            CellValue::Integer(n) => *n,
            CellValue::Real(n) => *n as i64,
            CellValue::Text(text) => text.trim().parse::<i64>().ok()?,
            _ => return None,
        };
        if total < (want as i64) * MIN_ROWS_PER_SLICE {
            return None;
        }
        let mut bounds: Vec<CellValue> = Vec::with_capacity(want - 1);
        for index in 1..want {
            let offset = total * index as i64 / want as i64;
            let sql = format!(
                "select {key} as k from {}{} order by {key} {}",
                self.from,
                self.filter_sql(),
                self.dialect.limit_clause(offset as u64, 1)
            );
            let probe = run_sql_in(&self.state, &self.conn, &self.scope, sql, 1).await.ok()?;
            let value = probe.rows.first()?.first()?.clone();
            if matches!(value, CellValue::Null) {
                return None;
            }
            bounds.push(value);
        }
        let mut out = Vec::with_capacity(want);
        for index in 0..want {
            let low = if index == 0 { None } else { Some(bounds[index - 1].clone()) };
            let high = if index + 1 == want {
                None
            } else {
                Some(bounds[index].clone())
            };
            out.push(RowRange { index, low, high });
        }
        Some(out)
    }
}

impl Reader for SqlReader {
    fn name(&self) -> &'static str {
        "sql-reader"
    }

    fn page_rows(&self) -> usize {
        self.page_rows_now()
    }

    fn max_parallel(&self) -> usize {
        // 排查用的逃生阀：某些源库在并发读下会打架（锁/游标/权限），设了这个环境变量
        // 就退回单流，不必改代码重编。
        if std::env::var_os("DBMIND_SYNC_NO_SPLIT").is_some() {
            return 1;
        }
        // 源端能不能并行取数，取决于**源库**声明的连接数：单连接池的库（SQLite/H2/
        // Derby/SQL Server）多开只会在池子里排队，反而多一层上下文切换。
        let base = write_parallelism(self.dialect.kind);
        // 任务级预算（见 `TablePlan::slices_cap`）：并发同步多张表时每张表少开几片，
        // 整次任务的源端读连接数也不会超。
        if self.slices_cap > 0 {
            base.min(self.slices_cap).max(1)
        } else {
            base
        }
    }

    fn split<'a>(&'a mut self, want: usize) -> BF<'a, Result<Vec<RowRange>, String>> {
        Box::pin(async move {
            let whole = vec![RowRange::default()];
            // 不分片的两种情况，理由各不相同：
            // - 无键：只能 OFFSET 翻页，切了也是每片从头扫；
            // - 有行数上限：全局配额要按顺序消费，切了就不知道谁先谁后。
            //
            // **复合主键照切**：分片只按**首列**取值域区间 `k1 >= lo and k1 < hi`（区间互不
            // 重叠、合起来覆盖全表），页内游标仍然用**完整键**做 keyset —— 两者 AND 在一起，
            // 一条行只会落在一个分片里。以前这里要求「恰好一列键」，于是复合主键的大表
            // 永远单流读（几十万行的表只有一条连接在拉）。
            if want <= 1 || self.paging_keys.is_empty() || self.row_limit > 0 {
                self.page_rows.store(PAGE_WIDE, Ordering::SeqCst);
                return Ok(whole);
            }
            let ranges = match self.numeric_ranges(want).await {
                Some(ranges) => Some(ranges),
                None => self.sampled_ranges(want).await,
            };
            match ranges {
                Some(ranges) if ranges.len() > 1 => {
                    // 分片时把页调小：N 片同时在途，页大了内存与延迟都会变差
                    self.page_rows.store(PAGE_SPLIT, Ordering::SeqCst);
                    Ok(ranges)
                }
                _ => {
                    self.page_rows.store(PAGE_WIDE, Ordering::SeqCst);
                    Ok(whole)
                }
            }
        })
    }

    fn page<'a>(&'a self, range: &'a RowRange, cursor: Option<Cursor>) -> BF<'a, Result<Page, String>> {
        Box::pin(async move {
            let (key, offset) = match cursor {
                Some(cursor) => (cursor.key, cursor.offset),
                None => (None, 0),
            };
            // 三件互不相干的条件，各自判断有没有：用户过滤 / 分片区间 / keyset 游标。
            // 曾经把「用户没写过滤」当成「没有条件」整个丢掉，连游标一起扔了 ——
            // 第二页把第一页又读一遍（详见 sync::page_sql 的注释）。
            let mut parts: Vec<String> = Vec::new();
            let user_where = self.where_clause.trim();
            if !user_where.is_empty() {
                parts.push(format!("({user_where})"));
            }
            if !range.is_whole() {
                let column = self.dialect.quote(&self.paging_keys[0]);
                if let Some(low) = range.low.as_ref() {
                    parts.push(format!("{column} >= {}", literal_for(low, self.dialect, None)));
                }
                if let Some(high) = range.high.as_ref() {
                    parts.push(format!("{column} < {}", literal_for(high, self.dialect, None)));
                }
            }
            let extra = parts.join(" and ");
            let cursor_arg = match (key.as_ref(), self.paging_keys.is_empty()) {
                (Some(values), false) => Some((self.paging_keys.as_slice(), values.as_slice())),
                _ => None,
            };
            let page_rows = self.page_rows_now();
            // 每片一条独立会话键：同一把键会被会话池归到同一条泳道（=同一条物理连接），
            // 四片并发就变成排队 —— 这是分片「看起来开了、实际没并发」的经典坑。
            // `sid` 同样必须带上：只按片号分键的话，**两张表**的同号分片会共用一条连接。
            let session = if range.is_whole() {
                format!("{SESSION_READ}-{}", self.sid)
            } else {
                format!("{SESSION_READ}-{}-{}", self.sid, range.index)
            };
            let rows = fetch_page(
                &self.state,
                &self.conn,
                &self.scope,
                &self.base,
                self.dialect,
                offset,
                cursor_arg,
                &extra,
                page_rows as u64,
                &session,
            )
            .await
            .map_err(|err| err.message)?;

            // 行数不能代表内存/网络页大小。先按实际页估算字节数，再为下一页
            // 调整行数：窄表逐步升到 10 万行，宽表自动降到至少 1 千行。
            let estimated_bytes = rows.iter().map(|row| estimate_row_bytes(row)).sum::<usize>();
            if estimated_bytes > 0 {
                let next_rows = ((page_rows as u128 * PAGE_BYTES_TARGET as u128)
                    / estimated_bytes as u128)
                    .clamp(PAGE_ROWS_MIN as u128, PAGE_ROWS_MAX as u128) as usize;
                // 小变化不折腾分页；只有明显更宽/更窄时才换档。
                if next_rows * 2 < page_rows || next_rows > page_rows * 2 {
                    self.page_rows.store(next_rows, Ordering::SeqCst);
                }
            }

            let done = rows.len() < page_rows;
            let next = if done {
                None
            } else {
                Some(Cursor {
                    key: rows.last().and_then(|row| self.key_of(row)),
                    offset: offset + page_rows as u64,
                })
            };
            Ok(Page { rows, next })
        })
    }
}

/// 整数键按值域等分：切出来的区间**连续、不重不漏**（相邻两片的边界严格相接）。
///
/// 纯函数单独拎出来是为了能被单测钉死 —— 分片切错的表现是「某几段数据压根没搬」
/// 或者「同一行搬了两遍」，而这两种在整库同步里都很难一眼看出来。
fn split_integer(lo: i64, hi: i64, want: usize) -> Option<Vec<RowRange>> {
    if want < 2 || hi <= lo {
        return None;
    }
    let span = (hi as i128) - (lo as i128) + 1;
    // 每片摊不到 MIN_ROWS_PER_SLICE 行就别切：建连成本会吃掉并行收益
    if span < (want as i128) * (MIN_ROWS_PER_SLICE as i128) {
        return None;
    }
    let mut out = Vec::with_capacity(want);
    for index in 0..want {
        let low = if index == 0 {
            None
        } else {
            Some(CellValue::Integer((lo as i128 + span * index as i128 / want as i128) as i64))
        };
        let high = if index + 1 == want {
            None
        } else {
            Some(CellValue::Integer(
                (lo as i128 + span * (index + 1) as i128 / want as i128) as i64,
            ))
        };
        out.push(RowRange { index, low, high });
    }
    Some(out)
}

/// 浮点/数值键同 `split_integer`，边界按比例落在值域上。
fn split_real(lo: f64, hi: f64, want: usize) -> Option<Vec<RowRange>> {
    if want < 2 || !(hi > lo) {
        return None;
    }
    let span = hi - lo;
    if span < (want as f64) * (MIN_ROWS_PER_SLICE as f64) {
        return None;
    }
    let mut out = Vec::with_capacity(want);
    for index in 0..want {
        let low = if index == 0 {
            None
        } else {
            Some(CellValue::Real(lo + span * index as f64 / want as f64))
        };
        let high = if index + 1 == want {
            None
        } else {
            Some(CellValue::Real(lo + span * (index + 1) as f64 / want as f64))
        };
        out.push(RowRange { index, low, high });
    }
    Some(out)
}

/// 给引擎/日志用的一句话描述（诊断时能看出「这张表怎么读的」）。
pub fn describe(range: &RowRange, table: &str) -> String {
    if range.is_whole() {
        format!("{table}：整表单流")
    } else {
        format!(
            "{table}：区间 [{}, {})",
            range.low.as_ref().map(|v| format!("{v:?}")).unwrap_or_else(|| "-∞".to_string()),
            range.high.as_ref().map(|v| format!("{v:?}")).unwrap_or_else(|| "+∞".to_string())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 分片的第一条底线：**相邻两片的边界严格相接**（上一片的 high == 下一片的 low），
    /// 首片下界开放、末片上界开放。任何一处不接，就意味着一整段数据漏搬或重复搬。
    #[test]
    fn 整数分片连续且不重不漏() {
        let ranges = split_integer(1, 1_000_000, 4).expect("100 万行应能切 4 片");
        assert_eq!(ranges.len(), 4);
        assert!(ranges[0].low.is_none(), "首片应取下界开放");
        assert!(ranges[3].high.is_none(), "末片应取上界开放");
        for pair in ranges.windows(2) {
            assert_eq!(
                format!("{:?}", pair[0].high),
                format!("{:?}", pair[1].low),
                "相邻片边界不相接"
            );
        }
        for (index, range) in ranges.iter().enumerate() {
            assert_eq!(range.index, index, "片号必须连续（会话键靠它区分连接）");
        }
    }

    #[test]
    fn 行数太少或只有一片时不切() {
        // 1000 行切 4 片，每片 250 行 —— 建连成本比省下的时间还多
        assert!(split_integer(1, 1_000, 4).is_none());
        assert!(split_integer(1, 1, 4).is_none(), "单行表不能切");
        assert!(split_integer(1, 1_000_000, 1).is_none(), "want=1 就是不切");
    }

    #[test]
    fn 浮点分片同样首尾开放() {
        let ranges = split_real(0.0, 1_000_000.0, 3).expect("应能切 3 片");
        assert_eq!(ranges.len(), 3);
        assert!(ranges[0].low.is_none());
        assert!(ranges[2].high.is_none());
    }

    /// 无键表的整片：不带任何边界条件（分页只能走 OFFSET）
    #[test]
    fn 整片不带边界() {
        let whole = RowRange::default();
        assert!(whole.is_whole());
        assert_eq!(whole.index, 0);
    }
}
