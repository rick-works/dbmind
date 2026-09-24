//! MongoDB 命令的只读判定。
//!
//! 与 SQL 不同，Mongo 的「语句」是一份 JSON 命令或 shell 风格调用，两种都要能判：
//!
//! ```text
//! 1) 原生命令     { "find": "users", "filter": { "age": { "$gt": 30 } } }
//! 2) shell 风格   db.users.find({ age: { $gt: 30 } }).limit(10)
//! 3) shell 助手   show collections / show dbs
//! ```
//!
//! **安全性原则与 SQL 侧完全一致：判不出来的一律按非只读处理。**
//! 这条不能松：Mongo 没有统一的语句分类元数据，命令别名又极多
//! （`deleteMany` / `findAndModify` / `bulkWrite` …），漏判一个就是数据事故。
//!
//! 一个容易漏的点：`aggregate` 通常是只读，但管道里含 `$out` / `$merge`
//! 时会把结果**写回集合** —— 这类必须按写处理。

use crate::sql::StatementKind;

/// 只读命令（元数据/查询）。名称先做归一化：小写、只留字母数字。
const READ_COMMANDS: &[&str] = &[
    // 查询
    "find",
    "findone",
    "count",
    "countdocuments",
    "estimateddocumentcount",
    "distinct",
    "aggregate",
    "getmore",
    "explain",
    "watch",
    "geonear",
    // 结构/元数据浏览
    "listcollections",
    "listindexes",
    "listdatabases",
    "dbstats",
    "collstats",
    "indexstats",
    "datasize",
    "validate",
    // 服务与账号信息
    "serverstatus",
    "buildinfo",
    "hostinfo",
    "connectionstatus",
    "getcmdlineopts",
    "ping",
    "hello",
    "ismaster",
    "usersinfo",
    "rolesinfo",
];

/// 数据变更命令。
const WRITE_COMMANDS: &[&str] = &[
    "insert",
    "insertone",
    "insertmany",
    "update",
    "updateone",
    "updatemany",
    "replaceone",
    "delete",
    "deleteone",
    "deletemany",
    "findandmodify",
    "findoneandupdate",
    "findoneandreplace",
    "findoneanddelete",
    "bulkwrite",
    "save",
    // mapReduce 默认会写输出集合，除非 inline —— 保守起见按写处理
    "mapreduce",
    "compact",
];

/// 结构与权限变更命令。
const DDL_COMMANDS: &[&str] = &[
    "create",
    "createcollection",
    "createindex",
    "createindexes",
    "createview",
    "createuser",
    "createrole",
    "drop",
    "dropdatabase",
    "dropcollection",
    "dropindex",
    "dropindexes",
    "dropuser",
    "droprole",
    "renamecollection",
    "collmod",
    "converttocapped",
    "clonecollectionascapped",
    "grantrolestouser",
    "revokerolesfromuser",
    "updaterole",
    "grantprivilegestorole",
    "revokeprivilegesfromrole",
    "shutdown",
    // 分片/副本集/参数：都算结构级变更，禁止在只读路径上执行
    "shardcollection",
    "enablesharding",
    "addshard",
    "removeshard",
    "addshardtag",
    "setfeaturecompatibilityversion",
    "setparameter",
    "replsetinitiate",
    "replsetreconfig",
    "replsetstepdown",
];

