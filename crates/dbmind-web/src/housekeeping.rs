//! 开机清扫：**把「用了几个月之后数据目录变成几个 G」这件事挡在后台**。
//!
//! 实测用户数据目录 1.5 GB，其中三块是纯浪费：
//!
//! | 位置 | 实测 | 成因 |
//! |---|---|---|
//! | `dbmind.db` | 805 MB（其中 **750 MB 是空闲页**） | 删了又写的表只把页挂回空闲链，文件不会自己变小；此前没有任何回收路径 |
//! | `tasks/*.json` | 983 个 / 9 MB（只会更多） | 每次同步、比对都落一份终态快照，从不过期 |
//! | `loaddata/*.tsv` | 2.8 MB | 同步崩溃/取消时 Java 宿主来不及删的临时文件 |
//!
//! 三件事都在**后台线程**做，且都有硬上限（见各自函数），失败只记日志 ——
//! 清扫永远不该影响启动和任何一次查询。
//!
//! 刻意**不做**的事：删查询历史 / 审计日志。那些是用户数据，删不删只由
//! 「清空日志」按钮和设置里的保留策略决定（见 `Store::prune_history` 的注释）。
//! 这里只把**空洞**还回去，一条记录都不动。

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use dbmind_core::Store;

/// 任务终态快照保留个数。9.3 KB 一个、峰值几百 KB，500 个约几 MB ——
/// 再老的记录界面里也翻不到，留着只是占盘。
const TASK_KEEP: usize = 500;
/// 装载临时文件保留时长：一天。正在跑的同步刚落的 TSV 绝不会被扫到。
const TEMP_KEEP: Duration = Duration::from_secs(24 * 60 * 60);

/// 起一个后台线程做清扫（调用方不等待）。
pub fn spawn(store: Arc<Store>) {
    let started = SystemTime::now();
    let _ = std::thread::Builder::new()
        .name("dbmind-housekeeping".to_string())
        .spawn(move || {
            let reclaimed = match store.reclaim(false) {
                Ok(0) => 0,
                Ok(bytes) => {
                    tracing::info!(
                        freed_mb = format!("{:.1}", bytes as f64 / 1048576.0),
                        ms = started.elapsed().map(|d| d.as_millis()).unwrap_or(0),
                        "已回收元数据库空闲空间"
                    );
                    bytes
                }
                // 多壳共用一个库时可能撞上锁：下次开机会再来一次，不用惊动用户
                Err(err) => {
                    tracing::warn!(error = %err, "回收元数据库空间失败");
                    0
                }
            };
            let _ = reclaimed;

            let home = dbmind_core::paths::home_dir();
            sweep_temp(&home.join("loaddata"), TEMP_KEEP);
            trim_dir(&home.join("tasks"), TASK_KEEP, ".json");
        });
}

/// 删掉目录里超过 `keep` 的**普通文件**（最新的留着），返回删除数。
///
/// 口径刻意保守：只认 `.json` 后缀（任务快照），目录/其它后缀一律不碰。
fn trim_dir(dir: &Path, keep: usize, suffix: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut files: Vec<(SystemTime, std::path::PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(suffix)
        })
        .map(|entry| {
            let stamp = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (stamp, entry.path())
        })
        .collect();
    if files.len() <= keep {
        return 0;
    }
    files.sort_by(|left, right| right.0.cmp(&left.0));
    let mut removed = 0usize;
    for (_, path) in files.into_iter().skip(keep) {
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(dir = %dir.display(), removed, "已清理过期快照");
    }
    removed
}

/// 删掉目录里比 `keep` 更久的文件（同步残留的 TSV），返回删除数。
fn sweep_temp(dir: &Path, keep: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        let Ok(age) = SystemTime::now().duration_since(modified) else {
            continue;
        };
        if age >= keep && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(dir = %dir.display(), removed, "已清理临时文件");
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dbmind-hk-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn trim_keeps_newest_and_only_matching_suffix() {
        let dir = temp_dir("trim");
        for index in 0..10 {
            std::fs::write(dir.join(format!("task_{index}.json")), b"{}").unwrap();
        }
        std::fs::write(dir.join("keep.log"), b"x").unwrap();
        assert_eq!(trim_dir(&dir, 4, ".json"), 6);
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().is_file()).count(),
            5,
            "4 个快照 + 那个不是 .json 的文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_temp_skips_fresh_files() {
        let dir = temp_dir("temp");
        std::fs::write(dir.join("load-fresh.tsv"), b"a").unwrap();
        assert_eq!(sweep_temp(&dir, TEMP_KEEP), 0, "刚写的文件不能被扫掉");
        assert_eq!(sweep_temp(&dir, Duration::ZERO), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
