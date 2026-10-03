//! 语句判定的**协议分派层**。
//!
//! 安全闸门必须与数据库协议无关：无论用户提交的是 SQL 还是 Mongo 命令，
//! 「这条语句是读还是写」都要在内核里判出来。于是：
//!
//! ```text
//!   连接类型(YAML) --protocol--> RuntimeProtocol --分派--> sql::classify / mongo::classify
//! ```
//!
//! 这样加 Redis / Elasticsearch 时，只需再加一个判定模块 + 一份 YAML，
//! 闸门本身不用动（这正是「数据驱动」在安全层的兑现）。

use crate::kind::RuntimeProtocol;

pub use crate::sql::StatementKind;
pub use crate::sql::WriteTarget;

/// 按协议拆分语句（安全策略据此拒绝「一次多条」）。
pub fn split_statements(protocol: RuntimeProtocol, text: &str) -> Vec<String> {
    match protocol {
        RuntimeProtocol::Sql => crate::sql::split_statements(text),
        // Mongo 侧直接复用 SQL 的拆分器：它能正确处理引号内的 `;`。
        // 代价是 Mongo 的 `//` 注释不会被识别 ⇒ 注释里的 `;` 可能多拆出一段，
        // 结果是「被判成多条语句而拒绝」——方向保守（宁可多拦），可以接受。
        RuntimeProtocol::Mongodb => crate::sql::split_statements(text),
        // Redis：一条命令一行
        RuntimeProtocol::Redis => crate::redis::split_statements(text),
        // Elasticsearch：一行 `METHOD /path` 开始一条请求（其后的 JSON 体属于它）
        RuntimeProtocol::Elasticsearch => crate::es::split_statements(text),
    }
}

/// 判断单条语句的类型。
pub fn classify(protocol: RuntimeProtocol, text: &str) -> StatementKind {
    match protocol {
        RuntimeProtocol::Sql => crate::sql::classify(text),
        RuntimeProtocol::Mongodb => crate::mongo::classify(text),
        RuntimeProtocol::Redis => crate::redis::classify(text),
        RuntimeProtocol::Elasticsearch => crate::es::classify(text),
    }
}

/// 危险语句识别（设置页「危险语句拦截」的判定源）。
///
/// 只有 SQL 有「无 WHERE 全表改 / TRUNCATE / DROP」这套形态；Mongo/Redis/ES
/// 的危险命令各成一族，不在这一层判（它们受只读与生产保护两道闸管着）。
pub fn danger_reason(protocol: RuntimeProtocol, text: &str) -> Option<&'static str> {
    match protocol {
        RuntimeProtocol::Sql => crate::sql::danger_reason(text),
        _ => None,
    }
}

/// 从 UPDATE/DELETE 抽「目标表 + WHERE」（影响行数预估用，见 [`crate::sql::write_target`]）。
/// 同 `danger_reason`：只有 SQL 形态可解析，其它协议返回 `None`。
pub fn write_target(protocol: RuntimeProtocol, text: &str) -> Option<WriteTarget> {
    match protocol {
        RuntimeProtocol::Sql => crate::sql::write_target(text),
        _ => None,
    }
}

/// 语句作用于哪个对象（只有读语句才有意义）。
///
/// 与 `classify` 一样按协议分派：**语句语义只在这里维护一处**，
/// 否则「谁能编辑」的判定会散落进壳层，慢慢和内核判定漂移。
///
/// 三种有「行」的协议都给得出来，严格程度一致 —— 结果必须确实是
/// **该对象的原始行/文档**（无 JOIN/聚合/别名、无 `$project`、无多索引）：
///
/// - SQL → 单表裸列（`sql::single_table`）；
/// - Mongo → 单集合的 `find` / `findOne`（`mongo::single_collection`）；
/// - ES → 单索引的 `/_search`（`es::single_index`）。
///
/// Redis 永远给 None：它的结果没有「行」这个概念，谈不上就地编辑。
pub fn source_object(protocol: RuntimeProtocol, text: &str) -> Option<String> {
    match protocol {
        RuntimeProtocol::Sql => crate::sql::single_table(text),
        RuntimeProtocol::Mongodb => crate::mongo::single_collection(text),
        RuntimeProtocol::Elasticsearch => crate::es::single_index(text),
        RuntimeProtocol::Redis => None,
    }
}

