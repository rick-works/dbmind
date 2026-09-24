//! Elasticsearch 语句的只读判定。
//!
//! 语句就是一条 REST 请求（这也是 ES 用户实际的工作方式）：
//!
//! ```text
//!   GET  /logs/_search
//!   POST /logs/_doc        { "level": "info" }
//!   PUT  /logs
//!   DELETE /logs/_doc/1
//! ```
//!
//! 首行是 `方法 路径`，其后为可选的请求体。判定靠**方法 + 路径末段动作**：
//!
//! - `GET` / `HEAD` ⇒ 读；
//! - `POST /…/_search`（含 `_count` / `_analyze` 等）⇒ 读，其它 `POST` ⇒ 写；
//! - `PUT` 单段路径（`/idx`）= 建索引 ⇒ 结构变更；`PUT /idx/_doc/1` ⇒ 写；`PUT /idx/_mapping` ⇒ 结构变更；
//! - `DELETE /idx` ⇒ 结构变更，`DELETE /idx/_doc/1` ⇒ 写。
//!
//! 判不出来的（含裸 JSON、未知方法）一律按非只读处理。

use crate::sql::StatementKind;

const METHODS: [&str; 6] = ["GET", "POST", "PUT", "DELETE", "HEAD", "PATCH"];

/// 路径末段是这些动作时，`POST` 属于只读查询。
const READ_ACTIONS: &[&str] = &[
    "_search",
    "_count",
    "_msearch",
    "_mget",
    "_analyze",
    "_explain",
    "_validate",
    "_field_caps",
    "_mapping",
    "_settings",
    "_stats",
    "_segments",
    "_recovery",
    "_shards",
    "_search_shards",
    "_resolve",
    "_rank_eval",
    "_terms_enum",
    "_pit",
    "_tasks",
    "_nodes",
    "_cluster",
    "_cat",
    "_alias",
    "_aliases",
    "_ilm",
    "_component_template",
];

/// 路径末段是这些动作时，`PUT` / `DELETE` 属于结构变更。
const STRUCTURE_ACTIONS: &[&str] = &[
    "_mapping",
    "_settings",
    "_alias",
    "_aliases",
    "_template",
    "_index_template",
    "_component_template",
    "_ilm",
    "_ingest",
    "_lifecycle",
    "_close",
    "_open",
    "_cache",
    "_flush",
    "_refresh",
    "_forcemerge",
];

/// 文档级写入动作（`PUT` / `DELETE` / `POST` 到这些路径是数据变更）。
const DOC_ACTIONS: &[&str] = &[
    "_doc",
    "_create",
    "_update",
    "_bulk",
    "_delete_by_query",
    "_update_by_query",
    "_reindex",
    "_msearch_template",
    "_search_template",
];

pub struct Request {
    pub method: String,
    pub path: String,
}

/// 解析首行 `方法 路径`；解析不出来返回 None。
pub fn parse(text: &str) -> Option<Request> {
    let first = text.lines().find(|line| !line.trim().is_empty())?;
    let mut parts = first.split_whitespace();
    // 注意：这里用 split_whitespace 取前两段 —— 请求体若与路径同行会被自动丢掉，
    // 判定只看方法与路径，正是我们需要的
    let method = parts.next()?.to_ascii_uppercase();
    if !METHODS.contains(&method.as_str()) {
        return None;
    }
    let raw_path = parts.next()?;
    // 允许只写路径（方法默认 GET）？不：ES 的写路径必须显式写方法，猜错代价太大。
    //
    // 路径只取第一个空白之前的部分：`POST /logs/_search { "query": {} }` 这种
    // 「路径与请求体同一行」的写法很常见，不切开就会把整行当成路径 ——
    // 判定退化成「不知道」，只读连接上连搜索都会被拒。
    let path = raw_path
        .split('?')
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string();
    Some(Request { method, path })
}

/// 路径里的**动作段**：ES 的动作段以 `_` 开头，且不一定是末段
/// （`/logs/_doc/1` → `_doc`，`/logs/_update/1` → `_update`）。
/// 没有 `_` 段时退回末段（`/logs` → `logs`）。
fn action(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    segments
        .iter()
        .find(|segment| segment.starts_with('_'))
        .or_else(|| segments.last())
        .map(|segment| segment.to_ascii_lowercase())
        .unwrap_or_default()
}

