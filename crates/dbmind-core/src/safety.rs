//! 安全策略：**所有写操作的唯一闸门**。
//!
//! 设计要点（对齐 dbx 的分层闸门）：
//! - 闸门在内核里，不在壳里 ⇒ CLI/MCP 的命令行开关**无法绕过**；
//! - 策略按「来源 + 连接属性 + 全局开关」三者与运算，任一层拒绝即拒绝；
//! - 拒绝时给稳定错误码，壳层可据此走「确认后重试」而不是解析文案。

use crate::error::{DbMindError, ErrorCode, Result};
use crate::sql::StatementKind;
use crate::statement::{classify, split_statements};
use crate::types::{AccessContext, ConnectionRecord};
use std::collections::HashSet;

/// 默认策略：桌面/CLI 可正常读写，AI/MCP 通道只读，生产保护关闭
/// （生产保护与 AI 写入两项在启动时从设置载入，并可在运行期切换）。
#[derive(Debug, Clone, Default)]
pub struct SafetyPolicy {
    /// 被标记为只读的连接 id（存储层的 `read_only` 字段是单一真源，这里做内存叠加）。
    read_only_connections: HashSet<String>,
    /// 全局写保护：生产/演练环境打开后，任何写语句一律拒绝。
    pub protect_production: bool,
    /// AI/MCP 通道是否允许写（默认 false；必须显式开启）。
    pub ai_write_enabled: bool,
}