/// 由**结果本身**推出来的行标识（协议事实），拿不到就返回 None。
///
/// 只有 ES 走这条：搜索命中天然带 `_id`（字符串），索引名由 `source_object` 给出，
/// 于是「这一行是谁」是确定的，不需要宿主额外说明。
///
/// Mongo **刻意不在这里推**：`_id` 在结果里只是一串十六进制，是不是 ObjectId
/// 只有宿主知道（见 `types::IdentityValueType`）。内核推不出来就不推 ——
/// 猜错会生成一个匹配不到任何行的条件，得到一个「保存成功、0 行受影响」的假成功。
pub fn derived_row_identity(
    protocol: RuntimeProtocol,
    columns: &[crate::types::ColumnMeta],
) -> Option<crate::types::RowIdentity> {
    if protocol != RuntimeProtocol::Elasticsearch {
        return None;
    }
    let has_id = columns.iter().any(|column| column.name == "_id");
    has_id.then(|| crate::types::RowIdentity {
        column: "_id".to_string(),
        value_type: crate::types::IdentityValueType::String,
    })
}

/// 整段是否只读：必须是单条且该条为只读。
pub fn is_read_only(protocol: RuntimeProtocol, text: &str) -> bool {
    // 各协议自己实现「单条 + 只读」的语义，这里只做分派（避免两处规则漂移）
    match protocol {
        RuntimeProtocol::Sql => crate::sql::is_read_only(text),
        RuntimeProtocol::Mongodb => crate::mongo::is_read_only(text),
        RuntimeProtocol::Redis => crate::redis::is_read_only(text),
        RuntimeProtocol::Elasticsearch => crate::es::is_read_only(text),
    }
}