/// 路径段数（忽略空段）：`/logs` → 1，`/logs/_doc/1` → 3。
fn segments(path: &str) -> usize {
    path.split('/').filter(|s| !s.is_empty()).count()
}

pub fn classify(text: &str) -> StatementKind {
    let Some(request) = parse(text) else {
        return StatementKind::Unknown;
    };
    let action = action(&request.path);
    let depth = segments(&request.path);
    let is_read_action = READ_ACTIONS.contains(&action.as_str());
    let is_structure_action = STRUCTURE_ACTIONS.contains(&action.as_str());
    let is_doc_action = DOC_ACTIONS.contains(&action.as_str());

    match request.method.as_str() {
        "GET" | "HEAD" => StatementKind::Read,
        // POST 既能查也能写：唯一可靠的判据是动作段
        "POST" => {
            if is_read_action {
                StatementKind::Read
            } else if is_structure_action {
                // 结构级动作（_close / _flush / _refresh …）算结构变更，不是数据写入
                StatementKind::Ddl
            } else if is_doc_action {
                StatementKind::Write
            } else if depth == 1 {
                // `POST /idx`（极少见）归读
                StatementKind::Read
            } else {
                StatementKind::Unknown
            }
        }
        "PUT" => {
            if is_structure_action {
                StatementKind::Ddl
            } else if is_doc_action {
                StatementKind::Write
            } else if depth == 1 {
                // `PUT /idx` = 建索引
                StatementKind::Ddl
            } else {
                StatementKind::Unknown
            }
        }
        "DELETE" => {
            if is_doc_action {
                StatementKind::Write
            } else if depth == 1 {
                // `DELETE /idx` = 删索引
                StatementKind::Ddl
            } else if is_structure_action {
                StatementKind::Ddl
            } else {
                StatementKind::Unknown
            }
        }
        "PATCH" => {
            if is_doc_action {
                StatementKind::Write
            } else {
                StatementKind::Unknown
            }
        }
        _ => StatementKind::Unknown,
    }
}

/// 按「方法行」拆分：一行以 HTTP 方法开头即视为新请求的开始。
pub fn split_statements(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let starts_request = METHODS
            .iter()
            .any(|method| trimmed.starts_with(&format!("{method} ")));
        if starts_request && !current.trim().is_empty() {
            out.push(current.trim_end().to_string());
            current.clear();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        out.push(current.trim_end().to_string());
    }
    out
}

/// 这条读请求是否作用于**单个索引**并返回一批文档行；是则返回索引名。
///
/// 只认 `GET|POST /<索引>/_search`：搜索命中就是索引里的原始文档，
/// `_id` 是命中的元数据（宿主的 `renderSearch` 已经把它作为首列放进了结果），
/// `_update/<id>` 正是按它更新。其余一律不认：
///
/// - `/_search`（全索引）与 `/<a>,<b>/_search`、通配 —— 没有「哪张索引」可言；
/// - `/_count`、`/_mapping`、`/logs/_doc/1` —— 不是「一批文档行」；
/// - 别名（`/myalias/_search`）无法在这一层分辨它指向几个索引，
///   但那条路不会改错数据：`_update/<id>` 遇到多索引别名会被 ES 直接拒绝。
pub fn single_index(text: &str) -> Option<String> {
    let statements = split_statements(text);
    if statements.len() != 1 {
        return None;
    }
    let request = parse(&statements[0])?;
    if !matches!(request.method.as_str(), "GET" | "POST") {
        return None;
    }
    if action(&request.path) != "_search" {
        return None;
    }
    let segments: Vec<&str> = request.path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() != 2 {
        return None;
    }
    let index = segments[0];
    if index.starts_with('_') || index.contains(',') || index.contains('*') {
        return None;
    }
    Some(index.to_string())
}

