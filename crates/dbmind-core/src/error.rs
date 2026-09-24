//! 错误模型：**稳定错误码 + 可变文案**。
//!
//! 规则：错误码是对外契约，一旦发布**永不改变含义、永不复用**；
//! 文案可以改、可以被前端 i18n 覆盖。壳层据此分类处理（重试 / 提示 / 阻断），
//! 而不是去匹配错误字符串。

use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, DbMindError>;

/// 错误码目录。新增错误必须在此登记，并在 `docs/error-codes.md` 说明。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // ---- 连接 DBMIND-CONN-xxxx
    /// 连接不存在（按 id 或名称都找不到）
    ConnNotFound,
    /// 连接配置不合法（缺必填项、字段组合矛盾）
    ConnInvalid,
    /// 建立连接失败（网络/认证/驱动）
    ConnConnectFailed,
    /// 连接类型未在 YAML 中登记
    ConnTypeUnknown,

    // ---- 查询 DBMIND-QUERY-xxxx
    /// SQL 本身不合法或不被允许（含多条语句）
    QueryInvalid,
    /// 执行失败（数据库返回错误）
    QueryFailed,
    /// 超过超时阈值
    QueryTimeout,
    /// 被用户取消
    QueryCanceled,
    /// 写语句被安全策略阻断
    QueryWriteBlocked,

    // ---- 安全 DBMIND-SAFETY-xxxx
    /// 该连接被标记为只读
    SafetyReadOnly,
    /// AI/MCP 通道默认只读，写操作需显式放开
    SafetyAiReadOnly,
    /// 生产保护生效
    SafetyProduction,

    // ---- 存储 DBMIND-STORAGE-xxxx
    /// 本地元数据库读写失败
    StorageFailed,
    /// 数据目录不可用
    StorageUnavailable,

    // ---- 驱动 DBMIND-DRV-xxxx
    /// 该类型的运行时尚未接入内核
    DriverNotImplemented,
    /// 驱动缺失或未就绪（如 JDBC 驱动 jar 未下载）
    DriverNotReady,

    // ---- 内部 DBMIND-INTERNAL-xxxx
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::ConnNotFound => "DBMIND-CONN-0001",
            ErrorCode::ConnInvalid => "DBMIND-CONN-0002",
            ErrorCode::ConnConnectFailed => "DBMIND-CONN-0003",
            ErrorCode::ConnTypeUnknown => "DBMIND-CONN-0004",
            ErrorCode::QueryInvalid => "DBMIND-QUERY-0001",
            ErrorCode::QueryFailed => "DBMIND-QUERY-0002",
            ErrorCode::QueryTimeout => "DBMIND-QUERY-0003",
            ErrorCode::QueryCanceled => "DBMIND-QUERY-0004",
            ErrorCode::QueryWriteBlocked => "DBMIND-QUERY-0005",
            ErrorCode::SafetyReadOnly => "DBMIND-SAFETY-0001",
            ErrorCode::SafetyAiReadOnly => "DBMIND-SAFETY-0002",
            ErrorCode::SafetyProduction => "DBMIND-SAFETY-0003",
            ErrorCode::StorageFailed => "DBMIND-STORAGE-0001",
            ErrorCode::StorageUnavailable => "DBMIND-STORAGE-0002",
            ErrorCode::DriverNotImplemented => "DBMIND-DRV-0001",
            ErrorCode::DriverNotReady => "DBMIND-DRV-0002",
            ErrorCode::Internal => "DBMIND-INTERNAL-0001",
        }
    }

    /// 分类前缀，便于日志聚合与前端分类展示。
    pub fn category(self) -> &'static str {
        match self {
            ErrorCode::ConnNotFound
            | ErrorCode::ConnInvalid
            | ErrorCode::ConnConnectFailed
            | ErrorCode::ConnTypeUnknown => "conn",
            ErrorCode::QueryInvalid
            | ErrorCode::QueryFailed
            | ErrorCode::QueryTimeout
            | ErrorCode::QueryCanceled
            | ErrorCode::QueryWriteBlocked => "query",
            ErrorCode::SafetyReadOnly | ErrorCode::SafetyAiReadOnly | ErrorCode::SafetyProduction => "safety",
            ErrorCode::StorageFailed | ErrorCode::StorageUnavailable => "storage",
            ErrorCode::DriverNotImplemented | ErrorCode::DriverNotReady => "driver",
            ErrorCode::Internal => "internal",
        }
    }

    /// 是否值得原样重试（幂等且是瞬时故障）。
    pub fn retryable(self) -> bool {
        matches!(self, ErrorCode::ConnConnectFailed | ErrorCode::StorageFailed)
    }

    /// Web 壳的 HTTP 状态码映射。
    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::ConnNotFound => 404,
            ErrorCode::ConnInvalid | ErrorCode::QueryInvalid => 400,
            ErrorCode::SafetyReadOnly
            | ErrorCode::SafetyAiReadOnly
            | ErrorCode::SafetyProduction
            | ErrorCode::QueryWriteBlocked => 403,
            ErrorCode::QueryTimeout => 504,
            ErrorCode::QueryCanceled => 499,
            ErrorCode::DriverNotImplemented => 501,
            ErrorCode::DriverNotReady => 503,
            ErrorCode::StorageUnavailable => 503,
            _ => 500,
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// SQL 出错位置（行列均从 1 开始）。存在时应当回填，便于编辑器定位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlPosition {
    pub line: usize,
    pub column: usize,
}

impl SqlPosition {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("[{code}] {message}")]
pub struct DbMindError {
    pub code: ErrorCode,
    pub message: String,
    pub detail: Option<String>,
    pub position: Option<SqlPosition>,
}

impl DbMindError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            position: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_position(mut self, line: usize, column: usize) -> Self {
        self.position = Some(SqlPosition::new(line, column));
        self
    }

    pub fn code_str(&self) -> &'static str {
        self.code.as_str()
    }

    pub fn retryable(&self) -> bool {
        self.code.retryable()
    }

    pub fn payload(&self) -> ErrorPayload {
        ErrorPayload {
            code: self.code.as_str().to_string(),
            category: self.code.category().to_string(),
            message: self.message.clone(),
            detail: self.detail.clone(),
            position: self.position,
        }
    }
}

/// 对外（HTTP / MCP / IPC）统一的错误结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: String,
    pub category: String,
    pub message: String,
    pub detail: Option<String>,
    pub position: Option<SqlPosition>,
}

impl std::fmt::Display for ErrorPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl From<rusqlite::Error> for DbMindError {
    fn from(e: rusqlite::Error) -> Self {
        // 元数据库错误与用户 SQL 错误要分开：前者是 storage，后者是 query
        DbMindError::new(ErrorCode::StorageFailed, "本地元数据库操作失败").with_detail(e.to_string())
    }
}

impl From<std::io::Error> for DbMindError {
    fn from(e: std::io::Error) -> Self {
        DbMindError::new(ErrorCode::StorageFailed, "文件读写失败").with_detail(e.to_string())
    }
}

impl From<serde_json::Error> for DbMindError {
    fn from(e: serde_json::Error) -> Self {
        DbMindError::new(ErrorCode::Internal, "JSON 处理失败").with_detail(e.to_string())
    }
}

/// 把「执行用户 SQL 时的 rusqlite 错误」翻译成查询类错误码。
pub fn query_error(e: rusqlite::Error) -> DbMindError {
    DbMindError::new(ErrorCode::QueryFailed, "SQL 执行失败").with_detail(e.to_string())
}