pub fn statement_count(protocol: RuntimeProtocol, text: &str) -> usize {
    match protocol {
        RuntimeProtocol::Sql => crate::sql::statement_count(text),
        _ => split_statements(protocol, text).len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_协议走_sql_判定() {
        assert_eq!(classify(RuntimeProtocol::Sql, "select 1"), StatementKind::Read);
        assert_eq!(classify(RuntimeProtocol::Sql, "drop table t"), StatementKind::Ddl);
        assert!(is_read_only(RuntimeProtocol::Sql, "select 1"));
        assert!(!is_read_only(RuntimeProtocol::Sql, "db.users.find({})"));
        // 「db.users.find」不是合法 SQL 首关键字 ⇒ 保守判为未知
        assert_eq!(
            classify(RuntimeProtocol::Sql, "db.users.find({})"),
            StatementKind::Unknown
        );
    }

    #[test]
    fn mongodb_协议走_mongo_判定() {
        assert_eq!(
            classify(RuntimeProtocol::Mongodb, "db.users.find({})"),
            StatementKind::Read
        );
        assert_eq!(
            classify(RuntimeProtocol::Mongodb, "db.users.deleteMany({})"),
            StatementKind::Write
        );
        assert!(!is_read_only(RuntimeProtocol::Mongodb, "db.users.drop()"));
        // 反过来：SQL 文本在 Mongo 协议下判不出来 ⇒ 保守
        assert_eq!(
            classify(RuntimeProtocol::Mongodb, "select 1"),
            StatementKind::Unknown
        );
    }

    #[test]
    fn redis_协议走命令判定() {
        assert_eq!(classify(RuntimeProtocol::Redis, "GET k"), StatementKind::Read);
        assert_eq!(classify(RuntimeProtocol::Redis, "SET k v"), StatementKind::Write);
        assert_eq!(classify(RuntimeProtocol::Redis, "FLUSHALL"), StatementKind::Ddl);
        assert!(is_read_only(RuntimeProtocol::Redis, "HGETALL h"));
        assert!(!is_read_only(RuntimeProtocol::Redis, "DEL k"));
        // Mongo 写法在 Redis 协议下判不出来 ⇒ 保守（非只读）
        assert_eq!(
            classify(RuntimeProtocol::Redis, "db.users.find({})"),
            StatementKind::Unknown
        );
    }

    #[test]
    fn elasticsearch_协议走请求判定() {
        assert_eq!(
            classify(RuntimeProtocol::Elasticsearch, "GET /logs/_search"),
            StatementKind::Read
        );
        assert_eq!(
            classify(RuntimeProtocol::Elasticsearch, "DELETE /logs"),
            StatementKind::Ddl
        );
        assert_eq!(
            classify(RuntimeProtocol::Elasticsearch, "POST /logs/_doc\n{}"),
            StatementKind::Write
        );
        assert!(is_read_only(
            RuntimeProtocol::Elasticsearch,
            "GET /_cat/indices?v"
        ));
        assert!(!is_read_only(RuntimeProtocol::Elasticsearch, "PUT /logs"));
    }

    /// 同一段文本在不同协议下含义不同 —— 这正是「协议」必须显式声明的原因。
    #[test]
    fn 同一文本在不同协议下不会被互相认错() {
        // Redis 里 SELECT 是切库：连接状态切换而非数据修改，归为读
        assert_eq!(classify(RuntimeProtocol::Redis, "SELECT 1"), StatementKind::Read);
        // Mongo 里 select 不是命令 ⇒ 判不出来（保守）
        assert_eq!(
            classify(RuntimeProtocol::Mongodb, "select 1"),
            StatementKind::Unknown
        );
        // ES 里必须带方法，裸 JSON 不猜
        assert_eq!(
            classify(RuntimeProtocol::Elasticsearch, "{ \"query\": {} }"),
            StatementKind::Unknown
        );
    }

    #[test]
    fn 按协议给出来源对象() {
        assert_eq!(
            source_object(RuntimeProtocol::Sql, "select * from users"),
            Some("users".to_string())
        );
        assert_eq!(
            source_object(RuntimeProtocol::Sql, "select count(*) from users"),
            None
        );
        // 三种「有行」的协议都给得出来
        assert_eq!(
            source_object(RuntimeProtocol::Mongodb, "db.users.find({})"),
            Some("users".to_string())
        );
        assert_eq!(
            source_object(RuntimeProtocol::Elasticsearch, "GET /logs/_search"),
            Some("logs".to_string())
        );
        // 但都严守同一条线：结果必须就是该对象的原始行/文档
        assert_eq!(
            source_object(
                RuntimeProtocol::Mongodb,
                "db.users.aggregate([{ \"$match\": {} }])"
            ),
            None
        );
        assert_eq!(
            source_object(RuntimeProtocol::Elasticsearch, "GET /_search"),
            None
        );
        // Redis 的结果没有「行」这个概念
        assert_eq!(source_object(RuntimeProtocol::Redis, "GET k"), None);
    }

    /// 行标识只从结果本身推得出来的协议推：ES 的命中 `_id` 天生是字符串。
    ///
    /// Mongo **刻意不推** —— `_id` 在结果里只是一串十六进制，是 ObjectId 还是字符串
    /// 只有宿主知道；在这里猜一个类型，界面就会拿它去生成一个匹配不到任何行的条件。
    #[test]
    fn 行标识只从推得出来的协议推() {
        let with_id = vec![
            crate::types::ColumnMeta {
                name: "_id".to_string(),
                type_name: None,
            },
            crate::types::ColumnMeta {
                name: "name".to_string(),
                type_name: None,
            },
        ];
        let identity = derived_row_identity(RuntimeProtocol::Elasticsearch, &with_id).unwrap();
        assert_eq!(identity.column, "_id");
        assert_eq!(identity.value_type, crate::types::IdentityValueType::String);

        // 结果里没有 `_id` 列 ⇒ 不给（界面据此拒绝编辑，而不是生成一个空条件）
        let without_id = vec![crate::types::ColumnMeta {
            name: "name".to_string(),
            type_name: None,
        }];
        assert_eq!(
            derived_row_identity(RuntimeProtocol::Elasticsearch, &without_id),
            None
        );

        // 其它协议不推
        assert_eq!(derived_row_identity(RuntimeProtocol::Mongodb, &with_id), None);
        assert_eq!(derived_row_identity(RuntimeProtocol::Sql, &with_id), None);
    }

    #[test]
    fn 两种协议都能识别多条语句() {
        assert_eq!(statement_count(RuntimeProtocol::Sql, "select 1; select 2"), 2);
        assert_eq!(
            statement_count(RuntimeProtocol::Mongodb, "db.a.find({}); db.a.drop()"),
            2
        );
    }
}
