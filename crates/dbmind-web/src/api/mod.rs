//! **上游兼容 REST 层** —— 让上游的原版前端跑在 DBmind 内核上。
//!
//! ## 为什么要有这一层
//!
//! 本项目的目标形态是：**前端一行不改**（连 `src/api/index.js` 都是上游的原样），
//! 后端由 DBmind 内核提供能力。两侧的模型不同，所以中间必须有且只有一层翻译：
//!
//! | |上游后端 | 本项目 |
//! |---|---|---|
//! | 语言/形态 | Spring Boot 多模块，每个库一个模块 | Rust 单进程，数据驱动连接类型 |
//! | 接口粒度 | 506 个具体 URL，元数据/DDL/同步/对比/备份/AI 各有专接口 | 内核只有「执行一条语句」+ 少量通用端点 |
//! | 元数据 | 各方言模块按类型实现 | 能走内核走内核，其余在兼容层按方言拼 SQL |
//!
//! 所以这一层做的事就是：**把上游的 506 个 URL 映射到内核能力上**，
//! 映射不到的地方**明确报错**（501 + 一句人话），绝不静默返回空数组 ——
//! 后者会被界面读成「这个库里没有索引」，而事实是「我们还没做这件事」。
//!
//! ## 路由的两段结构
//!
//! - 内核原生契约（约 20 个端点）保留在 `/api/dbmind/…` 下（CLI / 桌面 / 测试脚本仍在用）；
//! - 本层占据 `/api/…`（上游前端拼的就是这个前缀）。
//!
//! 同一个 URL 只归一方所有，避免「两个 `/api/connections` 谁生效」这种只能靠运气的问题。

pub mod ai;
pub mod backup;
pub mod compare;
pub mod conn;
pub mod datagen;
pub mod dialect;
pub mod driver;
pub mod error;
pub mod export;
pub mod import;
pub mod meta;
pub mod monitor;
pub mod nosql;
pub mod query;
pub mod scope;
pub mod shape;
pub mod sync;
pub mod sys;
pub mod tasks;
pub mod xlsx;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::Json;
use serde_json::json;
use axum::Router;
use dbmind_core::ConnectionRecord;

use crate::AppState;
use error::{XError, XResult};

/// 把阻塞的内核调用挪到 blocking 线程池（内核是 SQLite + 文件 IO + 子进程，全是阻塞的）。
pub(crate) async fn blocking<T, F>(task: F) -> XResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> dbmind_core::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| XError::internal(format!("工作线程异常：{e}")))?
        .map_err(XError::from)
}

/// 取连接记录；不存在时给出 404 语义的错误（而不是一个空对象让界面继续往下走）。
pub async fn require_record(state: &AppState, id: &str) -> XResult<ConnectionRecord> {
    let engine = state.engine();
    let id = id.to_string();
    blocking(move || engine.require_connection(&id)).await
}

/// 查询串解析。
///
/// 不用 `serde_urlencoded` 的理由：上游会在 `getTableCounts` 里发**数组参数**
/// （axios 序列化成 `tables=a&tables=b` 或 `tables[]=a`），而它遇到重复键只会保留一个 ——
/// 结果就是「批量行数只回填了第一张表」，看起来只是「有些表没行数」，很难查到根上。
/// 自己按出现次数收集，两种写法都认。
pub struct Params(Vec<(String, String)>);

impl Params {
    pub fn parse(raw: Option<&str>) -> Self {
        let mut pairs = Vec::new();
        if let Some(raw) = raw {
            for chunk in raw.split('&') {
                if chunk.is_empty() {
                    continue;
                }
                let (key, value) = match chunk.split_once('=') {
                    Some((key, value)) => (key, value),
                    None => (chunk, ""),
                };
                pairs.push((percent_decode(key), percent_decode(value)));
            }
        }
        Self(pairs)
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    }

    /// 同名参数的全部取值（`tables=a&tables=b` 与 `tables[]=a&tables[]=b` 都认）。
    pub fn all(&self, key: &str) -> Vec<String> {
        let bracket = format!("{key}[]");
        self.0
            .iter()
            .filter(|(k, _)| *k == key || *k == bracket)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
            .collect()
    }
}

