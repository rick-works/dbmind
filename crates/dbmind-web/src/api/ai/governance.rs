//! `/api/ai/governance/*` —— 数据治理（**规则引擎，几乎零 AI 成本**）。
//!
//! | 端点 | 做法 |
//! |---|---|
//! | `sensitive` | 按**字段名 + 注释**匹配敏感模式（身份证/银行卡/手机号/邮箱/住址/薪资/密码…） |
//! | `capacity` | 方言各自的系统表取行数与占用空间 |
//! | `relations` | 按**命名约定**推导表间关联（`xxx_id` → 表 `xxx`） |
//! | `er-graph` | 表（含完整列清单）+ 关系，形状由前端 ER 图直接消费（`tables` / `relations`） |
//! | `impact` | relations 的反向闭包（改这张表会波及谁）+ 视图依赖 |
//! | `quality/rules` | 按列的类型/可空/命名生成质量规则建议（可选让模型补充） |
//! | `quality/check` | 单表试跑（复用 quality 的规则引擎） |
//!
//! ## 为什么这一块不调模型
//!
//! 敏感字段识别、容量排行、关联推导，答案**由元数据决定**，模型加不了信息量，
//! 反而会引入不确定性（同一张表两次扫描给出不同结论，用户就没法信任它）。
//! 模型只在「按业务语义补充规则建议」这一处出场，且**失败不影响规则产出**
//! （规则引擎先给一版，模型只做增补）。
//!
//! ## 一条必须说清的边界
//!
//! **关系是「推导」的，不是数据库里声明的外键。** 内核没有暴露外键元数据，
//! 所以这里的关联全部来自命名约定，`confidence` 与 `kind` 字段就是在标注这件事 ——
//! 界面上必须能看出「这条线是猜的」。把猜的当真的用，会得出错误的删除顺序。

use axum::Json;
use dbmind_core::ColumnDetail;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::ai::tools::{columns_of, tables_of};
use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::meta::{rows_of, run_sql_in};
use crate::api::require_record;
use crate::AppState;

const MAX_TABLES_DEFAULT: usize = 200;
const MAX_TABLES_HARD: usize = 500;
/// 「系统行数估算为 0」时，最多为多少张表补一次实时 `count(*)`（见 `capacity`）。
///
/// 之所以要有这个上限：InnoDB 的 `table_rows` 只是统计信息里的**估算值**，表刚建好、
/// 批量写入后还没生成统计时它返回 0，于是整库每张表都是 0。补统计是必要的，
/// 但也不能无限制地逐表 `count(*)` —— 500 张表的大库会把「统计容量」变成一整库扫描。
const ROW_COUNT_FALLBACK_LIMIT: usize = 50;
/// 视图依赖检查最多看多少个视图（每个都要取一次定义）。
const MAX_VIEWS: usize = 30;

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GovReq {
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub table: Option<String>,
    #[serde(default)]
    pub tables: Option<Vec<String>>,
    #[serde(default)]
    pub max_tables: Option<usize>,
    #[serde(default)]
    pub model_id: Option<String>,
}

impl GovReq {
    pub(crate) fn conn(&self) -> XResult<String> {
        self.connection_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))
    }

    pub(crate) fn db(&self) -> String {
        self.database.clone().unwrap_or_default()
    }

    fn limit(&self) -> usize {
        self.max_tables
            .unwrap_or(MAX_TABLES_DEFAULT)
            .clamp(1, MAX_TABLES_HARD)
    }
}

/// 要扫描的表：请求显式给了就用它，否则按上限取库里的表。
async fn scan_tables(
    state: &AppState,
    req: &GovReq,
) -> XResult<(String, String, Vec<String>, usize)> {
    let conn = req.conn()?;
    let database = req.db();
    let all = tables_of(state, &conn, &database).await?;
    let names: Vec<String> = match &req.tables {
        Some(list) if !list.is_empty() => list
            .iter()
            .filter(|name| all.iter().any(|table| &table.name == *name))
            .cloned()
            .collect(),
        _ => all
            .iter()
            .take(req.limit())
            .map(|table| table.name.clone())
            .collect(),
    };
    Ok((conn, database, names, all.len()))
}

// ------------------------------------------------------------------ 敏感数据

struct SensitiveRule {
    label: &'static str,
    /// 与前端 `levelText` 对齐：high / medium / low / info
    level: &'static str,
    hint: &'static str,
    keys: &'static [&'static str],
}

