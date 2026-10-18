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
    /// 「该怎么做」的那句话。与 `detail` 成对出现时，界面把 `detail` 放上面、这句放下面。
    pub hint: Option<String>,
    /// 数据库 / 驱动返回的**原话**（例如 `SELECT command denied to user 'x' for table 'user'`）。
    /// 单独给出来是为了让界面能把它按等宽字体放在最上面：用户可以复制去搜，也不会
    /// 被"我们后来加的解释"盖住 —— 排错时第一眼看的就是这句。
    pub detail: Option<String>,
    /// 内核错误码（`DBMIND-CONN-0003`）。不发给前端（响应体保持 `{success,message}`），
    /// 只用来让中间层**分得清错因类别** —— 例如「库清单查询失败」要不要退回单库模式，
    /// 取决于那是"连不上"还是"连上了但这条查询不行"（见 `meta::databases`）。
    pub code: Option<String>,
}

impl XError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            hint: None,
            detail: None,
            code: None,
        }
    }

    /// 「原始错误在前、说明在后」的两段式错误。
    ///
    /// `message` 仍然是两者拼好的整句（老调用方、右上角通知、日志都靠它），
    /// `hint` / `detail` 只是把同一条信息拆开给界面排版用。
    pub fn with_detail(
        status: StatusCode,
        message: impl Into<String>,
        hint: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            status,
            message: message.into(),
            hint: Some(hint.into()),
            detail: Some(detail.into()),
            code: None,
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
            format!("「{feature}」在 DBmind 内核里还没有对应实现"),
        )
    }
}

impl From<DbMindError> for XError {
    fn from(err: DbMindError) -> Self {
        let status = StatusCode::from_u16(err.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut message = with_hint(&err.message);
        if let Some(detail) = &err.detail {
            let kept = human_detail(detail);
            // 剔掉的部分只进日志：排查时那条 `SQLRecoverableException, SQLState=08006…`
            // 仍然是唯一的线索，只是它不该出现在用户面前。
            if kept.trim() != detail.trim() {
                tracing::debug!(code = err.code_str(), detail = %detail, "错误详情（技术尾巴，仅日志）");
            }
            if !kept.trim().is_empty() {
                message.push('\n');
                message.push_str(kept.trim());
            }
        }
        Self {
            status,
            message,
            hint: None,
            detail: None,
            code: Some(err.code_str().to_string()),
        }
    }
}

impl XError {
    /// 是不是「连接本身不可用」（找不到 / 配置不合法 / 连不上 / 类型未知）。
    ///
    /// 用来区分两类失败：连接类错误必须**如实抛给界面**；而"连上了，但这条元数据查询
    /// 不被支持 / 没权限"才允许调用方做降级（例如库清单退回单库模式）。
    /// 不区分的话，改错主机/端口保存后，界面会拿到"200 + 退回来的库名"，一切看起来正常。
    pub fn is_conn_failure(&self) -> bool {
        matches!(
            self.code.as_deref(),
            Some("DBMIND-CONN-0001" | "DBMIND-CONN-0002" | "DBMIND-CONN-0003" | "DBMIND-CONN-0004")
        )
    }
}

/// `detail` 里的**技术尾巴**不该弹给用户（用户反馈：Oracle 连不上时弹窗最后一行是
/// `SQLRecoverableException, SQLState=08006, errorCode=17002`，纯噪音）。
///
/// `detail` 有两种来源：
/// 1. 我们自己写的操作指引（"把驱动 jar 放到 …，或执行 dbmind driver fetch"）——
///    有信息量，必须留着；
/// 2. JDBC 宿主用 `describe(SQLException)` 拼的异常类名 + SQLState + errorCode，
///    那是给排查用的。
///
/// 这里只按**行**筛掉第 2 类（外加 Java 栈帧与 `next:` 次生异常），其余一律保留；
/// 一行都不剩就把整段丢掉 —— 结论那句话本身已经说清了发生了什么。
fn human_detail(detail: &str) -> String {
    let all: Vec<&str> = detail.lines().filter(|l| !l.trim().is_empty()).collect();
    let kept: Vec<&str> = all.iter().copied().filter(|l| !is_technical_line(l)).collect();
    if kept.len() == all.len() {
        return detail.to_string(); // 没有技术行：原样（保留原有排版）
    }
    kept.join("\n")
}

/// 这一行是不是纯给开发看的（异常类名 / SQLState / 栈帧 / 次生异常）
fn is_technical_line(line: &str) -> bool {
    let s = line.trim();
    if s.starts_with("next: ") {
        return true;
    }
    // `at com.mysql.cj.jdbc.ConnectionImpl.createNewIO(ConnectionImpl.java:1234)`
    if s.starts_with("at ") && s.ends_with(')') && s.contains('(') {
        return true;
    }
    let has_state = s.contains("SQLState=");
    if has_state && s.contains("errorCode=") {
        return true;
    }
    // `SQLRecoverableException, SQLState=08006`（老代理只拼到 SQLState）
    let head = s
        .split(|c: char| c == ',' || c.is_whitespace())
        .next()
        .unwrap_or_default();
    has_state && head.ends_with("Exception")
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
        let XError { status, message, hint, detail, code: _ } = self;
        if status.is_server_error() {
            tracing::error!(status = %status, message = %message, "上游兼容层请求失败");
        }
        let mut body = json!({ "success": false, "message": message });
        // 只在真有拆段内容时多给两个字段：形状保持向后兼容（老前端只读 message）
        if let Some(hint) = hint {
            body["hint"] = json!(hint);
        }
        if let Some(detail) = detail {
            body["detail"] = json!(detail);
        }
        (status, Json(body)).into_response()
    }
}

pub type XResult<T> = Result<T, XError>;