/// 整段是否只读：必须是单条请求且为只读。
pub fn is_read_only(text: &str) -> bool {
    let statements = split_statements(text);
    !statements.is_empty() && statements.iter().all(|s| classify(s).is_read_only())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 读请求被认出来() {
        for text in [
            "GET /logs/_search",
            "GET /_cat/indices?v",
            "GET /logs/_mapping",
            "HEAD /logs",
            "POST /logs/_search\n{ \"query\": { \"match_all\": {} } }",
            "POST /_msearch",
            "GET /_cluster/health",
            "GET /",
        ] {
            assert_eq!(classify(text), StatementKind::Read, "{text}");
        }
    }

    #[test]
    fn 写请求被认出来() {
        for text in [
            "POST /logs/_doc\n{ \"level\": \"info\" }",
            "PUT /logs/_doc/1\n{ \"level\": \"warn\" }",
            "POST /logs/_update/1",
            "POST /_bulk\n{ \"index\": {} }",
            "DELETE /logs/_doc/1",
            "POST /logs/_delete_by_query",
        ] {
            assert_eq!(classify(text), StatementKind::Write, "{text}");
        }
    }

    #[test]
    fn 结构变更被认出来() {
        for text in [
            "PUT /logs",
            "DELETE /logs",
            "PUT /logs/_mapping",
            "PUT /logs/_settings",
            "POST /logs/_close",
        ] {
            assert_eq!(classify(text), StatementKind::Ddl, "{text}");
        }
    }

    #[test]
    fn 判不出来的一律当非只读() {
        // 裸 JSON：没有方法就不知道是读还是写，不能猜
        assert_eq!(
            classify("{ \"query\": { \"match_all\": {} } }"),
            StatementKind::Unknown
        );
        assert_eq!(classify("PATCH /logs/_whatever"), StatementKind::Unknown);
        assert_eq!(classify(""), StatementKind::Unknown);
        assert_eq!(classify("SELECT 1"), StatementKind::Unknown);
        assert!(!is_read_only("{ \"query\": {} }"));
    }

    #[test]
    fn 请求体与路径同行也能判定() {
        // 单行写法很常见：不能把请求体当成路径的一部分
        assert_eq!(
            classify(r#"POST /logs/_search { "query": { "match_all": {} } }"#),
            StatementKind::Read
        );
        assert_eq!(
            classify(r#"POST /logs/_doc { "level": "info" }"#),
            StatementKind::Write
        );
        assert_eq!(
            classify(r#"DELETE /logs/_doc/1?refresh=true"#),
            StatementKind::Write
        );
        assert!(is_read_only(r#"GET /logs/_search?size=1"#));
    }

    #[test]
    fn 多条请求不能整体当只读() {
        let text = "GET /logs/_search\n\nDELETE /logs";
        assert_eq!(split_statements(text).len(), 2);
        assert!(!is_read_only(text));
        assert!(is_read_only("GET /logs/_search"));
        // 请求体里的内容不参与拆分
        let with_body = "POST /logs/_search\n{\n  \"query\": { \"match_all\": {} }\n}";
        assert_eq!(split_statements(with_body).len(), 1);
        assert!(is_read_only(with_body));
    }

    #[test]
    fn 只有单索引搜索才给出来源索引() {
        let ok = |text: &str, expect: &str| {
            assert_eq!(single_index(text), Some(expect.to_string()), "应认出：{text}");
        };
        ok("GET /logs/_search", "logs");
        ok("POST /logs/_search\n{ \"query\": { \"match_all\": {} } }", "logs");
        ok("GET /logs/_search?size=5", "logs");
        // 路径末的斜杠不影响
        ok("GET /logs/_search/", "logs");

        let none = |text: &str| {
            assert_eq!(single_index(text), None, "不该认出：{text}");
        };
        // 没有「哪张索引」可言的
        none("GET /_search");
        none("GET /logs,metrics/_search");
        none("GET /logs*/_search");
        // 不是「一批文档行」的读取
        none("GET /logs/_count");
        none("GET /logs/_mapping");
        none("GET /logs/_doc/1");
        none("GET /_cat/indices?v");
        // 写请求
        none("POST /logs/_update/1");
        // 多条
        none("GET /logs/_search\n\nGET /logs/_count");
        none("");
    }
}
