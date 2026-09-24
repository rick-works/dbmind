//! DBMind HTTP 壳（axum）。
//!
//! ## 两套契约，各占各的前缀
//!
//! - `/api/dbmind/…` —— **内核原生契约**（约 20 个端点，`{code,message,detail,position}` 错误体，
//!   按错误码分支）。CLI / 桌面壳 / 冒烟脚本按它工作，保持稳定。
//! - `/api/…` —— **上游兼容契约**（见 `api` 模块）。上游的原版前端拼的就是这个前缀，
//!   所以它必须由兼容层独占：同一个 URL 有两个实现时，「到底哪个生效」只能靠运气。
//!
//! 三条约定：
//! 1. 业务逻辑全部转发给内核，壳里**不写任何 SQL 解析与权限判断**；
//! 2. 内核调用是阻塞的（SQLite + 文件 IO），一律走 `spawn_blocking`，避免堵住异步运行时；
//! 3. 错误统一回 `ErrorPayload`（错误码 + 分类 + 位置），HTTP 状态码由错误码映射，
//!    于是前端可以按码分支，而不是匹配文案。
//!
//! 之所以拆成 lib + 薄 bin：路由可被集成测试直接构造，命令行只是启动参数解析。

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use dbmind_core::{AccessContext, ConnectionConfig, DbMindEngine, DbMindError, ErrorCode, QueryRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

pub mod api;

#[derive(Clone)]
pub struct AppState {
    engine: Arc<DbMindEngine>,
    /// 异步任务表（导出 / 导入 / 整库转储 / 对比 / 同步 / 造数 共用一份）。
    ///
    /// 放在壳层而不是内层：内核只负责「执行一条语句」，没有「任务」这个概念；
    /// 而「任务」是界面语义（进度、取消、产物下载），属于传输层的事。
    pub tasks: Arc<crate::api::tasks::TaskRegistry>,
}

impl AppState {
    pub fn new(engine: DbMindEngine) -> Self {
        Self {
            engine: Arc::new(engine),
            tasks: Arc::new(crate::api::tasks::TaskRegistry::new()),
        }
    }
}

/// 统一错误响应。
pub struct ApiError(DbMindError);

impl From<DbMindError> for ApiError {
    fn from(err: DbMindError) -> Self {
        Self(err)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        Self(DbMindError::from(err))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if status.is_server_error() {
            // 只有 5xx 记细节；4xx 属于调用方可修正的输入
            tracing::error!(code = self.0.code_str(), detail = ?self.0.detail, "请求失败");
        }
        (status, Json(self.0.payload())).into_response()
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

/// 把阻塞的内核调用挪到 blocking 线程池。
async fn blocking<T, F>(task: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> dbmind_core::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| {
            ApiError(DbMindError::new(ErrorCode::Internal, "工作线程异常").with_detail(e.to_string()))
        })?
        .map_err(ApiError)
}

// ------------------------------------------------------------------ 处理器

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "version": dbmind_core::VERSION }))
}

async fn runtime_summary(State(state): State<AppState>) -> ApiResult {
    Ok(Json(serde_json::to_value(state.engine.runtime_summary())?))
}

async fn list_types(State(state): State<AppState>) -> ApiResult {
    Ok(Json(serde_json::to_value(state.engine.types())?))
}

/// 驱动与 agent 宿主就绪状态。
///
/// 单独一个端点（而不塞进 /types）：前端据此把「未接入」和「缺驱动」分开显示，
/// 并在缺驱动时给出坐标与安装命令。
async fn list_drivers(State(state): State<AppState>) -> ApiResult {
    let engine = state.engine.clone();
    let report = blocking(move || Ok(engine.driver_report())).await?;
    Ok(Json(serde_json::to_value(report)?))
}

/// 上传驱动的大小上限。**必须显式给**：axum 默认只收 2 MB，而驱动动辄十几 MB；
/// 但也不能不设上限 —— 那样任何请求都能把内存吃光。
const DRIVER_UPLOAD_LIMIT: usize = 64 * 1024 * 1024;

/// 某个 agentKey 对应的 **JDBC** 坐标；找不到就是「这个 key 不合法，或它不需要驱动」。
///
/// 这一步同时是**入参校验**：agentKey 会进文件路径（`<home>/drivers/<agentKey>/`），
/// 这里只放行「类型注册表里真的存在、且真的需要 JDBC 驱动」的 key ⇒ 路径穿越无从下手。
fn jdbc_artifact_of(engine: &DbMindEngine, agent_key: &str) -> Result<String, ApiError> {
    engine
        .types()
        .iter()
        .filter(|entry| entry.agent_key == Some(agent_key))
        .filter_map(|entry| entry.kind.jdbc_artifact())
        .next()
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError(DbMindError::new(
                ErrorCode::DriverNotReady,
                format!("没有 JDBC 类型使用 agentKey={agent_key}"),
            ))
        })
}

