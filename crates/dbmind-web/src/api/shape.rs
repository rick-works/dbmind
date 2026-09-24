//! 内核形状 →上游形状的转换，**只在这一层做**。
//!
//! 为什么必须有这一层：内核与上游是两套各自自洽的模型，字段名和粒度都不同。
//! 转换散落到各 handler 里就会出现「这个接口转了、那个忘了」，界面上表现为
//! 某一列是空的、某个角标不亮 —— 都是看不出原因的那种坏。
//!
//! 三处**最容易错、且错了很难发现**的地方，逐条记在这里：
//!
//! 1. **行是「列名 → 值」的对象，不是数组**。上游的表格组件按列名取值
//!    （`row[col]`），内核给的是按列序的数组。返回数组的后果是**列名对、行数对、
//!    每一格都显示 NULL** —— 只有真跑起来看数据才抓得到。
//! 2. **`executeTime` 不是 `durationMs`**。上游的结果页读 `result.executeTime`，
//!    给别的名字就是「耗时永远是空」。
//! 3. **连接类型必须大写**（`MYSQL`）。前端的类型注册表 `src/types/*.js` 全按大写索引，
//!    给内核的小写 key 会导致 JDBC URL 预览为空、Logo 退化、引用符/默认 schema 全走默认值。

use dbmind_core::{
    CellValue, ColumnDetail, ConnectionKind, ConnectionRecord, QueryResult, TableInfo, TableKind,
};
use serde_json::{json, Map, Value};

/// 连接分组（树里的「目录」）与「环境角标」存在连接的 `extra` 里。
///
/// 内核的 `extra` 是**透传给驱动**的开放字段（`extra.params` → JDBC Properties），
/// 所以往里放界面偏好是安全的：驱动只读 `params`，其余键无人过问。
///上游把这些放在连接记录本身，这里等价落位，语义一致。
pub const EXTRA_ENVIRONMENT: &str = "environment";
pub const EXTRA_ENV: &str = "env";
/// 认证方式（SQL Server：`sqlserver` | `windows`）。存在 `extra` 里，见 conn.rs 的说明。
pub const EXTRA_AUTH_TYPE: &str = "authType";

/// 未分组时的默认目录名。
///
/// **不能给空串**：上游的树只渲染 `envOrder = [自定义目录..., DEV, TEST, PROD]` 里存在的分组，
/// 而自定义目录来自「所有连接的 environment 中非空、非预置的值」。
/// 给空串 ⇒ 该分组不在 envOrder 里 ⇒ **连接在树上根本不显示**（接口却全是 200）。
pub const DEFAULT_ENVIRONMENT: &str = "数据源";

pub fn extra_str(record: &ConnectionRecord, key: &str) -> Option<String> {
    record
        .config
        .extra
        .as_ref()
        .and_then(|extra| extra.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// 内核的 kind（`sqlserver`）→上游的类型码（`SQLSERVER`）。
pub fn type_code(kind: ConnectionKind) -> String {
    kind.key().to_ascii_uppercase()
}

/// 单格值 → `i64`（取不到返回 `None`）。
///
/// 三种形态**都要认**，因为各家驱动回同一个数值的方式并不统一：
/// - 整数：MySQL / SQL Server 的 `count(*)`；
/// - **浮点**：ClickHouse 的 `count(*)` 回来是 `Real`（实测 `t=real v=4.0`）——
///   而 `serde_json` 的 `as_i64()` 对浮点型 Number **返回 None**；
/// - 字符串：有的驱动把大整数当字符串回。
///
/// 以前各处直接写 `...as_i64().unwrap_or(0)`：浮点与字符串都取不出来，于是**静默变成 0**。
/// 后果不是"少一个数字"，而是**把"不知道"伪装成"是 0"**：
/// ClickHouse 上树的每张表都显示 0 行，质量规则则永远判"通过"。
///
/// 所以这里返回 `Option`：调用方必须自己决定取不到时怎么表达（留空 / 报错 / 未知），
/// 而不是拿 0 顶替。
pub fn value_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value as i64)),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// 单格值 → 裸值。二进制只报大小，不尝试解码（解出来也是乱码）。
pub fn cell_to_value(cell: &CellValue) -> Value {
    match cell {
        CellValue::Null => Value::Null,
        CellValue::Integer(v) => json!(v),
        CellValue::Real(v) => json!(v),
        CellValue::Text(v) => json!(v),
        CellValue::Blob { len } => json!(format!("[blob {len} B]")),
    }
}

