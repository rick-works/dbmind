//! 安全策略：**所有写操作的唯一闸门**。
//!
//! 设计要点（分层闸门）：
//! - 闸门在内核里，不在壳里 ⇒ CLI/MCP 的命令行开关**无法绕过**；
//! - 策略按「来源 + 连接属性 + 全局开关」三者与运算，任一层拒绝即拒绝；
//! - 拒绝时给稳定错误码，壳层可据此走「确认后重试」而不是解析文案。

use crate::error::{DbMindError, ErrorCode, Result};
use crate::sql::StatementKind;
use crate::statement::{classify, split_statements};
use crate::types::{AccessContext, ConnectionRecord};
use std::collections::HashSet;

/// 默认策略：桌面/CLI 可正常读写，AI/MCP 通道只读，**生产保护开启**
/// （生产保护与 AI 写入两项在启动时从设置载入，并可在运行期切换；
/// `Default` 只给出结构初值，真正生效的默认值来自 settings 的种子，
/// 见 `crates/dbmind-core/src/storage.rs` 的 `seed_settings` / `migrate_settings`）。
#[derive(Debug, Clone, Default)]
pub struct SafetyPolicy {
    /// 被标记为只读的连接 id（存储层的 `read_only` 字段是单一真源，这里做内存叠加）。
    read_only_connections: HashSet<String>,
    /// 生产保护：开启后，**只有标注为「生产」环境（`PROD`）的数据源**拒绝写/结构变更；
    /// 开发 / 测试 / 自定义分组 / 未分组的数据源一律放行（作用域见
    /// `ConnectionRecord::is_prod_environment`）。
    pub protect_production: bool,
    /// AI/MCP 通道是否允许写（默认 false；必须显式开启）。
    pub ai_write_enabled: bool,
    /// 危险语句拦截：无 WHERE 的 UPDATE/DELETE、TRUNCATE、DROP 一律拒绝
    /// （默认 false；判不了的形态交给其它闸门，不会错拦）。
    pub block_dangerous: bool,
    /// 写操作影响行数上限：UPDATE/DELETE 预估影响超过该值时拒绝（0 = 不限制）。
    /// 预估是执行前的 `COUNT(*)`，预估本身失败 ⇒ 跳过检查放行原语句。
    pub max_write_rows: u64,
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

    pub fn with_block_dangerous(mut self, enabled: bool) -> Self {
        self.block_dangerous = enabled;
        self
    }