/// **下载驱动**：按该 agentKey 的 Maven 坐标从 Maven Central 拉一个 jar 落进驱动目录。
///
/// 内核刻意不引网络依赖（见 CLI 里那段说明），所以下载在这里做；但**地址的拼法只在核心里**
/// （`driver_artifact_url`）—— CLI / 桌面 / Web 三处共用一份，免得哪天有一处先漂。
async fn fetch_driver(State(state): State<AppState>, Path(agent_key): Path<String>) -> ApiResult {
    let artifact = jdbc_artifact_of(&state.engine, &agent_key)?;
    let key = agent_key;
    let result = blocking(move || {
        let url = dbmind_core::driver_artifact_url(&artifact)?;
        let response = ureq::get(&url).call().map_err(|err| {
            DbMindError::new(ErrorCode::DriverNotReady, format!("下载失败：{err}")).with_detail(url.clone())
        })?;
        let mut bytes: Vec<u8> = Vec::new();
        std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)?;
        let jar = dbmind_core::install_driver_jar(&key, &dbmind_core::driver_jar_name(&artifact)?, &bytes)?;
        Ok(json!({
            "agentKey": key,
            "jar": jar.display().to_string(),
            "bytes": bytes.len(),
            "url": url,
        }))
    })
    .await?;
    Ok(Json(result))
}

#[derive(Deserialize)]
struct UploadQuery {
    filename: String,
}

/// **上传驱动**：请求体**就是 jar 的字节**（不走 multipart —— 少一层解析，
/// 前端 `fetch(url, { body: file })` 直接发），文件名走查询串。
///
/// 两条校验都在核心里：文件名（挡 `../`、路径分隔符、非 `.jar`）与空文件。
/// 这里只负责把「agentKey + 文件名 + 字节」原样交给它。
async fn upload_driver(
    State(state): State<AppState>,
    Path(agent_key): Path<String>,
    Query(params): Query<UploadQuery>,
    body: Bytes,
) -> ApiResult {
    jdbc_artifact_of(&state.engine, &agent_key)?;
    let filename = params.filename;
    let bytes = body.to_vec();
    let result = blocking(move || {
        let jar = dbmind_core::install_driver_jar(&agent_key, &filename, &bytes)?;
        Ok(json!({
            "agentKey": agent_key,
            "jar": jar.display().to_string(),
            "bytes": bytes.len(),
        }))
    })
    .await?;
    Ok(Json(result))
}

async fn list_connections(State(state): State<AppState>) -> ApiResult {
    let engine = state.engine.clone();
    Ok(Json(serde_json::to_value(
        blocking(move || engine.list_connections()).await?,
    )?))
}

async fn create_connection(State(state): State<AppState>, Json(config): Json<ConnectionConfig>) -> ApiResult {
    let engine = state.engine.clone();
    let record = blocking(move || engine.add_connection(config)).await?;
    Ok(Json(serde_json::to_value(record)?))
}

async fn update_connection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(config): Json<ConnectionConfig>,
) -> ApiResult {
    let engine = state.engine.clone();
    let record = blocking(move || engine.update_connection(&id, config)).await?;
    Ok(Json(serde_json::to_value(record)?))
}

async fn delete_connection(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let engine = state.engine.clone();
    let removed = blocking(move || engine.remove_connection(&id)).await?;
    Ok(Json(json!({ "removed": removed })))
}

async fn test_connection(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let engine = state.engine.clone();
    let report = blocking(move || engine.test_connection(&id)).await?;
    Ok(Json(serde_json::to_value(report)?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadOnlyBody {
    read_only: bool,
}

async fn set_read_only(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ReadOnlyBody>,
) -> ApiResult {
    let engine = state.engine.clone();
    let record = blocking(move || engine.set_read_only(&id, body.read_only)).await?;
    Ok(Json(serde_json::to_value(record)?))
}

async fn execute(State(state): State<AppState>, Json(request): Json<QueryRequest>) -> ApiResult {
    let engine = state.engine.clone();
    // 先把 executionId 定下来：调用方拿得到它，取消才有依据
    let execution_id = request
        .execution_id
        .clone()
        .unwrap_or_else(DbMindEngine::next_execution_id);
    let request = request.with_execution_id(execution_id);
    let result = blocking(move || engine.execute(request, AccessContext::Web)).await?;
    Ok(Json(serde_json::to_value(result)?))
}

async fn cancel_execution(State(state): State<AppState>, Path(execution_id): Path<String>) -> Json<Value> {
    let cancelled = state.engine.cancel(&execution_id);
    Json(json!({ "executionId": execution_id, "cancelled": cancelled }))
}

async fn active_executions(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "active": state.engine.active_executions() }))
}