// ==================== 值比较的规范化（跨类型对比 / 同步共用） ====================
//
// 为什么必须有这一层：**同一个逻辑值，不同驱动会给出不同的变体或文本表示**，
// 而"这两个值是不是同一个"必须只看值本身。两类最要命的：
//
// 1. **数值**：MySQL `INT` → `Integer`，Oracle `NUMBER(p,s)` → `Real`
//    （见 agents 侧 `ResultMapper`：BigDecimal 按 scale 分派 integer/real）。
//    于是 `json!(3) != json!(3.0)`、旧键前缀 `i:` 与 `f:` 也不同 ——
//    「MySQL INT ↔ Oracle NUMBER」这种跨类型对比会把**整表**判成"仅源 / 仅目标"，
//    而且**不报错**（静默的错结果，比报错坏得多）。
// 2. **日期时间**：Oracle `DATE` → `2024-01-01T00:00`、MySQL `DATE` → `2024-01-01`、
//    PG `timestamp` → `2024-01-01T00:00:00` —— 同一个时刻三种写法。
//
// 归一之后，比较（`cells_equal`）与建键（`cell_key`）都只认规范形式。

/// 数值 → 规范十进制串：**整型与实数同形**（`3` 与 `3.0` 都得到 `"3"`）。
///
/// 非数值返回 `None`（调用方据此保持原来的"按类型严格比"语义）。
/// 注意 f64 的精度边界：超出 2^53 的整数无法精确表示，两侧都超界时仍可能不等 —— 属已知边界。
pub fn number_key(cell: &CellValue) -> Option<String> {
    match cell {
        CellValue::Integer(value) => Some(value.to_string()),
        CellValue::Real(value) => {
            if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.007_199_254_740_992e15 {
                Some((*value as i64).to_string())
            } else {
                Some(format!("{value}"))
            }
        }
        _ => None,
    }
}

/// 日期/时间文本 → 规范形式 `yyyy-MM-ddTHH:mm:ss[.ffffff]`（没有时间部分的补 `00:00:00`）。
///
/// 只认"长得像日期时间"的文本；其它一律 `None`，调用方按原样比较 ——
/// 也就是说普通字符串列**不受影响**，只有真的像日期时间的值才会被归一。
pub fn normalize_datetime(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let digit = |index: usize| bytes.get(index).is_some_and(u8::is_ascii_digit);
    // 日期部分必须严格是 yyyy-MM-dd
    if !(digit(0) && digit(1) && digit(2) && digit(3) && bytes[4] == b'-'
        && digit(5) && digit(6) && bytes[7] == b'-' && digit(8) && digit(9))
    {
        return None;
    }
    let two = |value: &str| value.len() == 2 && value.bytes().all(|c| c.is_ascii_digit());
    // 光看"位数对不对"不够：`2024-13-99` 这种也要挡掉 —— 否则普通文本里长得像日期的串
    // 会被当成时间归一，反而制造假的相等。范围校验够用（02-30 这种不查）。
    let number = |text: &str| text.parse::<u32>().ok();
    let month = number(&trimmed[5..7])?;
    let day = number(&trimmed[8..10])?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let rest = trimmed[10..].trim_start_matches(['T', 't', ' ']).trim();
    let mut out = String::with_capacity(26);
    out.push_str(&trimmed[..10]);
    out.push('T');
    if rest.is_empty() {
        out.push_str("00:00:00");
        return Some(out);
    }
    let (head, frac) = match rest.split_once('.') {
        Some((head, frac)) => (head.trim(), Some(frac)),
        None => (rest, None),
    };
    let mut parts = head.split(':');
    let hour = parts.next()?;
    let minute = parts.next()?;
    let second = parts.next().unwrap_or("00");
    if parts.next().is_some() || !two(hour) || !two(minute) || !two(second) {
        return None;
    }
    // 时间部分同样做范围校验（`25:99:99` 不是时间）
    if number(hour)? > 23 || number(minute)? > 59 || number(second)? > 60 {
        return None;
    }
    out.push_str(hour);
    out.push(':');
    out.push_str(minute);
    out.push(':');
    out.push_str(second);
    if let Some(frac) = frac {
        // 秒的小数部分：最多留 6 位（微秒），并去掉尾随 0（`00` 与 `000000` 都当没有）
        let digits: String = frac.chars().take_while(char::is_ascii_digit).take(6).collect();
        let digits = digits.trim_end_matches('0');
        if !digits.is_empty() {
            out.push('.');
            out.push_str(digits);
        }
    }
    Some(out)
}