    pub fn with_max_write_rows(mut self, max: u64) -> Self {
        self.max_write_rows = max;
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
    ///
    /// `internal`：是否**内核内部链路**（数据传输/对比/导入导出的批量执行，非用户手输）。
    /// 这类链路有多语句批的硬需求（identity 的 SET+INSERT+SET 同批、删库的
    /// ALTER+DROP 同批），**放行多语句**；其余写闸门（只读连接/生产保护/危险拦截）
    /// 全部照常生效。用户手输（桌面/CLI/MCP/AI）的多语句仍按原规则拦截。
    pub fn check(&self, connection: &ConnectionRecord, sql: &str, ctx: AccessContext, internal: bool) -> Result<()> {
        let protocol = connection.kind().protocol();
        let statements = split_statements(protocol, sql);
        if statements.is_empty() {
            return Err(DbMindError::new(ErrorCode::QueryInvalid, "语句为空"));
        }
        if statements.len() > 1 && !internal {
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

        // 危险语句拦截：放在所有写闸门最前面 —— 无 WHERE 的全表 UPDATE/DELETE
        // 连开发库都值得拦（手滑一次就是整表没了）。
        if self.block_dangerous {
            if let Some(reason) =
                crate::statement::danger_reason(protocol, &statements[0])
            {
                return Err(DbMindError::new(
                    ErrorCode::SafetyDangerous,
                    format!(
                        "危险操作已被拦截：{reason}。如确需执行，请到设置 → 安全与会话中关闭「危险操作拦截」",
                    ),
                ));
            }
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

        // 生产保护**只对标注为「生产」环境的数据源生效**：开发 / 测试 / 自定义分组 /
        // 未分组的一律放行（作用域由 `ConnectionRecord::is_prod_environment` 划定，
        // 影子连接继承主连接的环境标注，跨库浏览同样受保护）。
        if self.protect_production && connection.is_prod_environment() {
            return Err(DbMindError::new(
                ErrorCode::SafetyProduction,
                format!(
                    "生产保护已开启：数据源「{}」标注为生产环境，本次修改数据或结构的操作已被拒绝",
                    connection.name()
                ),
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

    /// 带**分组**的连接（`extra.group`），生产保护的作用域测试用。
    fn conn_with_env(env: &str) -> ConnectionRecord {
        let mut c = conn(false);
        c.config.extra = Some(serde_json::json!({ "group": env }));
        c
    }

    #[test]
    fn 只读语句永远放行() {
        let policy = SafetyPolicy::new();
        policy.check(&conn(true), "select 1", AccessContext::Mcp, false).unwrap();
    }

    #[test]
    fn ai_通道默认拦截写() {
        let policy = SafetyPolicy::new();
        let err = policy
            .check(&conn(false), "delete from t", AccessContext::Mcp, false)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyAiReadOnly);

        let permissive = SafetyPolicy::new().with_ai_write(true);
        permissive
            .check(&conn(false), "delete from t", AccessContext::Mcp, false)
            .unwrap();
    }

    #[test]
    fn 只读连接拦截写() {
        let policy = SafetyPolicy::new();
        let err = policy
            .check(&conn(true), "update t set a=1", AccessContext::Cli, false)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);
    }

    #[test]
    fn 生产保护优先于其它放行条件() {
        let policy = SafetyPolicy::new()
            .with_ai_write(true)
            .with_production_protection(true);
        let err = policy
            .check(
                &conn_with_env("PROD"),
                "insert into t values (1)",
                AccessContext::Desktop, false,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyProduction);
    }

    /// 生产保护的**作用域**：环境标识角标（`env`）或目录（`environment`）标注为生产才拦。
    /// （真机踩过两回：① 全局一刀切把无标注的只读浏览都拦了；② 只查目录字段，
    /// 用户把角标设成生产、目录还在「本地分组」，保护完全没生效。）
    #[test]
    fn 生产保护只对标注为生产的数据源生效() {
        let policy = SafetyPolicy::new().with_production_protection(true);
        for env in ["DEV", "TEST", "STAGING", "本地分组", ""] {
            let mut c = conn(false);
            if !env.is_empty() {
                c.config.extra = Some(serde_json::json!({ "group": env }));
            }
            policy
                .check(&c, "insert into t values (1)", AccessContext::Desktop, false)
                .unwrap_or_else(|e| panic!("环境 [{env}] 不应被生产保护拦截: {e}"));
        }
        // 角标（env 字段）标成生产 ⇒ 拦；角标是别的 ⇒ 放行
        let mut badge_prod = conn(false);
        badge_prod.config.extra = Some(serde_json::json!({ "env": "PROD", "group": "本地分组" }));
        policy
            .check(&badge_prod, "insert into t values (1)", AccessContext::Desktop, false)
            .unwrap_err();
        let mut badge_dev = conn(false);
        badge_dev.config.extra = Some(serde_json::json!({ "env": "DEV", "group": "本地分组" }));
        policy
            .check(&badge_dev, "insert into t values (1)", AccessContext::Desktop, false)
            .unwrap_or_else(|e| panic!("角标 DEV 不应被拦截: {e}"));
        // 大小写不敏感（前端存的是大写键，但别把 "prod" 这类手输值放空子）
        policy
            .check(
                &conn_with_env("prod"),
                "insert into t values (1)",
                AccessContext::Desktop, false,
            )
            .unwrap_err();
        // 旧版分组键 `environment` 依然被认（老库里的存量连接不迁移也能受保护）
        let mut legacy = conn(false);
        legacy.config.extra = Some(serde_json::json!({ "environment": "PROD" }));
        policy
            .check(&legacy, "insert into t values (1)", AccessContext::Desktop, false)
            .unwrap_err();
    }

    #[test]
    fn 多条语句一律拒绝() {
        let policy = SafetyPolicy::new();
        let err = policy
            .check(&conn(true), "select 1; drop table t", AccessContext::Desktop, false)
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
                AccessContext::Desktop, false,
            )
            .unwrap();
        policy
            .check(
                &mongo_conn(true),
                r#"{ "count": "users" }"#,
                AccessContext::Desktop, false,
            )
            .unwrap();

        // 写命令：只读连接拦截
        let err = policy
            .check(
                &mongo_conn(true),
                "db.users.deleteMany({})",
                AccessContext::Desktop, false,
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
            .check(&mongo_conn(true), "db.users.drop()", AccessContext::Cli, false)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);

        // AI/MCP 通道默认只读
        let err = policy
            .check(
                &mongo_conn(false),
                r#"{ "insert": "users", "documents": [] }"#,
                AccessContext::Mcp, false,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyAiReadOnly);

        // 生产保护（只对标注为生产的连接生效）
        let strict = SafetyPolicy::new()
            .with_ai_write(true)
            .with_production_protection(true);
        let mut mongo_prod = mongo_conn(false);
        mongo_prod.config.extra = Some(serde_json::json!({ "group": "PROD" }));
        let err = strict
            .check(
                &mongo_prod,
                "db.users.updateOne({}, { $set: { a: 1 } })",
                AccessContext::Desktop, false,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyProduction);
        // 未标注环境的同一条写命令不受生产保护影响
        strict
            .check(
                &mongo_conn(false),
                "db.users.updateOne({}, { $set: { a: 1 } })",
                AccessContext::Desktop, false,
            )
            .unwrap();

        // 判不出来的命令按非只读处理（保守）——否则新命令就是缺口
        let err = policy
            .check(
                &mongo_conn(true),
                "db.users.someBrandNewAdminCommand({})",
                AccessContext::Desktop, false,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);

        // aggregate 写回集合（$out/$merge）：最容易漏的一类，必须被拦
        let err = policy
            .check(
                &mongo_conn(true),
                r#"db.orders.aggregate([{ "$match": {} }, { "$out": "summary" }])"#,
                AccessContext::Desktop, false,
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
            .check(&conn(true), "drop table t", AccessContext::Cli, false)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);
    }
}