fn percent_decode(value: &str) -> String {
    if !value.contains('%') && !value.contains('+') {
        return value.to_string();
    }
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// 说明：原先这里有一组「尚未接入」的 501 占位（compare / sync / backup / datagen / AI）。
// 它们的角色是**在实现之前先把入口占住**，让界面拿到「还没做」而不是「地址写错了」。
// 随着各域逐个落地，占位已经全部换成了真实 handler —— 只剩 AI 内部尚未落地的子域
// （知识库 / 团队知识 / 治理与质量 / 智能体），那些在 `ai::routes()` 里按前缀注册了 501。

/// 关系型模块的全部路由（9 个模块前缀共用同一批 handler：方言差异由连接类型决定，
/// 与 URL 前缀无关 —— 这正是「同一个实现服务多种类型」的好处）。
fn relational_module(module: &str) -> Router<AppState> {
    let p = |suffix: &str| format!("/api/{module}{suffix}");
    Router::new()
        .route(&p("/test"), post(meta::test))
        .route(&p("/{id}/features"), get(meta::features))
        .route(&p("/{id}/catalogs"), get(meta::catalogs))
        .route(&p("/{id}/databases"), get(meta::databases))
        .route(&p("/{id}/schemas"), get(meta::schemas))
        .route(&p("/{id}/tables"), get(meta::tables))
        .route(&p("/{id}/table-count"), get(meta::table_count))
        .route(&p("/{id}/stats"), get(meta::stats))
        .route(&p("/{id}/columns"), get(meta::columns))
        .route(&p("/{id}/column-comments"), get(meta::column_comments))
        .route(&p("/{id}/search-objects"), get(meta::search_objects))
        .route(&p("/{id}/data"), get(meta::data))
        .route(&p("/{id}/data-save"), post(meta::data_save))
        .route(&p("/{id}/ddl"), get(meta::ddl))
        .route(&p("/{id}/monitor"), get(monitor::monitor))
        .route(&p("/{id}/monitor/kill"), post(monitor::monitor_kill))
        .route(&p("/{id}/indexes"), get(meta::indexes))
        .route(&p("/{id}/procedures"), get(meta::procedures))
        .route(&p("/{id}/triggers"), get(meta::triggers))
        .route(&p("/{id}/events"), get(meta::events))
        .route(&p("/{id}/users"), get(meta::users))
        .route(&p("/{id}/user-info"), get(meta::user_info))
        .route(&p("/{id}/user-action"), post(meta::user_action))
        .route(&p("/{id}/object-info"), get(meta::object_info))
        .route(&p("/{id}/alter"), post(meta::alter))
        .route(&p("/{id}/table-action"), post(meta::table_action))
        .route(&p("/{id}/rename-object"), post(meta::rename_object))
        .route(&p("/query/{id}"), post(query::execute))
        .route(&p("/query/{id}/batch"), post(query::execute_batch))
        .route(&p("/query/{id}/count"), post(query::count))
        .route(&p("/query/{id}/tx"), post(query::tx))
        .route(&p("/query/cancel/{executionId}"), post(query::cancel))
        .route(&p("/query/run-file-task/{id}"), post(query::run_file_start))
        .route(&p("/query/run-file-task/status/{taskId}"), get(query::run_file_status))
        .route(&p("/query/run-file-task/cancel/{taskId}"), post(query::run_file_cancel))
        // 导出：同步（当前页 / 整库）+ 异步（提交 / 进度 / 取消 / 下载）
        //
        // 注意 `/export/task/{id}` 的提交与进度**必须合并注册**：matchit 不允许同一个
        // 路径位置挂两个不同的参数名（`{id}` 与 `{taskId}` 会被判定为冲突并直接 panic）。
        // 同一个路径、不同方法写在一个 `MethodRouter` 上，既避开了这条限制，也更贴合语义。
        .route(&p("/export/{id}"), post(export::single))
        .route(&p("/export/db/{id}"), post(export::db))
        .route(&p("/export/task/{id}"), post(export::task).get(export::status))
        .route(&p("/export/dump-task/{id}"), post(export::dump_task))
        .route(&p("/export/cancel/{taskId}"), post(export::cancel))
        .route(&p("/export/download/{taskId}"), get(export::download))
        // 导入：multipart 上传 → 异步写入 → 进度 / 取消
        // 上传体积单独放宽（默认 2 MB 连一个几十兆的 CSV 都收不下）
        .route(
            &p("/import/{id}"),
            post(import::start).layer(DefaultBodyLimit::max(import::IMPORT_LIMIT)),
        )
        .route(&p("/import/task/{id}"), get(import::status))
        .route(&p("/import/cancel/{taskId}"), post(import::cancel))
}

/// NoSQL 模块的路由（MongoDB / Redis / Elasticsearch）。
fn nosql_module(module: &str) -> Router<AppState> {
    let p = |suffix: &str| format!("/api/{module}{suffix}");
    Router::new()
        .route(&p("/test"), post(nosql::test))
    // NoSQL 类型没有 catalog 层级：返回空清单（前端展开连接时统一会请求一次，
    // 404 会被当成异常打进控制台 —— 空数组才是「没有这一层」的正确表达）
    .route(&p("/{id}/catalogs"), get(|| async { Json(json!([])) }))
        .route(&p("/{id}/databases"), get(nosql::databases))
        .route(&p("/{id}/collections"), get(nosql::collections))
        .route(&p("/{id}/documents"), get(nosql::documents))
        .route(&p("/{id}/collection"), delete(nosql::delete_collection))
        .route(&p("/{id}/execute"), post(nosql::execute))
        .route(&p("/{id}/monitor"), get(nosql::monitor))
        .route(&p("/cancel/{executionId}"), post(nosql::cancel))
}

/// 组装上游兼容路由（不含 state，由 `lib.rs` 统一 `with_state`）。
pub fn routes() -> Router<AppState> {
    let mut router = Router::new()
        .route("/api/connections", get(conn::list).post(conn::create))
        .route(
            "/api/connections/{id}",
            get(conn::get).put(conn::update).delete(conn::delete),
        )
        .route("/api/connections/{id}/copy", post(conn::copy))
        .route("/api/drivers/types", get(sys::driver_types))
        .route("/api/drivers/status", get(sys::driver_status))
        // 强制重新下载驱动（清掉失败缓存，用于重试；设置页「驱动下载」的「下载」按钮就用它）
        .route("/api/drivers/{code}/install", post(driver::install))
        // 手动上传驱动 jar（multipart，文件字段名 file，可多选）：
        // 离线 / 内网机器拉不到 Maven 时的唯一出路。体积单独放宽 —— 默认 2 MB 连一个驱动包都收不下。
        .route(
            "/api/drivers/{code}/upload",
            post(driver::upload).layer(DefaultBodyLimit::max(driver::DRIVER_UPLOAD_LIMIT)),
        )
        .route(
            "/api/settings/paths",
            get(sys::paths_get).put(sys::paths_put),
        )
        .route("/api/settings/open-dir", post(sys::open_dir))
        .route(
            "/api/settings/driver-mirror",
            get(sys::mirror_get).put(sys::mirror_put),
        )
        .route(
            "/api/settings/legacy-tls",
            get(sys::legacy_tls_get).put(sys::legacy_tls_put),
        )
        // 尚未接入的功能域：显式 501 + 说明（而不是 404 让界面只说「请求失败」）。
        // 注意 `{*rest}` 至少要有一段子路径：`/api/compare` 这种**没有子路径**的也要单独注册，
        // 否则它会落到 404（上游的 `POST /api/compare` 正是无子路径的那个）。
        .merge(ai::routes())
        // 造数：预览（同步，不写库）+ 生成（异步任务，可取消）
        .route("/api/datagen/preview", post(datagen::preview))
        .route("/api/datagen/start", post(datagen::start))
        .route("/api/datagen/task/{id}", get(datagen::status))
        .route("/api/datagen/cancel/{taskId}", post(datagen::cancel))
        // 对比：提交 → 进度 → 取消（单端点，连接 ID 自带类型信息）
        .route("/api/compare", post(compare::start))
        .route("/api/compare/task/{id}", get(compare::status))
        .route("/api/compare/cancel/{taskId}", post(compare::cancel))
        // 同步：单表（同步返回）+ 多对象（异步任务）+ 进度 + 取消
        .route("/api/sync", post(sync::table))
        .route("/api/sync/db", post(sync::db))
        .route("/api/sync/task/{id}", get(sync::status))
        .route("/api/sync/cancel/{taskId}", post(sync::cancel))
        // 备份 / 还原：命令行探测与安装引导 + 任务 + 下载
        .route("/api/backup/start", post(backup::start))
        .route("/api/backup/default-dir", get(backup::default_dir))
        .route("/api/backup/check-dir", post(backup::check_dir))
        .route("/api/backup/browse-dirs", get(backup::browse_dirs))
        .route("/api/backup/browse-files", get(backup::browse_files))
        .route("/api/backup/cli-guide", get(backup::cli_guide))
        .route("/api/backup/cli-tool-status", get(backup::cli_tool_status))
        .route("/api/backup/cli-tool-install", post(backup::cli_tool_install))
        .route("/api/backup/task/{id}", get(backup::task_status))
        .route("/api/backup/cancel/{id}", post(backup::cancel))
        .route("/api/backup/download/{id}", get(backup::download))
        // 任务中途的「自动安装 / 我已装好 / 跳过」三选
        .route(
            "/api/backup/task/{id}/{decision}",
            post(backup::install_decision),
        )
        .route("/api/backup/restore/start", post(backup::restore_start))
        .route(
            "/api/backup/restore/start-local",
            post(backup::restore_start_local),
        )
        .route("/api/backup/restore/task/{id}", get(backup::restore_status))
        .route("/api/backup/restore/cancel/{id}", post(backup::cancel))
        .route(
            "/api/backup/restore/task/{id}/{decision}",
            post(backup::install_decision),
        );
    // 说明：`/api/ai/kb/…` 已由 `/api/ai/{*rest}` 覆盖，不能再单独注册一条更具体的分支
    // （matchit 不允许在 catch-all 之下再挂静态段）。知识库的文案并在 AI 那条里说清。

    for module in dialect::RELATIONAL_MODULES {
        router = router.merge(relational_module(module));
    }
    for module in dialect::NOSQL_MODULES {
        router = router.merge(nosql_module(module));
    }
    router
}