/// 敏感字段模式表。
///
/// 刻意**不用裸 `name`**：几乎每张表都有 `name`/`title`，把它当「姓名」会让报告里
/// 塞满噪音（用户第一次用就觉得不准，之后就不再看了）。
/// 「证据」里会写出命中的是哪一段文字，方便人工复核。
const SENSITIVE: &[SensitiveRule] = &[
    SensitiveRule {
        label: "身份证号",
        level: "high",
        hint: "仅保留前 6 位与后 4 位，中间以 * 代替",
        keys: &["id_card", "idcard", "identity_no", "cert_no", "sfzh", "身份证"],
    },
    SensitiveRule {
        label: "银行卡号",
        level: "high",
        hint: "只展示后 4 位（如 **** **** 1234）",
        keys: &["bank_card", "card_no", "cardno", "bank_account", "银行卡", "开户账号"],
    },
    SensitiveRule {
        label: "密码 / 密钥",
        level: "high",
        hint: "禁止明文存储与展示，只存加盐哈希",
        keys: &["password", "passwd", "pwd", "secret", "access_key", "密码", "密钥"],
    },
    SensitiveRule {
        label: "手机号",
        level: "medium",
        hint: "中间四位以 * 代替（如 138****8888）",
        keys: &["phone", "mobile", "tel_no", "telephone", "手机", "联系电话"],
    },
    SensitiveRule {
        label: "电子邮箱",
        level: "medium",
        hint: "@ 前只保留首字符（如 a***@example.com）",
        keys: &["email", "e_mail", "mail_addr", "邮箱"],
    },
    SensitiveRule {
        label: "薪资",
        level: "medium",
        hint: "按角色控制可见范围，普通查询只返回区间",
        keys: &["salary", "wage", "annual_pay", "薪资", "工资", "年薪"],
    },
    SensitiveRule {
        label: "家庭住址",
        level: "low",
        hint: "按需脱敏到「市/区」粒度",
        keys: &["home_address", "address_detail", "住址", "家庭地址"],
    },
    SensitiveRule {
        label: "姓名",
        level: "info",
        hint: "姓氏保留、名字以 * 代替（张*）",
        keys: &["real_name", "true_name", "customer_name", "contact_name", "姓名", "联系人"],
    },
];

