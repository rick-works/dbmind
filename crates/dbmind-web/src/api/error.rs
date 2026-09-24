//!上游兼容层的错误出口。
//!
//!上游前端的 axios 拦截器只认响应体里的 `message`（见 `frontend/src/api/index.js`），
//! 所以错误体统一是 `{ success: false, message: "..." }`；HTTP 状态码再按语义给出，
//! 让「接口不存在 / 参数不对 / 没实现」在浏览器调试面板里一眼可分。
//!
//! 与内核原生层的 `ApiError`（`{code, message, detail, position}`）刻意分开：
//! 那是给按错误码分支的调用方（CLI/桌面）用的，这里要对齐的是上游的形状。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use dbmind_core::{DbMindError, ErrorCode};
use serde_json::json;

#[derive(Debug)]
pub struct XError {
    pub status: StatusCode,
    pub message: String,
}

impl XError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    /// 能力尚未接入：501 + 明确文案。
    ///
    /// **不许静默返回空数组** —— 那会被界面读成「这个库里没有索引」，
    /// 而事实是「我们还没做这件事」，两种坏消息必须分清。
    pub fn not_implemented(feature: &str) -> Self {
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            format!("「{feature}」在 DBMind 内核里还没有对应实现"),
        )
    }
}

impl From<DbMindError> for XError {
    fn from(err: DbMindError) -> Self {
        let status = StatusCode::from_u16(err.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut message = with_hint(&err.message);
        if let Some(detail) = &err.detail {
            if !detail.is_empty() {
                message.push('\n');
                message.push_str(detail);
            }
        }
        Self { status, message }
    }
}

/// 给内核里几条「字面上正确、但用户不知道该做什么」的错误补一句可操作的提示。
///
/// 典型例子：`连接「x」缺少 {database} 对应的字段` —— 这句话把 JDBC URL 模板里的占位符
/// 原样甩给了用户。他刚在界面上填过数据库名（甚至能看到右边就写着名字），却被告知缺字段，
/// 第一反应一定是「我填错了？」。补一句「填哪个框」能省掉一整轮排查。
pub fn with_hint(message: &str) -> String {
    let hint = if message.contains("{database}") {
        Some("请在连接里填写「数据库名称」（SQL Server / PostgreSQL / KingbaseES 的 JDBC URL 里需要它；SQL Server 可先填 master）")
    } else if message.contains("{filePath}") {
        Some("请在连接里选择本地数据库文件路径")
    } else if message.contains("{host}") || message.contains("{port}") {
        Some("请填写「主机」与「端口」")
    } else if message.contains("{username}") {
        Some("请在连接里填写「用户名」")
    } else {
        None
    };
    match hint {
        Some(hint) => format!("{message}。→ {hint}"),
        None => message.to_string(),
    }
}

/// 字符串一律当「内部错误」。
///
/// 存在的理由很实际：写入器（导出 sink）、任务工作体这些地方返回的是
/// `Result<_, String>`（它们不该认识 HTTP），有了这条，`?` 就能直接进 `XResult`，
/// 不必在每个调用点套一层 `map_err`。
impl From<String> for XError {
    fn from(message: String) -> Self {
        Self::internal(message)
    }
}

impl From<serde_json::Error> for XError {
    fn from(err: serde_json::Error) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("序列化失败：{err}"),
        )
    }
}

impl From<std::io::Error> for XError {
    fn from(err: std::io::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, format!("IO 失败：{err}"))
    }
}

impl From<ErrorCode> for XError {
    fn from(code: ErrorCode) -> Self {
        Self::new(
            StatusCode::from_u16(code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            code.as_str(),
        )
    }
}

impl IntoResponse for XError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(status = %self.status, message = %self.message, "上游兼容层请求失败");
        }
        (self.status, Json(json!({ "success": false, "message": self.message }))).into_response()
    }
}

pub type XResult<T> = Result<T, XError>;
