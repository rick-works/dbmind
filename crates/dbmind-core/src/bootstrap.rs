//! 首次运行的初始化。
//!
//! ## 为什么要它
//!
//! 以前"初次安装"约等于**什么都没准备**：`~/.dbmind` 只会多出库文件与 `agent-hosts`，
//! 其余目录与提示词模板全是"用到才建"。用户装完打开一看：
//! 一个只有四个文件的文件夹，像装坏了。
//!
//! ## 只做两件事（全部幂等，重复启动无副作用）
//!
//! 1. **把数据目录的骨架建齐**（drivers / exports / logs / prompts / work / backups）；
//! 2. 写下初始化标记（`app_settings.initializedAt`）—— 以后要做"仅首次"的事情时有个依据。
//!
//! ## 刻意不做的：塞示例数据
//!
//! 曾经放过一份示例 SQLite 库 + 一条示例连接（"装完就有东西可点"）。用户反馈**不要**：
//! 数据库客户端一打开就多出一条别人给的连接与几张陌生表，是打扰而不是帮助。
//! 界面空着就空着 —— 那是用户自己的库该有的样子。
//!
//! ## 一条硬规矩
//!
//! 只**新建缺失的目录**，绝不改动已有的任何数据（连接、历史、设置都不碰）。

use crate::error::Result;
use crate::paths;
use crate::storage::Store;
use std::path::{Path, PathBuf};

/// 初始化完成的标记（写在 `app_settings` 里）。
pub const SETTINGS_INITIALIZED_AT: &str = "app.initializedAt";

/// 初始化做了什么（给启动日志用）。
#[derive(Debug, Default, Clone)]
pub struct InitReport {
    /// 这次真正新建出来的目录名。
    pub created_dirs: Vec<String>,
}

impl InitReport {
    /// 有没有实际动作（没有就别打日志，免得每次启动都刷一行）。
    pub fn is_empty(&self) -> bool {
        self.created_dirs.is_empty()
    }
}

/// 数据目录骨架：`(展示名, 路径)`。
///
/// `work/tmp` 与 `work/restore` 是同步导入与备份还原用的临时区，
/// 提前建好省得那些路径各自 `ensure_dir` 一遍。
fn layout(home: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("drivers", home.join("drivers")),
        ("exports", home.join("exports")),
        ("logs", home.join("logs")),
        ("prompts", home.join("prompts")),
        ("work/tmp", home.join("work").join("tmp")),
        ("work/restore", home.join("work").join("restore")),
        ("backups", home.join("backups")),
    ]
}

/// 把数据目录骨架建齐，返回**这次新建**的目录名。
fn ensure_layout_in(home: &Path) -> Result<Vec<String>> {
    let mut created = Vec::new();
    for (name, dir) in layout(home) {
        if !dir.is_dir() {
            paths::ensure_dir(&dir)?;
            created.push(name.to_string());
        }
    }
    Ok(created)
}

/// 首次运行初始化。内存库（测试、临时会话）直接跳过。
pub fn ensure_first_run(store: &Store) -> Result<InitReport> {
    ensure_first_run_in(store, &paths::home_dir())
}

/// 同上，但数据目录由调用方给（只为可测：测试用临时目录，不去碰真实的 `~/.dbmind`）。
fn ensure_first_run_in(store: &Store, home: &Path) -> Result<InitReport> {
    let mut report = InitReport::default();
    if is_memory(store) {
        return Ok(report);
    }
    report.created_dirs = ensure_layout_in(home)?;

    // 标记在 → 初始化过了，不必再写一遍
    if store.get_setting(SETTINGS_INITIALIZED_AT)?.is_some() {
        return Ok(report);
    }
    store.set_setting(SETTINGS_INITIALIZED_AT, &crate::storage::now_iso())?;
    Ok(report)
}

/// 内存库判断：它没有对应的磁盘目录，也不该去动 `~/.dbmind`。
fn is_memory(store: &Store) -> bool {
    store.path().to_string_lossy() == ":memory:"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ConnectionConfig;
    use crate::ConnectionKind;

    /// 每个用例一个独立临时目录（不碰真实的 ~/.dbmind）。
    struct TempHome(PathBuf);

    impl TempHome {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("dbmind-bootstrap-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn open_store(home: &Path) -> Store {
        Store::open(&home.join("dbmind.db")).unwrap()
    }

    #[test]
    fn fresh_home_gets_layout_only() {
        let temp = TempHome::new("fresh");
        let home = temp.0.as_path();
        let store = open_store(home);

        let report = ensure_first_run_in(&store, home).unwrap();
        assert!(!report.created_dirs.is_empty(), "应该建出目录骨架");
        for dir in ["drivers", "exports", "logs", "prompts", "work", "backups"] {
            assert!(home.join(dir).is_dir(), "{dir} 应该被建出来");
        }
        assert!(home.join("work").join("tmp").is_dir());
        // 关键：不塞任何示例数据，也不建 samples 目录
        assert!(store.list_connections().unwrap().is_empty(), "不该有任何连接");
        assert!(!home.join("samples").exists(), "不该建 samples 目录");
        assert!(store
            .get_setting(SETTINGS_INITIALIZED_AT)
            .unwrap()
            .is_some());

        // 再跑一次：什么都不该变（幂等）
        let again = ensure_first_run_in(&store, home).unwrap();
        assert!(again.is_empty(), "第二次不应该有任何动作");
    }

    #[test]
    fn existing_data_is_never_touched() {
        let temp = TempHome::new("existing");
        let home = temp.0.as_path();
        let store = open_store(home);
        // 模拟已经有用户数据的库（老版本升级上来，没有初始化标记）
        store
            .insert_connection(&ConnectionConfig {
                name: "我的库".to_string(),
                kind: ConnectionKind::Sqlite,
                host: None,
                port: None,
                database: None,
                username: None,
                password: None,
                file_path: Some(home.join("mine.db").display().to_string()),
                color: None,
                extra: None,
            })
            .unwrap();

        ensure_first_run_in(&store, home).unwrap();
        let connections = store.list_connections().unwrap();
        assert_eq!(connections.len(), 1, "只该有他自己那一条");
        assert_eq!(connections[0].name(), "我的库");
        // 标记补上、目录也建齐，但数据一个字节都不动
        assert!(store
            .get_setting(SETTINGS_INITIALIZED_AT)
            .unwrap()
            .is_some());
        assert!(home.join("exports").is_dir());
    }
}
