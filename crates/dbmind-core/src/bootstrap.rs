//! 首次运行的初始化。
//!
//! ## 为什么要它
//!
//! 以前"初次安装"约等于**什么都没准备**：`~/.dbmind` 只会多出库文件与 `agent-hosts`，
//! 其余目录、提示词模板、示例数据全是"用到才建"。用户装完打开一看：
//! 一个只有四个文件的文件夹、界面里一条连接都没有 —— 像装坏了。
//!
//! ## 做三件事（全部幂等，重复启动无副作用）
//!
//! 1. **把数据目录的骨架建齐**（drivers / exports / logs / prompts / samples / work / backups）；
//! 2. **只在真正全新时才放示例数据**：一个示例 SQLite 库 + 一条指向它的连接，
//!    让用户装完就有东西可点开看；
//! 3. 写下初始化标记（`app_settings.initializedAt`）。
//!
//! ## 一条硬规矩：绝不往老用户的数据里塞东西
//!
//! 老版本升级上来的库里**没有**初始化标记，但它有用户自己的连接。所以判断"是否全新"
//! 不能只看标记，必须同时看**连接表是不是空的** —— 两个条件都满足才放示例。

use crate::error::Result;
use crate::paths;
use crate::storage::Store;
use crate::types::ConnectionConfig;
use crate::ConnectionKind;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// 初始化完成的标记（写在 `app_settings` 里）。
pub const SETTINGS_INITIALIZED_AT: &str = "app.initializedAt";

/// 示例库所在目录（跟着数据目录走，`DBMIND_HOME` 一改就一起搬）。
const SAMPLES_DIR: &str = "samples";
/// 示例库文件名。
const SAMPLE_DB: &str = "demo.db";
/// 示例连接在界面上的名字。带"示例"二字，用户一眼知道可以删。
const SAMPLE_CONNECTION: &str = "示例数据库（SQLite）";

/// 初始化做了什么（给启动日志用）。
#[derive(Debug, Default, Clone)]
pub struct InitReport {
    /// 这次真正新建出来的目录名。
    pub created_dirs: Vec<String>,
    /// 是否放了示例库与示例连接。
    pub seeded_sample: bool,
}

impl InitReport {
    /// 有没有实际动作（没有就别打日志，免得每次启动都刷一行）。
    pub fn is_empty(&self) -> bool {
        self.created_dirs.is_empty() && !self.seeded_sample
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
        (SAMPLES_DIR, home.join(SAMPLES_DIR)),
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

    // 标记在 → 初始化过了，别再动用户的数据
    if store.get_setting(SETTINGS_INITIALIZED_AT)?.is_some() {
        return Ok(report);
    }

    // 没有标记：可能真是全新，也可能是老版本升级上来（老版本不写这个标记）。
    // 用"连接表空不空"来区分 —— 老用户有自己的连接，就只补标记、不放示例。
    if store.list_connections()?.is_empty() {
        let sample = home.join(SAMPLES_DIR).join(SAMPLE_DB);
        write_sample_database(&sample)?;
        store.insert_connection(&ConnectionConfig {
            name: SAMPLE_CONNECTION.to_string(),
            kind: ConnectionKind::Sqlite,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: Some(sample.display().to_string()),
            color: Some("#3ddc97".to_string()),
            extra: None,
        })?;
        report.seeded_sample = true;
    }

    store.set_setting(SETTINGS_INITIALIZED_AT, &crate::storage::now_iso())?;
    Ok(report)
}

/// 建一个可以直接点开看的示例库（两张表 + 几行数据）。
///
/// 用内核自带的 rusqlite 直接建文件：不需要驱动、不需要联网、不需要任何连接，
/// 所以哪怕离线装完也能立刻看到东西。重复调用不会重复插数据。
fn write_sample_database(path: &Path) -> Result<()> {
    paths::ensure_parent(path)?;
    let conn = Connection::open(path).map_err(|e| {
        crate::error::DbMindError::new(
            crate::error::ErrorCode::StorageFailed,
            format!("创建示例库失败：{}", path.display()),
        )
        .with_detail(e.to_string())
    })?;
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS customers (
    id     INTEGER PRIMARY KEY,
    name   TEXT    NOT NULL,
    level  INTEGER NOT NULL DEFAULT 1,
    region TEXT
);

CREATE TABLE IF NOT EXISTS orders (
    id          INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id),
    product     TEXT    NOT NULL,
    amount      REAL    NOT NULL,
    created_at  TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_orders_customer ON orders(customer_id);
"#,
    )
    .map_err(sample_error)?;