/// 单格值 → 比较/建键用的规范串。
///
/// 前缀刻意区分类型：`d:` 数值、`t:` 日期时间、`s:` 其它文本、`n:` 空、`b:` 二进制 ——
/// 所以「INT 3」与「VARCHAR '3'」仍然是**两种值**（这条语义不能被归一冲掉）。
pub fn cell_key(cell: &CellValue) -> String {
    match cell {
        CellValue::Null => "n:".to_string(),
        CellValue::Blob { len } => format!("b:{len}"),
        CellValue::Text(text) => match normalize_datetime(text) {
            Some(normalized) => format!("t:{normalized}"),
            None => format!("s:{text}"),
        },
        _ => match number_key(cell) {
            Some(number) => format!("d:{number}"),
            None => "n:".to_string(),
        },
    }
}

/// 两格是否相等（跨类型语义）：数值跨变体按数值比、日期时间按规范形式比、
/// 其余仍按"类型严格"比（文本 `'3'` ≠ 数值 `3`）。
pub fn cells_equal(left: Option<&CellValue>, right: Option<&CellValue>) -> bool {
    match (left, right) {
        (Some(CellValue::Blob { len: a }), Some(CellValue::Blob { len: b })) => a == b,
        (Some(a), Some(b)) => cell_key(a) == cell_key(b),
        (None, None) => true,
        _ => false,
    }
}