#[derive(Deserialize)]
struct RefreshQuery {
    /// `?refresh=true` 绕过结构缓存（界面上的「刷新」按钮）
    #[serde(default)]
    refresh: bool,
}

#[derive(Deserialize)]
struct TableQuery {
    table: String,
    #[serde(default)]
    refresh: bool,
}

async fn list_tables(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<RefreshQuery>,
) -> ApiResult {
    let engine = state.engine.clone();
    let tables = blocking(move || {
        if params.refresh {
            engine.list_tables_fresh(&id)
        } else {
            engine.list_tables(&id)
        }
    })
    .await?;
    Ok(Json(serde_json::to_value(tables)?))
}

async fn list_columns(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<TableQuery>,
) -> ApiResult {
    let engine = state.engine.clone();
    let columns = blocking(move || {
        if params.refresh {
            engine.list_columns_fresh(&id, &params.table)
        } else {
            engine.list_columns(&id, &params.table)
        }
    })
    .await?;
    Ok(Json(serde_json::to_value(columns)?))
}

/// 结构缓存概览：界面据此显示「缓存于 N 秒前」，避免用户以为结构一定是实时的。
async fn schema_cache_info(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let engine = state.engine.clone();
    let info = blocking(move || engine.schema_cache_info(&id)).await?;
    Ok(Json(serde_json::to_value(info)?))
}

/// 手工作废缓存（界面上的「清缓存」）。
async fn clear_schema_cache(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let engine = state.engine.clone();
    let cleared = blocking(move || engine.clear_schema_cache(&id)).await?;
    Ok(Json(json!({ "cleared": cleared })))
}

#[derive(Deserialize)]
struct HistoryQuery {
    #[serde(default = "default_history_limit")]
    limit: usize,
    connection: Option<String>,
}

fn default_history_limit() -> usize {
    50
}

async fn list_history(State(state): State<AppState>, Query(params): Query<HistoryQuery>) -> ApiResult {
    let engine = state.engine.clone();
    let history = blocking(move || engine.history(params.limit, params.connection.as_deref())).await?;
    Ok(Json(serde_json::to_value(history)?))
}

async fn clear_history(State(state): State<AppState>) -> ApiResult {
    let engine = state.engine.clone();
    let cleared = blocking(move || engine.clear_history()).await?;
    Ok(Json(json!({ "cleared": cleared })))
}

async fn list_settings(State(state): State<AppState>) -> ApiResult {
    let engine = state.engine.clone();
    let settings = blocking(move || engine.settings()).await?;
    let map: serde_json::Map<String, Value> =
        settings.into_iter().map(|(k, v)| (k, Value::String(v))).collect();
    Ok(Json(Value::Object(map)))
}

#[derive(Deserialize)]
struct SettingBody {
    value: String,
}

async fn put_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<SettingBody>,
) -> ApiResult {
    let engine = state.engine.clone();
    let value = body.value;
    // 闭包会拿走所有权，故先复制一份给任务，原值留给响应
    let task_key = key.clone();
    let task_value = value.clone();
    blocking(move || engine.set_setting(&task_key, &task_value)).await?;
    Ok(Json(json!({ "key": key, "value": value })))
}

// ------------------------------------------------------------------ 装配