    // 只在还没有数据时插一次（幂等）
    let existing: i64 = conn
        .query_row("SELECT COUNT(*) FROM customers", [], |row| row.get(0))
        .map_err(sample_error)?;
    if existing == 0 {
        conn.execute_batch(
            r#"
INSERT INTO customers (id, name, level, region) VALUES
    (1, '华东贸易', 3, '上海'),
    (2, '北方物流', 2, '北京'),
    (3, '南方零售', 1, '广州');

INSERT INTO orders (id, customer_id, product, amount, created_at) VALUES
    (1, 1, '标准版授权', 12800.00, '2026-09-01'),
    (2, 1, '技术支持年费', 3600.00,  '2026-09-05'),
    (3, 2, '标准版授权', 12800.00, '2026-09-11'),
    (4, 3, '基础版授权', 4800.00,  '2026-09-18');
"#,
        )
        .map_err(sample_error)?;
    }
    Ok(())
}

fn sample_error(e: rusqlite::Error) -> crate::error::DbMindError {
    crate::error::DbMindError::new(
        crate::error::ErrorCode::StorageFailed,
        "初始化示例数据库失败",
    )
    .with_detail(e.to_string())
}

/// 内存库判断：它没有对应的磁盘目录，也不该去动 `~/.dbmind`。
fn is_memory(store: &Store) -> bool {
    store.path().to_string_lossy() == ":memory:"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ConnectionConfig;

    /// 每个用例一个独立临时目录（不碰真实的 ~/.dbmind）。
    struct TempHome(PathBuf);

    impl TempHome {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "dbmind-bootstrap-{tag}-{}",
                std::process::id()
            ));
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
    fn fresh_home_gets_layout_and_sample() {
        let temp = TempHome::new("fresh");
        let home = temp.0.as_path();
        let store = open_store(home);

        let report = ensure_first_run_in(&store, home).unwrap();
        assert!(report.seeded_sample, "全新库应该放示例数据");
        assert!(!report.created_dirs.is_empty(), "应该建出目录骨架");
        assert!(home.join("samples").join("demo.db").is_file(), "示例库文件应存在");
        assert!(home.join("prompts").is_dir());
        assert!(home.join("work").join("tmp").is_dir());

        // 示例连接确实进库了，而且指向那个示例库
        let connections = store.list_connections().unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].name(), SAMPLE_CONNECTION);
        assert!(connections[0]
            .config
            .file_path
            .as_deref()
            .unwrap_or_default()
            .ends_with("demo.db"));

        // 再跑一次：标记挡住了，什么都不该变（幂等）
        let again = ensure_first_run_in(&store, home).unwrap();
        assert!(again.is_empty(), "第二次不应该有任何动作");
        assert_eq!(store.list_connections().unwrap().len(), 1, "不能重复塞示例连接");
    }

    #[test]
    fn existing_user_never_gets_sample_data() {
        let temp = TempHome::new("existing");
        let home = temp.0.as_path();
        let store = open_store(home);
        // 模拟"老版本升级上来"：库里有用户自己的连接，但没有初始化标记
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

        let report = ensure_first_run_in(&store, home).unwrap();
        assert!(!report.seeded_sample, "老用户绝不能被塞示例连接");
        let connections = store.list_connections().unwrap();
        assert_eq!(connections.len(), 1, "只该有他自己那一条");
        assert_eq!(connections[0].name(), "我的库");
        // 但标记要补上，且目录骨架仍然建齐
        assert!(store.get_setting(SETTINGS_INITIALIZED_AT).unwrap().is_some());
        assert!(home.join("exports").is_dir());
    }
}