/// 组合键：多列主键 → 一个可比较的字符串（分隔符用不可见字符，避免 `a|b` 与 `a|b|c` 撞车）。
pub fn row_key(row: &[CellValue], indexes: &[usize]) -> String {
    indexes
        .iter()
        .map(|index| {
            row.get(*index)
                .map(cell_key)
                .unwrap_or_else(|| "n:".to_string())
        })
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

#[cfg(test)]
mod value_tests {
    use super::*;

    /// 数值跨变体：MySQL 的 `INT` 回 `Integer`、Oracle 的 `NUMBER(p,s)` 回 `Real` —— 必须是同一个值。
    ///
    /// 这条守着"跨类型对比把整表判成仅源/仅目标"的静默错误。
    #[test]
    fn 数值跨变体的键与比较() {
        let int = CellValue::Integer(3);
        let real = CellValue::Real(3.0);
        assert_eq!(cell_key(&int), cell_key(&real));
        assert!(cells_equal(Some(&int), Some(&real)));
        // 「类型严格」这条不能被归一冲掉：文本 '3' 与数值 3 仍是两种值
        let text = CellValue::Text("3".to_string());
        assert_ne!(cell_key(&text), cell_key(&int));
        assert!(!cells_equal(Some(&text), Some(&int)));
        // 真正不同的数值仍然不等
        assert!(!cells_equal(Some(&int), Some(&CellValue::Real(3.5))));
        // 组合键同理
        assert_eq!(
            row_key(&[CellValue::Integer(3), CellValue::Text("a".into())], &[0, 1]),
            row_key(&[CellValue::Real(3.0), CellValue::Text("a".into())], &[0, 1])
        );
    }

    /// 日期时间跨表示：`2024-01-01` / `2024-01-01T00:00` / `...T00:00:00` / `... 00:00:00` 是同一个值。
    #[test]
    fn 日期时间跨表示的归一() {
        let a = CellValue::Text("2024-01-01".to_string());
        let b = CellValue::Text("2024-01-01T00:00".to_string());
        let c = CellValue::Text("2024-01-01T00:00:00".to_string());
        let d = CellValue::Text("2024-01-01 00:00:00".to_string());
        assert_eq!(cell_key(&a), cell_key(&b));
        assert_eq!(cell_key(&b), cell_key(&c));
        assert_eq!(cell_key(&c), cell_key(&d));
        assert!(cells_equal(Some(&a), Some(&d)));
        // 秒的小数尾随 0 也要归一
        assert_eq!(
            cell_key(&CellValue::Text("2024-01-01T10:20:30.500".to_string())),
            cell_key(&CellValue::Text("2024-01-01T10:20:30.5".to_string()))
        );
        // 真的不同时刻仍然不等
        assert!(!cells_equal(Some(&a), Some(&CellValue::Text("2024-01-02T00:00:00".to_string()))));
        // 普通文本不受影响（不会被误当日期）
        assert_ne!(cell_key(&CellValue::Text("abc".to_string())), cell_key(&a));
        assert_eq!(normalize_datetime("abc"), None);
        assert_eq!(normalize_datetime("2024-13-99"), None);
    }
}

/// 内核 `QueryResult` →上游`QueryResult`（见 `上游-core` 的 entity 类）。
pub fn query_result_json(result: &QueryResult) -> Value {
    let columns: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
    let column_types: Vec<String> = result
        .columns
        .iter()
        .map(|c| c.type_name.clone().unwrap_or_default())
        .collect();
    let rows: Vec<Value> = result
        .rows
        .iter()
        .map(|raw| {
            let mut row = Map::new();
            for (i, name) in columns.iter().enumerate() {
                let value = raw.get(i).map(cell_to_value).unwrap_or(Value::Null);
                row.insert(name.clone(), value);
            }
            Value::Object(row)
        })
        .collect();

    //上游用 `affectedRows >= 0 && message` 判定「这是一条影响行数的语句」；
    // 内核在只读语句上给 None，这里翻成 -1，语义与它一致。
    let affected_rows = result.affected_rows.map(|rows| rows as i64).unwrap_or(-1);
    json!({
        "columns": columns,
        "columnTypes": column_types,
        "rows": rows,
        "rowCount": result.row_count as i64,
        "totalCount": if result.truncated { -1 } else { result.row_count as i64 },
        "affectedRows": affected_rows,
        "executeTime": result.duration_ms as i64,
        "message": message_of(result, affected_rows),
        "success": true,
        "notices": result.notices,
        "failedSql": Value::Null,
        "sqlState": Value::Null,
        "errorCode": 0,
        "hasMore": result.truncated,
    })
}

fn message_of(result: &QueryResult, affected_rows: i64) -> String {
    if affected_rows >= 0 {
        return format!("Query OK, {affected_rows} rows affected");
    }
    format!("Query OK, {} rows returned", result.row_count)
}

/// 失败时的上游`QueryResult`（前端会把它当结果渲染并显示红色错误块）。
pub fn query_failure_json(message: &str, execute_time: u64) -> Value {
    json!({
        "columns": [],
        "columnTypes": [],
        "rows": [],
        "rowCount": 0,
        "totalCount": -1,
        "affectedRows": -1,
        "executeTime": execute_time as i64,
        "message": message,
        "success": false,
        "notices": [],
        "failedSql": Value::Null,
        "sqlState": Value::Null,
        "errorCode": 0,
        "hasMore": false,
    })
}

/// 内核 `TableInfo` →上游`TableInfo`（树的「表 / 视图」分类按 `type === 'VIEW'` 分）。
pub fn table_json(info: &TableInfo) -> Value {
    json!({
        "schema": Value::Null,
        "name": info.name,
        "type": if info.kind == TableKind::View { "VIEW" } else { "TABLE" },
        "comment": Value::Null,
        // 行数：内核拿不到就是 null（**不是 0**）。0 表示「这张表是空的」，
        // null 表示「不知道」—— 两者在界面上必须能区分（留空 vs 显示 0）。
        // 精确值由前端展开后的批量 COUNT(*) 回填（见 `/table-count`）。
        "rows": info.row_estimate,
        "engine": Value::Null,
        "charset": Value::Null,
        // ClickHouse 的排序键 / 分区键：建表时确定、之后改不了，界面只读展示。
        // 初值 null，由 `/tables` 的表选项富化按方言回填（拿不到就一直是 null）。
        "sortingKey": Value::Null,
        "partitionKey": Value::Null,
        "createTime": Value::Null,
        "columns": [],
    })
}

/// 内核 `ColumnDetail` →上游`DbColumn`。
pub fn column_json(detail: &ColumnDetail) -> Value {
    let type_name = detail.type_name.clone().unwrap_or_default();
    json!({
        "name": detail.name,
        "type": type_name,
        "nullable": detail.nullable,
        "key": if detail.primary_key { "PRI" } else { "" },
        "defaultValue": detail.default_value,
        "comment": Value::Null,
        "extra": "",
        "autoIncrement": false,
        "primaryKey": detail.primary_key,
        "sortKey": false,
        "ordinal": 0,
    })
}

/// 内核连接记录 →上游`ConnectionInfo`。
///
/// 关于 `hasPassword`：上游用它来决定「留空表示不修改」，而**口令本身从不回传**
/// （内核的 `ConnectionConfig::password` 标了 `skip_serializing`）。
/// 所以这里用 `username` 是否存在来判断「这条连接配过凭证」—— 对需要认证的库这是等价信息，
/// 对 SQLite 这类无凭证的类型则恒为 false，符合事实。
pub fn connection_json(record: &ConnectionRecord) -> Value {
    let config = &record.config;
    let params: Vec<Value> = config
        .extra
        .as_ref()
        .and_then(|extra| extra.get("params"))
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(k, v)| json!({ "name": k, "value": v.as_str().unwrap_or_default() }))
                .collect()
        })
        .unwrap_or_default();
    let environment = extra_str(record, EXTRA_ENVIRONMENT).unwrap_or_else(|| DEFAULT_ENVIRONMENT.to_string());
    let env = extra_str(record, EXTRA_ENV).unwrap_or_default();

    json!({
        "id": record.id,
        "name": config.name,
        "type": type_code(record.kind()),
        "host": config.host.clone().unwrap_or_default(),
        "port": config.port,
        "database": config.database.clone().unwrap_or_default(),
        "username": config.username.clone().unwrap_or_default(),
        "authType": extra_str(record, EXTRA_AUTH_TYPE).unwrap_or_default(),
        "filePath": config.file_path.clone().unwrap_or_default(),
        "charset": "UTF-8",
        "remark": "",
        "environment": environment,
        "env": env,
        "jdbcUrl": "",
        "params": params,
        "connectTimeout": 10,
        "socketTimeout": 600,
        "writeTimeout": 300,
        "esProtocol": "http",
        "sshEnabled": false,
        "sshHost": "",
        "sshPort": Value::Null,
        "sshUser": "",
        "sshAuthType": "password",
        "sshPassword": "",
        "sshKeyPath": "",
        "sshKeyPassphrase": "",
        "hasPassword": config.username.as_deref().map(|u| !u.is_empty()).unwrap_or(false),
        "hasSshPassword": false,
        // 备注：存在 `extra.note` 里（与 environment/env 同一处），没写就是空串
        "note": config
            .extra
            .as_ref()
            .and_then(|extra| extra.get("note"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        "readOnly": record.read_only,
        "color": config.color.clone().unwrap_or_default(),
        "createTime": record.created_at,
        "updateTime": record.updated_at,
    })
}

/// 把内核 `TableInfo` 列表转成上游形状的数组。
pub fn tables_json(tables: &[TableInfo]) -> Value {
    Value::Array(tables.iter().map(table_json).collect())
}

pub fn columns_json(columns: &[ColumnDetail]) -> Value {
    Value::Array(columns.iter().map(column_json).collect())
}