impl SafetyPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_production_protection(mut self, enabled: bool) -> Self {
        self.protect_production = enabled;
        self
    }

    pub fn with_ai_write(mut self, enabled: bool) -> Self {
        self.ai_write_enabled = enabled;
        self
    }

    pub fn mark_read_only(&mut self, connection_id: impl Into<String>) {
        self.read_only_connections.insert(connection_id.into());
    }

    pub fn unmark_read_only(&mut self, connection_id: &str) {
        self.read_only_connections.remove(connection_id);
    }

    pub fn is_read_only(&self, connection: &ConnectionRecord) -> bool {
        connection.read_only || self.read_only_connections.contains(&connection.id)
    }

    /// 执行前的强制校验。
    ///
    /// 语句判定**按连接的协议分派**（SQL 走词法判定、MongoDB 走命令判定）：
    /// 闸门本身与协议无关，但它必须懂该协议的语句语义，否则非 SQL 库上会失效。
    pub fn check(&self, connection: &ConnectionRecord, sql: &str, ctx: AccessContext) -> Result<()> {
        let protocol = connection.kind().protocol();
        let statements = split_statements(protocol, sql);
        if statements.is_empty() {
            return Err(DbMindError::new(ErrorCode::QueryInvalid, "语句为空"));
        }
        if statements.len() > 1 {
            return Err(DbMindError::new(
                ErrorCode::QueryInvalid,
                format!(
                    "一次只允许执行一条语句（检测到 {} 条），请拆分后执行",
                    statements.len()
                ),
            ));
        }

        let kind = classify(protocol, &statements[0]);
        if kind.is_read_only() {
            return Ok(());
        }

        // 以下都是「写/结构变更/无法归类」——逐层过闸
        if ctx.is_ai_like() && !self.ai_write_enabled {
            return Err(DbMindError::new(
                ErrorCode::SafetyAiReadOnly,
                format!(
                    "{} 通道默认只允许只读语句；如需放开，请在设置中显式开启「允许 AI/MCP 写入」",
                    ctx.as_str().to_uppercase()
                ),
            ));
        }

        if self.is_read_only(connection) {
            return Err(DbMindError::new(
                ErrorCode::SafetyReadOnly,
                format!(
                    "连接「{}」被标记为只读，{} 语句已被拒绝",
                    connection.name(),
                    kind.as_str().to_uppercase()
                ),
            ));
        }

        if self.protect_production {
            return Err(DbMindError::new(
                ErrorCode::SafetyProduction,
                "生产保护已开启：本次操作会修改数据或结构，已被拒绝（关闭生产保护后重试）",
            ));
        }

        if kind == StatementKind::Unknown {
            // 无法归类：写通道放行（例如 SET / CALL），但记录在通知里由壳层提示
            tracing::debug!(target: "dbmind::safety", sql = %statements[0], "无法归类的语句按写操作放行");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ConnectionConfig;
    use crate::ConnectionKind;

    fn conn(read_only: bool) -> ConnectionRecord {
        ConnectionRecord {
            id: "c1".into(),
            config: ConnectionConfig::new("demo", ConnectionKind::Sqlite).with_file("./demo.db"),
            read_only,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn 只读语句永远放行() {
        let policy = SafetyPolicy::new();
        policy.check(&conn(true), "select 1", AccessContext::Mcp).unwrap();
    }

    #[test]
    fn ai_通道默认拦截写() {
        let policy = SafetyPolicy::new();
        let err = policy
            .check(&conn(false), "delete from t", AccessContext::Mcp)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyAiReadOnly);

        let permissive = SafetyPolicy::new().with_ai_write(true);
        permissive
            .check(&conn(false), "delete from t", AccessContext::Mcp)
            .unwrap();
    }

    #[test]
    fn 只读连接拦截写() {
        let policy = SafetyPolicy::new();
        let err = policy
            .check(&conn(true), "update t set a=1", AccessContext::Cli)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);
    }

    #[test]
    fn 生产保护优先于其它放行条件() {
        let policy = SafetyPolicy::new()
            .with_ai_write(true)
            .with_production_protection(true);
        let err = policy
            .check(&conn(false), "insert into t values (1)", AccessContext::Desktop)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyProduction);
    }

    #[test]
    fn 多条语句一律拒绝() {
        let policy = SafetyPolicy::new();
        let err = policy
            .check(&conn(false), "select 1; drop table t", AccessContext::Desktop)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::QueryInvalid);
    }

    fn mongo_conn(read_only: bool) -> ConnectionRecord {
        ConnectionRecord {
            id: "m1".into(),
            config: ConnectionConfig::new("mongo", ConnectionKind::Mongodb).with_host("127.0.0.1"),
            read_only,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// 非 SQL 协议必须**同样**受闸门约束 —— 这是引入 protocol 抽象的目的。
    #[test]
    fn mongodb_命令同样过闸() {
        let policy = SafetyPolicy::new();

        // 只读命令：即使连接只读也放行
        policy
            .check(
                &mongo_conn(true),
                "db.users.find({ age: 30 })",
                AccessContext::Desktop,
            )
            .unwrap();
        policy
            .check(
                &mongo_conn(true),
                r#"{ "count": "users" }"#,
                AccessContext::Desktop,
            )
            .unwrap();

        // 写命令：只读连接拦截
        let err = policy
            .check(
                &mongo_conn(true),
                "db.users.deleteMany({})",
                AccessContext::Desktop,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);
        assert!(
            err.message.contains("WRITE"),
            "应说明被拦的是写操作：{}",
            err.message
        );

        // 结构变更：同样拦截
        let err = policy
            .check(&mongo_conn(true), "db.users.drop()", AccessContext::Cli)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);

        // AI/MCP 通道默认只读
        let err = policy
            .check(
                &mongo_conn(false),
                r#"{ "insert": "users", "documents": [] }"#,
                AccessContext::Mcp,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyAiReadOnly);

        // 生产保护
        let strict = SafetyPolicy::new()
            .with_ai_write(true)
            .with_production_protection(true);
        let err = strict
            .check(
                &mongo_conn(false),
                "db.users.updateOne({}, { $set: { a: 1 } })",
                AccessContext::Desktop,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyProduction);

        // 判不出来的命令按非只读处理（保守）——否则新命令就是缺口
        let err = policy
            .check(
                &mongo_conn(true),
                "db.users.someBrandNewAdminCommand({})",
                AccessContext::Desktop,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);

        // aggregate 写回集合（$out/$merge）：最容易漏的一类，必须被拦
        let err = policy
            .check(
                &mongo_conn(true),
                r#"db.orders.aggregate([{ "$match": {} }, { "$out": "summary" }])"#,
                AccessContext::Desktop,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);
    }

    #[test]
    fn 内存标记也能让连接只读() {
        let mut policy = SafetyPolicy::new();
        policy.mark_read_only("c1");
        assert!(policy.is_read_only(&conn(false)));
        let err = policy
            .check(&conn(false), "drop table t", AccessContext::Cli)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);
    }
}