/// 命令名归一化：小写、去掉下划线与其它非字母数字字符。
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// 去掉 `//` 行注释与 `/* */` 块注释（字符串内部不处理，交给调用方保守处理）。
fn strip_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    let mut in_string: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(quote) = in_string {
            out.push(c);
            if c == quote {
                in_string = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            in_string = Some(c);
            out.push(c);
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 从 JSON 文本里取出第一个键名（命令文档的第一个键就是命令名）。
fn first_json_key(text: &str) -> Option<String> {
    let bytes: Vec<char> = text.chars().collect();
    let start = bytes.iter().position(|c| *c == '"')?;
    let mut name = String::new();
    let mut i = start + 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '\\' {
            // 键名里的转义极少见，保守返回 None 交给「未知」档
            return None;
        }
        if c == '"' {
            return Some(name);
        }
        name.push(c);
        i += 1;
    }
    None
}

/// 取出 `(` 之前紧邻的标识符：`db.users.findAndModify(...)` → `findAndModify`。
fn method_before_paren(text: &str) -> Option<String> {
    let paren = text.find('(')?;
    let head = &text[..paren];
    let name: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// 抽出命令名。识别不出返回 None。
pub fn command_name(text: &str) -> Option<String> {
    let cleaned = strip_comments(text);
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }

    // show collections / show dbs 之类：shell 助手，只读
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("show ") {
        return Some("show".to_string());
    }
    // use db：只是切换当前库，不产生读写
    if lower.starts_with("use ") {
        return None;
    }

    // 原生命令：{ "find": ... }
    if trimmed.starts_with('{') {
        return first_json_key(trimmed);
    }

    // shell 风格：db.xxx.method(...) / db.runCommand({...}) / db.adminCommand({...})
    let method = method_before_paren(trimmed)?;
    let normalized = normalize(&method);
    if matches!(normalized.as_str(), "runcommand" | "admincommand") {
        // 真正的命令在括号里的 JSON 文档中
        if let Some(open) = trimmed.find('(') {
            return first_json_key(&trimmed[open..]);
        }
    }
    Some(method)
}

/// 判断一条 Mongo 命令的类型。
pub fn classify(command: &str) -> StatementKind {
    let cleaned = strip_comments(command);
    let Some(name) = command_name(&cleaned) else {
        return StatementKind::Unknown;
    };
    let normalized = normalize(&name);

    if normalized == "show" {
        return StatementKind::Read;
    }
    // aggregate 写回集合的两种形式（$out / $merge）必须按写处理
    if normalized == "aggregate"
        && (cleaned.contains("$out") || cleaned.contains("$merge") || cleaned.contains("$Out"))
    {
        return StatementKind::Write;
    }
    if READ_COMMANDS.contains(&normalized.as_str()) {
        return StatementKind::Read;
    }
    if WRITE_COMMANDS.contains(&normalized.as_str()) {
        return StatementKind::Write;
    }
    if DDL_COMMANDS.contains(&normalized.as_str()) {
        return StatementKind::Ddl;
    }
    StatementKind::Unknown
}

/// 取「命令文档的第一个键」对应的**字符串值**：`{ "find": "users", … }` → `users`。
/// 值不是字符串（如 `{ "find": { … } }`）时返回 None。
fn first_json_string_value(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    // 跳过第一个键名（连同引号）
    let mut i = chars.iter().position(|c| *c == '"')? + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            // 键名里有转义极少见：保守放弃
            return None;
        }
        if chars[i] == '"' {
            i += 1;
            break;
        }
        i += 1;
    }
    // 键名之后必须是冒号
    while i < chars.len() && chars[i] != ':' {
        if !chars[i].is_whitespace() {
            return None;
        }
        i += 1;
    }
    i += 1;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if chars.get(i) != Some(&'"') {
        return None;
    }
    i += 1;
    let mut out = String::new();
    while i < chars.len() {
        match chars[i] {
            // 集合名里的转义极少见：保守放弃，不去猜它展开成什么
            '\\' => return None,
            '"' => return Some(out),
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    None
}

/// 取**开头**那个标识符，且它后面必须紧跟 `(`：`find({…})` → `find`。
///
/// 与 `method_before_paren` 的区别很关键：那个取的是「括号前最后一个标识符」
/// （用于 `db.users.findAndModify(…)` 取到 `findAndModify`，判命令名是对的）；
/// 而这里要的是**紧跟在集合后面**的第一个方法名 ——
/// 用错就会把 `db.my.coll.find(…)`（点号集合名）判成「集合 my、方法 find」，
/// 于是生成一条改到**别的集合**里去的语句。
fn leading_identifier(text: &str) -> Option<String> {
    let open = text.find('(')?;
    let name: String = text[..open]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// 这条读命令是否作用于**单个集合**；是则返回集合名。
///
/// 只认 `find` / `findOne`：这两种命令的结果就是集合里的**原始文档**，
/// `_id` 是存储值，可以据此唯一定位一行。`aggregate` 一律不接受 ——
/// 哪怕只写了 `$match`，`$project` / `$addFields` 也会让结果里的字段
/// **不再等于**存储的字段，照着它写回就是改错数据
///（与 SQL 侧不接受表达式/别名是同一条理由）。
///
/// 解析刻意严格：`db.<集合>.<方法>(` 里第一段是集合、第二段必须是方法。
/// 于是 `db.my.coll.find(…)`（点号集合名）会被判成「集合 my、方法 coll」——
/// 不是读方法 ⇒ 返回 None，而不是**猜成集合 my** 去改错地方。
pub fn single_collection(command: &str) -> Option<String> {
    let statements = crate::sql::split_statements(command);
    if statements.len() != 1 {
        return None;
    }
    let cleaned = strip_comments(&statements[0]);
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 原生命令形式：{ "find": "users", "filter": { … } }
    if trimmed.starts_with('{') {
        let name = first_json_key(trimmed)?;
        if !matches!(normalize(&name).as_str(), "find" | "findone") {
            return None;
        }
        let collection = first_json_string_value(trimmed)?;
        return if collection.is_empty() {
            None
        } else {
            Some(collection)
        };
    }

    // shell 风格：db.<集合>.<方法>(
    let rest = trimmed.strip_prefix("db.")?;
    let dot = rest.find('.')?;
    let collection = &rest[..dot];
    if collection.is_empty() {
        return None;
    }
    // 紧跟在集合后面的那个方法才是要用的（`db.users.find(…).limit(1)` → find）
    let method = leading_identifier(&rest[dot + 1..])?;
    if !matches!(normalize(&method).as_str(), "find" | "findone") {
        return None;
    }
    Some(collection.to_string())
}

/// 整段是否只读：**必须**单条且为只读。
pub fn is_read_only(command: &str) -> bool {
    let statements = super::sql::split_statements(command);
    !statements.is_empty() && statements.iter().all(|s| classify(s).is_read_only())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 原生命令按首键判定() {
        assert_eq!(
            classify(r#"{ "find": "users", "filter": {} }"#),
            StatementKind::Read
        );
        assert_eq!(classify(r#"{ "count": "users" }"#), StatementKind::Read);
        assert_eq!(classify(r#"{ "listCollections": 1 }"#), StatementKind::Read);
        assert_eq!(
            classify(r#"{ "insert": "users", "documents": [] }"#),
            StatementKind::Write
        );
        assert_eq!(classify(r#"{ "deleteMany": "users" }"#), StatementKind::Write);
        assert_eq!(classify(r#"{ "drop": "users" }"#), StatementKind::Ddl);
        assert_eq!(classify(r#"{ "createIndexes": "users" }"#), StatementKind::Ddl);
    }

    #[test]
    fn shell_风格按方法名判定() {
        assert_eq!(classify("db.users.find({ age: 30 })"), StatementKind::Read);
        assert_eq!(classify("db.users.findOne({ _id: 1 })"), StatementKind::Read);
        assert_eq!(
            classify("db.users.find().sort({ a: 1 }).limit(10)"),
            StatementKind::Read
        );
        assert_eq!(classify("db.users.insertOne({ a: 1 })"), StatementKind::Write);
        assert_eq!(classify("db.users.deleteMany({})"), StatementKind::Write);
        assert_eq!(
            classify("db.users.updateMany({}, { $set: { a: 1 } })"),
            StatementKind::Write
        );
        assert_eq!(
            classify("db.users.findAndModify({ query: {}, update: {} })"),
            StatementKind::Write
        );
        assert_eq!(classify("db.users.drop()"), StatementKind::Ddl);
        assert_eq!(classify("db.dropDatabase()"), StatementKind::Ddl);
        assert_eq!(classify("db.users.createIndex({ a: 1 })"), StatementKind::Ddl);
        assert_eq!(classify("show collections"), StatementKind::Read);
        assert_eq!(classify("show dbs"), StatementKind::Read);
    }

    #[test]
    fn runcommand_要看括号里的命令() {
        assert_eq!(classify(r#"db.runCommand({ "ping": 1 })"#), StatementKind::Read);
        assert_eq!(
            classify(r#"db.adminCommand({ "dropDatabase": 1 })"#),
            StatementKind::Ddl
        );
        assert_eq!(
            classify(r#"db.runCommand({ "insert": "t", "documents": [] })"#),
            StatementKind::Write
        );
    }

    #[test]
    fn aggregate_写回集合时算写() {
        // 普通管道：只读
        assert_eq!(
            classify(r#"db.orders.aggregate([{ "$match": { "a": 1 } }])"#),
            StatementKind::Read
        );
        // $out：把结果写进集合
        assert_eq!(
            classify(r#"db.orders.aggregate([{ "$match": {} }, { "$out": "big" }])"#),
            StatementKind::Write
        );
        // $merge：同样写回
        assert_eq!(
            classify(r#"db.orders.aggregate([{ "$merge": { "into": "big" } }])"#),
            StatementKind::Write
        );
    }

    #[test]
    fn 判不出来的一律当非只读() {
        assert_eq!(classify("db.users.someFutureCommand({})"), StatementKind::Unknown);
        assert_eq!(classify(r#"{ "brandNewAdminThing": 1 }"#), StatementKind::Unknown);
        assert_eq!(classify("use admin"), StatementKind::Unknown);
        assert_eq!(classify(""), StatementKind::Unknown);
        assert!(!is_read_only("db.users.someFutureCommand({})"));
    }

    #[test]
    fn 注释与空白不影响判定() {
        assert_eq!(classify("// 查一下\ndb.users.find({})"), StatementKind::Read);
        assert_eq!(classify("/* block */ db.users.drop()"), StatementKind::Ddl);
        assert_eq!(classify("   db.users.count({})   "), StatementKind::Read);
    }

    #[test]
    fn 多条命令不算只读() {
        assert!(!is_read_only("db.users.find({}); db.users.drop()"));
        assert!(is_read_only("db.users.find({})"));
    }

    #[test]
    fn 只有单集合的原始文档查询才给出来源集合() {
        let ok = |text: &str, expect: &str| {
            assert_eq!(
                single_collection(text),
                Some(expect.to_string()),
                "应认出来源：{text}"
            );
        };
        ok("db.users.find({})", "users");
        ok("db.users.findOne({ _id: 1 })", "users");
        ok("db.users.find({ age: 30 }).sort({ age: 1 }).limit(10)", "users");
        ok(r#"{ "find": "users", "filter": { "age": 30 } }"#, "users");
        ok(r#"{ "findOne": "users" }"#, "users");
        ok("// 注释\ndb.订单.find({})", "订单");
        // 投影不影响定位：`_id` 是**存储值**，照它定位是对的。
        //（投影里写了 `_id: 0` 时结果里根本没有 `_id` 列，
        // 那一层由「结果里有没有行标识列」去挡，不在这里判。）
        ok("db.users.find({}, { name: 1 })", "users");
        // 集合名恰好叫 find 也要认对
        ok("db.find.find({})", "find");

        let none = |text: &str| {
            assert_eq!(single_collection(text), None, "不该认出来源：{text}");
        };
        // 聚合：字段可能被 $project / $addFields 改写 ⇒ 照着写回就是改错数据
        none(r#"db.users.aggregate([{ "$match": {} }])"#);
        none(r#"{ "aggregate": "users", "pipeline": [], "cursor": {} }"#);
        // 写命令（引擎只对读语句取来源，这里再挡一层）
        none("db.users.updateOne({}, { $set: { a: 1 } })");
        none("db.users.deleteMany({})");
        // 点号集合名：会被判成「集合 my、方法 coll」⇒ 不是读方法 ⇒ 不认。
        // 关键是**不猜成集合 my** —— 猜错就是改到别的集合里去。
        none("db.my.coll.find({})");
        // 非单条 / 其它命令 / 空
        none("db.a.find({}); db.b.find({})");
        none("db.users.count({})");
        none("db.users.distinct(\"x\")");
        none("show collections");
        none("");
    }
}