/// 组装路由。`dist` 存在时一并托管前端产物（生产单进程）。
pub fn build_router(state: AppState, dist: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/runtime", get(runtime_summary))
        .route("/types", get(list_types))
        .route("/drivers", get(list_drivers))
        // 下载驱动：按 agentKey 的 Maven 坐标从中央仓拉（地址拼法在核心里）
        .route("/drivers/{agent_key}/fetch", post(fetch_driver))
        // 上传驱动：请求体是 jar 字节，文件名走 ?filename=；单独放大 body 上限
        .route(
            "/drivers/{agent_key}/upload",
            post(upload_driver).layer(DefaultBodyLimit::max(DRIVER_UPLOAD_LIMIT)),
        )
        .route("/connections", get(list_connections).post(create_connection))
        .route(
            "/connections/{id}",
            put(update_connection).delete(delete_connection),
        )
        .route("/connections/{id}/test", post(test_connection))
        .route("/connections/{id}/read-only", post(set_read_only))
        .route("/connections/{id}/tables", get(list_tables))
        .route("/connections/{id}/columns", get(list_columns))
        .route(
            "/connections/{id}/schema-cache",
            get(schema_cache_info).delete(clear_schema_cache),
        )
        .route("/query", post(execute))
        .route("/query/{execution_id}/cancel", post(cancel_execution))
        .route("/executions", get(active_executions))
        .route("/history", get(list_history).delete(clear_history))
        .route("/settings", get(list_settings))
        .route("/settings/{key}", put(put_setting));

    //上游兼容层独占 /api（原版前端一行不改地打过来）；
    // 内核原生契约挪到 /api/dbmind，避免同一个 URL 有两个实现。
    let router = Router::new()
        .merge(api::routes())
        .nest("/api/dbmind", api);

    if let Some(dir) = &dist {
        if dir.is_dir() {
            tracing::info!(path = %dir.display(), "托管前端产物");
        } else {
            tracing::warn!(path = %dir.display(), "前端产物目录不存在，仅提供 API");
        }
    }

    // 统一的回落策略见 `spa_fallback` 的注释：**/api 之下未命中必须报错，不能回落成 HTML**。
    let fallback_dist = dist;
    router
        .fallback(move |uri: axum::http::Uri| {
            let dist = fallback_dist.clone();
            async move { spa_fallback(dist, uri).await }
        })
        // 开发期前端由 vite 提供（不同端口），故放开 CORS
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// 未命中任何路由时的处理。
///
/// 分两半，这个分法很重要：
///
/// - `/api/**` 未命中 ⇒ **404 + JSON 说明**。早先直接交给 SPA 回落，于是「接口不存在」
///   会返回 `index.html`（HTTP 200 + 一坨 HTML），前端拿它去 `JSON.parse` 只会得到
///   「请求失败」——排查方向被彻底带偏。接口不存在就是接口不存在。
/// - 其余路径 ⇒ 前端产物；文件不存在就回落 `index.html`（Vue Router 的 history 模式要靠它）。
async fn spa_fallback(dist: Option<PathBuf>, uri: axum::http::Uri) -> Response {
    let path = uri.path().to_string();
    if path == "/api" || path.starts_with("/api/") {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "message": format!("接口不存在：{path}"),
                "hint": "上游兼容接口在 /api/… 下；内核原生接口在 /api/dbmind/… 下",
            })),
        )
            .into_response();
    }

    let Some(dir) = dist else {
        return (
            StatusCode::NOT_FOUND,
            "未托管前端产物（启动时用 --dist 指定目录）",
        )
            .into_response();
    };

    let relative = path.trim_start_matches('/');
    // 挡目录穿越：`..` 一律拒绝，不让 URL 逃出产物目录
    if !relative.is_empty() && !relative.contains("..") {
        let file = dir.join(relative);
        if let Ok(bytes) = tokio::fs::read(&file).await {
            // 缓存策略：`assets/` 下的产物带内容哈希（内容变名字就变），可以长缓存；
            // 其余（如 index.html）必须每次回源校验 —— 否则改版后浏览器还在拿旧页面，
            // 用户会反复看到"改动没生效"（本项目实际发生过）。
            let immutable = relative.starts_with("assets/");
            let cache_control = if immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            return (
                [
                    (axum::http::header::CONTENT_TYPE, mime_of(&file)),
                    (axum::http::header::CACHE_CONTROL, cache_control),
                ],
                bytes,
            )
                .into_response();
        }
    }

    match tokio::fs::read(dir.join("index.html")).await {
        Ok(bytes) => (
            [
                (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
                // index.html 不缓存：让浏览器每次都校验，拿到最新构建的引用
                (axum::http::header::CACHE_CONTROL, "no-cache"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "前端产物缺失：找不到 index.html").into_response(),
    }
}

fn mime_of(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub struct Options {
    pub host: String,
    pub port: u16,
    pub store: Option<PathBuf>,
    pub dist: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            // 默认沿用上游的历史端口：前端的 dev 代理（vite.config.mjs）指向 20361，
            // 后端跟着它走，前端就**一行都不用改**。
            port: 20361,
            store: None,
            dist: None,
        }
    }
}

/// 启动 HTTP 壳（带优雅退出）。
pub async fn serve(options: Options) -> Result<(), String> {
    let engine = match &options.store {
        Some(path) => DbMindEngine::open(path),
        None => DbMindEngine::open_default(),
    }
    .map_err(|err| format!("无法打开元数据库: {err}"))?;

    // 让"环境式"的读写（AI 设置这类不接 Store 参数的地方）能拿到主库；
    // 再把老版本散在 ~/.dbmind 下的 json 状态一次性导进来。顺序有讲究：先装句柄，迁移才有地方写。
    dbmind_core::install_global_store(engine.store().clone());
    let imported = crate::api::ai::migrate::import_legacy();
    if imported > 0 {
        tracing::info!(actions = imported, "已把旧版 json 状态导入主库");
    }

    let summary = engine.runtime_summary();
    tracing::info!(
        store = %summary.store_path,
        connections = summary.connection_count,
        drivers = format!("{}/{}", summary.implemented_types, summary.declared_types),
        "DBMind 内核就绪"
    );

    let router = build_router(AppState::new(engine), options.dist.clone());
    let addr: SocketAddr = format!("{}:{}", options.host, options.port)
        .parse()
        .map_err(|e| format!("非法监听地址: {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("无法监听 {addr}: {e}"))?;
    tracing::info!(%addr, "HTTP 壳已启动");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("服务异常退出: {e}"))
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("收到中断信号，准备退出");
}

/// 找前端产物目录。
///
/// 桌面壳**自己起内核**时，页面就是从这个服务取的，所以它必须托管前端 —— 以前这里是写死的
/// `None`，于是"自己起内核"的桌面版打开就是一张「未托管前端产物」的白页
/// （只有恰好复用了外部带 `--dist` 的 `dbmind-web.exe` 时才看不出来，所以一直没暴露）。
///
/// 顺序：`DBMIND_DIST` 环境变量（显式指定最优先）→ **exe 旁的 `web/`**
/// （便携版与安装包的布局，二者都把产物放在可执行文件旁边）→ 开发态沿 `target/<profile>/`
/// 向上找仓库里的 `frontend/dist`。
fn detect_dist() -> Option<PathBuf> {
    let is_dist = |dir: &PathBuf| dir.join("index.html").is_file();

    if let Some(explicit) = std::env::var_os("DBMIND_DIST") {
        let dir = PathBuf::from(explicit);
        if is_dist(&dir) {
            return Some(dir);
        }
    }

    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();

    let beside = dir.join("web");
    if is_dist(&beside) {
        return Some(beside);
    }

    // 开发态：exe 在 target/debug 或 target/release 下，往上两三层就是仓库根
    for depth in ["../../", "../../../", "../../../../"] {
        let candidate = dir.join(depth).join("frontend/dist");
        if is_dist(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// 在后台线程启动内嵌 HTTP 壳。
///
/// 桌面壳（Tauri）用它：**前端仍然通过 HTTP 访问同一个内核**，因此
/// 桌面版与 Web 版共享同一份前端代码、同一套错误结构、同一份契约，
/// 而不是给桌面单独发明一套 IPC 命令。数据库客户端的痛点从来不是传输方式，
/// 而是「两套实现行为不一致」。
pub fn spawn_embedded(port: u16, store: Option<PathBuf>) -> Result<(), String> {
    // 前端产物在起线程之前定下来，好把结果记进日志（找不到时那句 warn 是最有用的线索）
    let dist = detect_dist();
    match &dist {
        Some(dir) => tracing::info!(path = %dir.display(), "内嵌壳将同时托管前端产物"),
        None => tracing::warn!("没找到前端产物：内嵌壳只提供 API，页面会是白页（可用 DBMIND_DIST 指定）"),
    }
    let handle = std::thread::Builder::new()
        .name("dbmind-http".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(err) => {
                    tracing::error!("创建内嵌运行时失败: {err}");
                    return;
                }
            };
            runtime.block_on(async move {
                let options = Options {
                    host: "127.0.0.1".to_string(),
                    port,
                    store,
                    dist,
                };
                if let Err(err) = serve(options).await {
                    tracing::error!("内嵌 HTTP 壳退出: {err}");
                }
            });
        })
        .map_err(|e| format!("启动内嵌 HTTP 壳失败: {e}"))?;
    let _ = handle;
    Ok(())
}