/// `POST /api/ai/governance/sensitive` —— 敏感字段扫描。
pub async fn scan_sensitive(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<GovReq>,
) -> XResult<Json<Value>> {
    let (conn, database, names, scanned_all) = scan_tables(&state, &req).await?;
    let mut findings: Vec<Value> = Vec::new();
    let mut scanned = 0usize;
    for table in &names {
        let columns = columns_of(&state, &conn, &database, table).await?;
        if columns.is_empty() {
            continue;
        }
        scanned += 1;
        // 注释按表批量取一次（每列一次往返在大库上是几十倍的差距）
        let comments = table_comments(&state, &conn, &database, table).await;
        for column in &columns {
            let name = column.name.to_ascii_lowercase();
            let comment = comments
                .get(&column.name.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default();
            let comment_lower = comment.to_ascii_lowercase();
            if let Some(rule) = SENSITIVE.iter().find(|rule| {
                rule.keys
                    .iter()
                    .any(|key| name.contains(key) || comment_lower.contains(key))
            }) {
                let evidence = if rule
                    .keys
                    .iter()
                    .any(|key| name.contains(key))
                {
                    format!("字段名包含「{}」", column.name)
                } else {
                    format!("字段注释包含「{}」", comment.trim())
                };
                findings.push(json!({
                    "level": rule.level,
                    "table": table,
                    "column": column.name,
                    "columnType": column.type_name,
                    "comment": comment,
                    "sensitiveLabel": rule.label,
                    "evidence": evidence,
                    "maskHint": rule.hint,
                }));
            }
        }
    }
    // 高危在前：用户点开这份报告，第一眼该看到的是最该处理的
    let rank = |level: &str| match level {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 3,
    };
    findings.sort_by_key(|item| {
        rank(item["level"].as_str().unwrap_or(""))
    });
    let high = findings
        .iter()
        .filter(|item| item["level"] == json!("high"))
        .count();
    Ok(Json(json!({
        "success": true,
        "scanned": scanned,
        "tablesTotal": scanned_all,
        "findings": findings,
        "summary": {
            "high": high,
            "medium": findings.iter().filter(|item| item["level"] == json!("medium")).count(),
            "low": findings.iter().filter(|item| item["level"] == json!("low")).count(),
            "info": findings.iter().filter(|item| item["level"] == json!("info")).count(),
        },
        "note": "依据字段名与注释匹配，未抽样数据内容；命中的规则可在结果里看到「依据」",
    })))
}

/// 一张表的列注释：`列名(小写) → 注释`。
///
/// 内核的 `ColumnDetail` 不带注释（它不是所有方言都能便宜拿到），所以这里按方言取一次；
/// **取不到就返回空表**，匹配退化到只看字段名 —— 不编造、也不因此让扫描失败。
async fn table_comments(
    state: &AppState,
    conn: &str,
    database: &str,
    table: &str,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Ok(record) = require_record(state, conn).await else {
        return out;
    };
    let dialect = Dialect::new(record.kind());
    let Some(sql) = dialect.table_comments(table).sql() else {
        return out;
    };
    let Ok(result) = run_sql_in(state, conn, database, sql, 1000).await else {
        return out;
    };
    for row in rows_of(&result) {
        let name = row
            .get("column_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let comment = row
            .get("comment")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if !name.is_empty() && !comment.is_empty() {
            out.insert(name, comment);
        }
    }
    out
}

// ------------------------------------------------------------------ 容量

/// 各方言的「每张表多少行、占多少字节、注释是什么」。
fn capacity_sql(kind: &str) -> Option<&'static str> {
    match kind {
        "mysql" | "mariadb" | "doris" => Some(
            "select table_name as t, table_rows as num_rows, \
             (coalesce(data_length, 0) + coalesce(index_length, 0)) as bytes, \
             coalesce(table_comment, '') as comment \
             from information_schema.tables \
             where table_schema = database() and table_type = 'BASE TABLE'",
        ),
        "postgresql" | "kingbase" => Some(
            "select c.relname as t, coalesce(s.n_live_tup, 0) as num_rows, \
             pg_total_relation_size(c.oid) as bytes, \
             coalesce(obj_description(c.oid, 'pg_class'), '') as comment \
             from pg_class c \
             left join pg_stat_user_tables s on s.relid = c.oid \
             where c.relkind = 'r' \
               and c.relnamespace = (select oid from pg_namespace where nspname = current_schema())",
        ),
        // SQL Server 用 OUTER APPLY 逐表算，而不是 GROUP BY：
        // 一旦 GROUP BY，select 里的 `coalesce(ep.value, '')` 与 group by 里的 `ep.value`
        // 就被当成两个表达式（SQL Server 要求完全一致），直接报「列没有包含在聚合函数或 GROUP BY 子句中」。
        "sqlserver" => Some(
            "select t.name as t, \
             cast(coalesce(p.rows, 0) as bigint) as num_rows, \
             cast(coalesce(a.bytes, 0) as bigint) as bytes, \
             cast(coalesce(ep.value, '') as nvarchar(400)) as comment \
             from sys.tables t \
             outer apply (select sum(rows) as rows from sys.partitions \
                          where object_id = t.object_id and index_id in (0, 1)) p \
             outer apply (select sum(au.total_pages) * 8 * 1024 as bytes \
                          from sys.allocation_units au \
                          join sys.partitions pp on pp.partition_id = au.container_id \
                          where pp.object_id = t.object_id and pp.index_id in (0, 1)) a \
             left join sys.extended_properties ep \
                    on ep.major_id = t.object_id and ep.minor_id = 0 and ep.name = 'MS_Description'",
        ),
        "oracle" | "dm" => Some(
            "select table_name as t, coalesce(num_rows, 0) as num_rows, \
             coalesce((select sum(bytes) from user_segments s where s.segment_name = u.table_name), 0) as bytes, \
             '' as comment from user_tables u",
        ),
        // 文件型 / 嵌入式库没有「表占用空间」的口径：如实说没有，而不是编一个 0
        _ => None,
    }
}

fn human_bytes(bytes: i64) -> String {
    let value = bytes as f64;
    if value >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", value / 1024.0 / 1024.0 / 1024.0)
    } else if value >= 1024.0 * 1024.0 {
        format!("{:.1} MB", value / 1024.0 / 1024.0)
    } else if value >= 1024.0 {
        format!("{:.1} KB", value / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// `POST /api/ai/governance/capacity` —— 容量排行。
pub async fn capacity(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<GovReq>,
) -> XResult<Json<Value>> {
    let (conn, database, names, _) = scan_tables(&state, &req).await?;
    let dialect = Dialect::new(require_record(&state, &conn).await?.kind());
    let mut stats: Vec<Value> = Vec::new();
    let mut size_supported = false;
    let mut note: Option<String> = None;

    if let Some(sql) = capacity_sql(dialect.kind.key()) {
        match run_sql_in(&state, &conn, &database, sql.to_string(), MAX_TABLES_HARD).await {
            Ok(result) => {
                size_supported = true;
                for row in rows_of(&result) {
                    let table = row
                        .get("t")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if table.is_empty() {
                        continue;
                    }
                    let rows = row.get("num_rows").and_then(Value::as_i64).unwrap_or(0);
                    let bytes = row.get("bytes").and_then(Value::as_i64).unwrap_or(0);
                    let comment = row
                        .get("comment")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    stats.push(json!({
                        "table": table,
                        "rows": rows,
                        "sizeBytes": bytes,
                        "sizeText": human_bytes(bytes),
                        "comment": comment,
                    }));
                }

                // 系统行数是**估算值**，不是事实：InnoDB 的 `table_rows` 在「表刚建好 +
                // 批量写入完、统计信息还没生成」时返回 0 —— 界面上每张表都是「0 行」，
                // 和「真的是空表」长得一模一样（这正是下面 `stats.is_empty()` 那段警惕的坑，
                // 只不过它防的是另一个分支）。
                //
                // 所以对**估值为 0** 的表补一次实时 `count(*)`。
                // 先收集再统计：避免在这段 await 循环里跨步持有 `stats` 的可变借用。
                let pending: Vec<(usize, String)> = stats
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item["rows"].as_i64().unwrap_or(0) == 0)
                    .map(|(index, item)| {
                        (index, item["table"].as_str().unwrap_or("").to_string())
                    })
                    .collect();
                let mut recounted = 0usize;
                let mut skipped = 0usize;
                for (index, table) in pending {
                    if recounted >= ROW_COUNT_FALLBACK_LIMIT {
                        skipped += 1;
                        continue;
                    }
                    recounted += 1;
                    let count_sql = dialect.count_sql(&table);
                    if let Ok(result) =
                        run_sql_in(&state, &conn, &database, count_sql, 1).await
                    {
                        if let Some(value) = rows_of(&result)
                            .first()
                            .and_then(|row| row.values().next())
                            .and_then(Value::as_i64)
                        {
                            if value >= 0 {
                                stats[index]["rows"] = json!(value);
                            }
                        }
                    }
                    // 统计不到就保持原样：宁可显示估算的 0，也不谎报一个数
                }
                if recounted > 0 {
                    note = Some(format!(
                        "{recounted} 张表的系统行数估算为 0（多为统计信息未更新），已改为实时统计"
                    ));
                }
                if skipped > 0 {
                    note = Some(match note.take() {
                        Some(existing) => format!("{existing}；另有 {skipped} 张表未补统计，仍为估算值"),
                        None => format!("另有 {skipped} 张表未补统计，仍为估算值"),
                    });
                }
            }
            Err(err) => {
                // 取不到就退化为「只有行数」，并把原因写进 note —— 不能装作统计成功了
                note = Some(format!("无法读取占用空间（{}），本次只统计行数", err.message));
            }
        }
    } else {
        note = Some(format!(
            "{} 没有「表占用空间」的系统口径，本次只统计行数",
            dialect.kind.key().to_ascii_uppercase()
        ));
    }

    if stats.is_empty() {
        // 方言没有「表占用空间」的系统口径（文件型库 / 嵌入式库），或系统表查询失败。
        //
        // 这里**不能**退化用元数据里的行数估算：那个字段在很多类型上压根是 `None`，
        // 一旦 `unwrap_or(0)`，界面上每张表都是「0 行」—— 和「真的是空表」长得一模一样。
        // 所以改为**实时 `count(*)`**：用户点的是「统计容量」，准确优先；
        // 表数上限（≤500）保证了最坏情况可控。
        for table in names.iter().take(MAX_TABLES_HARD) {
            let count_sql = dialect.count_sql(table);
            let rows = match run_sql_in(&state, &conn, &database, count_sql, 1).await {
                Ok(result) => rows_of(&result)
                    .first()
                    .and_then(|row| row.values().next())
                    .and_then(Value::as_i64)
                    .unwrap_or(-1),
                Err(_) => -1,
            };
            stats.push(json!({
                "table": table,
                "rows": rows,
                "sizeBytes": 0,
                "sizeText": "—",
                "comment": "",
                "rowsUnavailable": rows < 0,
            }));
        }
        if note.is_none() {
            note = Some("该类型没有系统表口径，行数为实时统计".to_string());
        }
    }
    if size_supported {
        stats.sort_by(|a, b| {
            b["sizeBytes"]
                .as_i64()
                .unwrap_or(0)
                .cmp(&a["sizeBytes"].as_i64().unwrap_or(0))
        });
    } else {
        stats.sort_by(|a, b| {
            b["rows"]
                .as_i64()
                .unwrap_or(0)
                .cmp(&a["rows"].as_i64().unwrap_or(0))
        });
    }
    let total_rows: i64 = stats.iter().map(|item| item["rows"].as_i64().unwrap_or(0)).sum();
    let total_size: i64 = stats.iter().map(|item| item["sizeBytes"].as_i64().unwrap_or(0)).sum();
    Ok(Json(json!({
        "success": true,
        "scanned": stats.len(),
        "totalRows": total_rows,
        "totalSize": total_size,
        "totalSizeText": if size_supported { human_bytes(total_size) } else { "—".to_string() },
        "sizeSupported": size_supported,
        "tables": stats,
        "note": note,
    })))
}

// ------------------------------------------------------------------ 关联推导

struct Relation {
    from_table: String,
    from_column: String,
    to_table: String,
    to_column: String,
    confidence: f64,
}

/// 按命名约定推导关联：`customer_id` → 表 `customer` / `customers`。
fn infer_relations(tables: &[String], columns: &[(String, Vec<ColumnDetail>)]) -> Vec<Relation> {
    let key_pattern = Regex::new(r"(?i)^(.+?)_?(id|no|code|key)$").expect("固定的正则不会失败");
    let lower_tables: Vec<(String, String)> = tables
        .iter()
        .map(|name| (name.to_ascii_lowercase(), name.clone()))
        .collect();
    let mut out: Vec<Relation> = Vec::new();
    for (table, cols) in columns {
        let table_lower = table.to_ascii_lowercase();
        for column in cols {
            let name = column.name.to_ascii_lowercase();
            let Some(captures) = key_pattern.captures(&name) else {
                continue;
            };
            let prefix = captures.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
            if prefix.len() < 2 {
                continue;
            }
            // 三段匹配：完全同名 > 复数形式 > 名称包含（置信度依次降低）
            let mut best: Option<(String, f64)> = None;
            for (lower, original) in &lower_tables {
                if *lower == table_lower {
                    continue;
                }
                let score = if *lower == prefix {
                    0.9
                } else if *lower == format!("{prefix}s") || *lower == format!("{prefix}es") {
                    0.8
                } else if lower.starts_with(&prefix) && prefix.len() >= 4 {
                    0.5
                } else {
                    continue;
                };
                if best.as_ref().map(|(_, current)| score > *current).unwrap_or(true) {
                    best = Some((original.clone(), score));
                }
            }
            if let Some((target, confidence)) = best {
                // 目标列：优先用对方的主键，没有就退回同名列
                let target_cols = columns
                    .iter()
                    .find(|(name, _)| *name == target)
                    .map(|(_, cols)| cols);
                let to_column = target_cols
                    .and_then(|cols| {
                        cols.iter()
                            .find(|col| col.primary_key)
                            .or_else(|| cols.iter().find(|col| col.name.eq_ignore_ascii_case(&column.name)))
                    })
                    .map(|col| col.name.clone())
                    .unwrap_or_else(|| column.name.clone());
                out.push(Relation {
                    from_table: table.clone(),
                    from_column: column.name.clone(),
                    to_table: target,
                    to_column,
                    confidence,
                });
            }
        }
    }
    out
}

/// 取一批表的列（治理的几个端点共用）。
async fn columns_batch(
    state: &AppState,
    conn: &str,
    database: &str,
    names: &[String],
) -> XResult<Vec<(String, Vec<ColumnDetail>)>> {
    let mut out = Vec::new();
    for name in names {
        let columns = columns_of(state, conn, database, name).await?;
        if !columns.is_empty() {
            out.push((name.clone(), columns));
        }
    }
    Ok(out)
}

/// `POST /api/ai/governance/relations` —— 关联清单。
pub async fn relations(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<GovReq>,
) -> XResult<Json<Value>> {
    let (conn, database, names, _) = scan_tables(&state, &req).await?;
    let columns = columns_batch(&state, &conn, &database, &names).await?;
    let list = infer_relations(&names, &columns);
    Ok(Json(json!({
        "success": true,
        "scanned": columns.len(),
        "relations": list.iter().map(|rel| json!({
            "fromTable": rel.from_table,
            "fromColumn": rel.from_column,
            "toTable": rel.to_table,
            "toColumn": rel.to_column,
            "confidence": rel.confidence,
            "kind": "inferred",
        })).collect::<Vec<_>>(),
        "note": "关联按命名约定推导（如 customer_id → customer），不是数据库里声明的外键；每条都带 confidence",
    })))
}

/// `POST /api/ai/governance/er-graph` —— ER 图（表 + 关系）。
///
/// 形状**必须**是前端 `ErDiagramView` 直接消费的那一份：`tables`（每张表带完整列清单）
/// + `relations`（`fromTable` / `toTable` 这一套键名，与 [`relations`] 端点保持一致）。
///
/// 曾经这里返回的是 `nodes` / `edges`，且 `nodes[].columns` 只是个**数字**（列数）——
/// 前端 `tables` 取不到就渲染空画布，`columns` 不是数组也让列行没法画。
/// 结果是「ER 图点了没反应」：顶部提示条正常、画布一片空白，看起来像后端没返回数据，
/// 而实际上接口 200、数据齐全。改形状前先确认前端怎么读（`ErDiagramView` 的 `load`/`layout`）。
pub async fn er_graph(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<GovReq>,
) -> XResult<Json<Value>> {
    let (conn, database, names, _) = scan_tables(&state, &req).await?;
    let columns = columns_batch(&state, &conn, &database, &names).await?;
    let list = infer_relations(&names, &columns);
    let tables: Vec<Value> = columns
        .iter()
        .map(|(table, cols)| {
            json!({
                "name": table,
                "columns": cols
                    .iter()
                    .map(|col| json!({
                        "name": col.name,
                        "type": col.type_name.clone().unwrap_or_default(),
                        "pk": col.primary_key,
                        "nullable": col.nullable,
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let relations: Vec<Value> = list
        .iter()
        .map(|rel| json!({
            "fromTable": rel.from_table,
            "fromColumn": rel.from_column,
            "toTable": rel.to_table,
            "toColumn": rel.to_column,
            "confidence": rel.confidence,
            "kind": "inferred",
        }))
        .collect();
    Ok(Json(json!({
        "success": true,
        "tables": tables,
        "relations": relations,
        "note": "边由命名约定推导，非数据库外键",
    })))
}

/// `POST /api/ai/governance/impact` —— 改动影响面。
pub async fn impact(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<GovReq>,
) -> XResult<Json<Value>> {
    let table = req
        .table
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("请指定要分析的表（table）"))?;
    let (conn, database, names, _) = scan_tables(&state, &req).await?;
    let columns = columns_batch(&state, &conn, &database, &names).await?;
    let list = infer_relations(&names, &columns);

    // 直接依赖：谁的列指向我这
    let direct: Vec<Value> = list
        .iter()
        .filter(|rel| rel.to_table.eq_ignore_ascii_case(&table))
        .map(|rel| json!({
            "table": rel.from_table,
            "column": rel.from_column,
            "via": format!("{}.{} → {}.{}", rel.from_table, rel.from_column, rel.to_table, rel.to_column),
            "confidence": rel.confidence,
        }))
        .collect();
    // 间接依赖：直接依赖者的上游（只走一层 —— 再深就变成「整库都相关」，没有指导意义）
    let direct_names: Vec<String> = direct
        .iter()
        .filter_map(|item| item["table"].as_str().map(str::to_string))
        .collect();
    let indirect: Vec<Value> = list
        .iter()
        .filter(|rel| direct_names.iter().any(|name| name.eq_ignore_ascii_case(&rel.to_table)))
        .filter(|rel| !rel.to_table.eq_ignore_ascii_case(&table))
        .map(|rel| json!({
            "table": rel.from_table,
            "column": rel.from_column,
            "layer": 2,
        }))
        .collect();

    // 视图依赖：取定义文本，看有没有提到这张表
    let all = tables_of(&state, &conn, &database).await?;
    let dialect = Dialect::new(require_record(&state, &conn).await?.kind());
    let mut views: Vec<Value> = Vec::new();
    let mut views_checked = 0usize;
    for view in all
        .iter()
        .filter(|item| item.kind == dbmind_core::TableKind::View)
        .take(MAX_VIEWS)
    {
        let Some(sql) = dialect.ddl(&view.name).sql() else {
            continue;
        };
        views_checked += 1;
        if let Ok(result) = run_sql_in(&state, &conn, &database, sql, 20).await {
            let mentions = rows_of(&result)
                .iter()
                .flat_map(|row| row.values())
                .any(|value| {
                    value
                        .as_str()
                        .map(|text| {
                            text.to_ascii_lowercase()
                                .contains(&table.to_ascii_lowercase())
                        })
                        .unwrap_or(false)
                });
            if mentions {
                views.push(json!({ "name": view.name }));
            }
        }
    }

    Ok(Json(json!({
        "success": true,
        "table": table,
        "direct": direct,
        "indirect": indirect,
        "views": views,
        "viewsChecked": views_checked,
        "summary": {
            "direct": direct.len(),
            "indirect": indirect.len(),
            "views": views.len(),
        },
        "note": "依赖关系由命名约定推导；视图依赖通过扫描视图定义文本得出",
    })))
}

// ------------------------------------------------------------------ 质量规则建议

/// `POST /api/ai/governance/quality/rules` —— 按表结构生成质量规则建议。
///
/// 规则引擎先给一版（按类型/可空/命名/主键），模型可用时再让它补充业务语义规则；
/// **模型失败不影响规则产出**。
pub async fn quality_rules(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<GovReq>,
) -> XResult<Json<Value>> {
    let conn = req.conn()?;
    let database = req.db();
    let table = req
        .table
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("请指定表（table）"))?;
    let columns = columns_of(&state, &conn, &database, &table).await?;
    if columns.is_empty() {
        return Err(XError::bad_request(format!("表 {table} 不存在或没有列信息")));
    }
    let mut rules: Vec<Value> = Vec::new();
    for column in &columns {
        let name = column.name.to_ascii_lowercase();
        let type_name = column.type_name.clone().unwrap_or_default().to_ascii_lowercase();
        let text_type = ["char", "text", "clob", "string", "varchar"]
            .iter()
            .any(|needle| type_name.contains(needle));
        let number_type = ["int", "decimal", "numeric", "number", "float", "double", "real"]
            .iter()
            .any(|needle| type_name.contains(needle));

        if !column.nullable {
            rules.push(rule_json(
                &column.name,
                "not_null",
                "高",
                format!("{} 不允许为空", column.name),
                json!({}),
            ));
        } else {
            rules.push(rule_json(
                &column.name,
                "null_ratio",
                "低",
                format!("{} 可空，建议监控空值率", column.name),
                json!({ "max": 50 }),
            ));
        }
        if column.primary_key || name == "id" || name.ends_with("_id") || name.ends_with("_code") {
            rules.push(rule_json(
                &column.name,
                "unique",
                "高",
                format!("{} 应唯一", column.name),
                json!({}),
            ));
        }
        if text_type {
            if name.contains("email") || name.contains("mail") {
                rules.push(rule_json(&column.name, "email", "中", "邮箱格式", json!({})));
            } else if name.contains("phone") || name.contains("mobile") || name.contains("tel") {
                rules.push(rule_json(&column.name, "phone", "中", "手机号格式", json!({})));
            } else if name.contains("id_card") || name.contains("idcard") {
                rules.push(rule_json(&column.name, "id_card", "中", "身份证格式", json!({})));
            } else if name.contains("status") || name.contains("state") || name.contains("type") {
                rules.push(rule_json(
                    &column.name,
                    "enum",
                    "低",
                    format!("{} 取值应在约定集合内", column.name),
                    json!({ "values": "" }),
                ));
            }
            if let Some(length) = type_length(&type_name) {
                if length > 0 && length <= 2000 {
                    rules.push(rule_json(
                        &column.name,
                        "max_length",
                        "低",
                        format!("{} 长度不应超过 {}（与列定义一致）", column.name, length),
                        json!({ "max": length }),
                    ));
                }
            }
        }
        if number_type
            && ["amount", "price", "qty", "quantity", "age", "score", "count"]
                .iter()
                .any(|needle| name.contains(needle))
        {
            rules.push(rule_json(
                &column.name,
                "range",
                "低",
                format!("{} 应在业务合理区间内", column.name),
                json!({ "min": 0 }),
            ));
        }
    }

    let mut ai_used = false;
    let mut ai_error: Option<String> = None;
    if let Ok(model) = crate::api::ai::config::resolve(req.model_id.as_deref()) {
        let schema = columns
            .iter()
            .map(|column| {
                format!(
                    "{} {} {}",
                    column.name,
                    column.type_name.clone().unwrap_or_default(),
                    if column.nullable { "" } else { "not null" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let system = "你是数据质量规则顾问，只输出 JSON。";
        let user = format!(
            "表 {table} 的结构：\n{schema}\n\n\
             请按业务语义补充质量规则建议，只输出 JSON：\
             {{\"rules\":[{{\"column\":\"列名\",\"type\":\"not_null|not_empty|unique|enum|max_length|min_length|range|pattern|null_ratio|constant|empty_column|email|phone|id_card\",\
             \"level\":\"高|中|低\",\"message\":\"业务说明\",\"params\":{{}}}}]}}\n\
             只补充上面结构里真实存在的列；如果没什么可补的，返回空数组。"
        );
        let messages = vec![crate::api::ai::config::Msg {
            role: "user".to_string(),
            content: user,
        }];
        match crate::api::ai::run_blocking(move || {
            crate::api::ai::config::chat(&model, system, &messages)
        })
        .await
        {
            Ok(raw) => {
                ai_used = true;
                let text = raw.trim();
                let json_text = match (text.find('{'), text.rfind('}')) {
                    (Some(start), Some(end)) if end > start => &text[start..=end],
                    _ => text,
                };
                if let Ok(value) = serde_json::from_str::<Value>(json_text) {
                    if let Some(list) = value.get("rules").and_then(Value::as_array) {
                        for item in list {
                            let Some(column) = item.get("column").and_then(Value::as_str) else {
                                continue;
                            };
                            if !columns.iter().any(|known| known.name == column) {
                                continue;
                            }
                            let rule_type = item
                                .get("type")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            if rule_type.is_empty() {
                                continue;
                            }
                            // 与规则引擎已有的建议去重：同一个列同一个类型只留一条
                            if rules.iter().any(|existing| {
                                existing["column"] == json!(column) && existing["type"] == json!(rule_type)
                            }) {
                                continue;
                            }
                            rules.push(rule_json(
                                column,
                                &rule_type,
                                item.get("level").and_then(Value::as_str).unwrap_or("中"),
                                item
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("模型建议"),
                                item.get("params").cloned().unwrap_or(json!({})),
                            ));
                        }
                    }
                }
            }
            Err(err) => ai_error = Some(err.message),
        }
    }

    Ok(Json(json!({
        "success": true,
        "table": table,
        "rules": rules,
        "aiUsed": ai_used,
        "aiError": ai_error,
    })))
}

fn rule_json(column: &str, rule_type: &str, level: &str, message: impl Into<String>, params: Value) -> Value {
    json!({
        "column": column,
        "type": rule_type,
        "level": level,
        "message": message.into(),
        "enabled": true,
        "params": params,
    })
}

/// 从 `varchar(50)` 这类类型名里取长度。
fn type_length(type_name: &str) -> Option<i64> {
    let open = type_name.find('(')?;
    let close = type_name[open..].find(')')? + open;
    type_name[open + 1..close]
        .split(',')
        .next()
        .and_then(|value| value.trim().parse::<i64>().ok())
}

/// `POST /api/ai/governance/quality/check` —— 单表试跑（复用质量扫描引擎）。
pub async fn quality_check(
    state: axum::extract::State<AppState>,
    Json(req): Json<Value>,
) -> XResult<Json<Value>> {
    // 入参是自由 JSON（规则可随请求带上），所以再包一层 `Json` 交给质量引擎
    crate::api::ai::quality::check_one(state, Json(req)).await
}

/// 治理域路由。
pub fn routes() -> axum::Router<AppState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/api/ai/governance/sensitive", post(scan_sensitive))
        .route("/api/ai/governance/capacity", post(capacity))
        .route("/api/ai/governance/relations", post(relations))
        .route("/api/ai/governance/er-graph", post(er_graph))
        .route("/api/ai/governance/impact", post(impact))
        // 规则建议与单表试跑：前端放在「质量规则」页签里，但归属治理域
        .route("/api/ai/governance/quality/rules", post(quality_rules))
        .route("/api/ai/governance/quality/check", post(quality_check))
}
