//! **搬运限速**：把「每秒最多搬多少行 / 多少字节」变成一条共享的时间表。
//!
//! ## 为什么不能各睡各的
//!
//! 一张表可能有 4 路 writer（4 条事务连接）并发写，若每路都按「用户填的速率」睡，
//! 合计就是 4 倍 —— 用户填 5000 行/秒会看到 2 万行/秒，限速等于没限。所以调度时刻是
//! **全任务共享**的一份（`next`）：每一批先领一个时间槽，槽的结束时刻 = 开始 + 本批应
//! 占用的时长，领完就睡到自己的结束时刻。这样无论几路并发，合计速率恒等于设定值。
//!
//! ## 两条曲线取更慢的
//!
//! 界面同时给了「行/秒」和「MB/秒」：窄表按行算、宽表按字节算才是用户真正想要的，
//! 两条都以 0 表示不限，填了就取更慢的那条（等价于 `max(行耗时, 字节耗时)`）。
//!
//! ## 只影响吞吐，不影响语义
//!
//! 限速只决定「多久发下一批」，不改变批量大小、事务边界与重试逻辑；目标端慢到本身就
//! 低于限速时，`next` 追不上 `now`，一行都不用睡（不会拖慢本来就慢的任务）。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use dbmind_core::CellValue;

/// 限速表。`Pacer::new` 在两条曲线都为 0 时返回 `None`（完全不介入热路径）。
#[derive(Debug)]
pub struct Pacer {
    rows_per_sec: u64,
    bytes_per_sec: u64,
    /// 下一批可以**结束**的时刻（全任务共享，见模块文档）
    next: Mutex<Instant>,
}

impl Pacer {
    /// 按界面参数建限速表；两条都是 0 ⇒ `None`（不限速）。
    pub fn new(rows_per_sec: u64, bytes_per_sec: u64) -> Option<std::sync::Arc<Self>> {
        if rows_per_sec == 0 && bytes_per_sec == 0 {
            return None;
        }
        Some(std::sync::Arc::new(Self {
            rows_per_sec,
            bytes_per_sec,
            next: Mutex::new(Instant::now()),
        }))
    }

    /// 这一批（`rows` 行 / `bytes` 字节）按两条曲线算出的**最小占用时长**。
    fn need(&self, rows: u64, bytes: u64) -> Duration {
        let by_rows = if self.rows_per_sec == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(rows as f64 / self.rows_per_sec as f64)
        };
        let by_bytes = if self.bytes_per_sec == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(bytes as f64 / self.bytes_per_sec as f64)
        };
        by_rows.max(by_bytes)
    }

    /// 领一个时间槽并睡到槽结束。并发调用者按领取顺序**串行**占用时间表，
    /// 所以合计速率不会超过设定值。
    pub async fn pace(&self, rows: u64, bytes: u64) {
        let need = self.need(rows, bytes);
        if need.is_zero() {
            return;
        }
        let wait = {
            let mut next = match self.next.lock() {
                Ok(guard) => guard,
                // 锁中毒只可能是别的任务 panic 过：时间表照用（最坏是这一批多睡一点）
                Err(poisoned) => poisoned.into_inner(),
            };
            let now = Instant::now();
            // 上一次的槽还没结束就从它的结束时刻接着排，否则从现在开始
            let start = (*next).max(now);
            *next = start + need;
            (*next).saturating_duration_since(now)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// 一页数据的**近似字节数**：限速用，不要求精确，只要求「宽表和窄表的量级对得上」。
///
/// 只数真正占空间的字段（文本/定点/二进制/只带长度的超大字列），数字类按 8 字节估。
/// 不按渲染后的 SQL 长度算：那要在写侧渲染完才知道，而限速必须在**发出去之前**生效。
pub fn page_bytes(rows: &[Vec<CellValue>]) -> u64 {
    let mut total: u64 = 0;
    for row in rows {
        for value in row {
            total += match value {
                CellValue::Text(text) => text.len() as u64,
                CellValue::Decimal(text) => text.len() as u64,
                CellValue::Bytes(bytes) => bytes.len() as u64,
                CellValue::Blob { len } => *len as u64,
                CellValue::Null => 1,
                CellValue::Integer(_) | CellValue::Real(_) => 8,
            };
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(count: usize, text: &str) -> Vec<Vec<CellValue>> {
        (0..count)
            .map(|_| vec![CellValue::Integer(1), CellValue::Text(text.to_string())])
            .collect()
    }

    /// 两条曲线取更慢的：行 10/s（10 行 = 1 秒）比字节 1000/s（14 字节 ≈ 0.014 秒）慢
    #[test]
    fn 两条曲线取更慢的() {
        let pacer = Pacer::new(10, 1_000).expect("有速率就该有 pacer");
        let need = pacer.need(10, 14 * 10);
        assert!(need >= Duration::from_millis(990), "应按行算：{need:?}");
    }

    /// 两条都是 0 ⇒ 不建表（热路径连一次 Instant::now 都不做）
    #[test]
    fn 不限速时没有限速表() {
        assert!(Pacer::new(0, 0).is_none());
        assert!(Pacer::new(0, 1_000_000).is_some());
        assert!(Pacer::new(500, 0).is_some());
    }

    /// 字节估算：文本按实际长度、数字按 8、Null 按 1
    #[test]
    fn 页字节按内容估算() {
        let page = rows(2, "abcdef");
        // 2 行 × (8 + 6)
        assert_eq!(page_bytes(&page), 28);
        assert_eq!(page_bytes(&[vec![CellValue::Null]]), 1);
    }
}
