//! 表数据浏览与行编辑：分页取数、高级搜索、UPDATE/DELETE 条件拼装、时间字面量。
//!
//! 豫园 Oracle 11g 的日期字面量故障（`temporal_literal` 的 to_date 显式格式串）
//! 守门单测在这个文件，动这里的拼装先跑它。

use super::*;

async fn run_sql_external(
    state: &AppState,
    conn: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    crate::api::driver::ensure_for_connection(state, conn).await?;
    let engine = state.engine();
    let request = QueryRequest {
        schema: None,
        connection: conn.to_string(),
        sql,
        options: QueryOptions {
            max_rows,
            timeout_ms: 120_000,
        },
        execution_id: None,
        session: None,
        read_only: Some(true),
        internal: true,
    };
    blocking(move || engine.execute(request, AccessContext::Web)).await
}

/// 外部 catalog 的表引用形态无法从库名区分：DataLake catalog 的子库（`catalog.db`）
/// 走**三段名** `catalog.db.table`；字面带点库名（如 MySQL 引擎库 `fakecat.demo`）
/// 只认**两段名**整体引用（三段名在 CH 24.8 实测直接 SYNTAX_ERROR）。
/// 依候选顺序逐个尝试，全部失败报最后一个错误。
async fn run_external(
    state: &AppState,
    conn: &str,
    variants: &[String],
    build_sql: impl Fn(&str) -> String,
    max_rows: usize,
) -> XResult<QueryResult> {
    let mut last_err: Option<XError> = None;
    for quoted in variants {
        let sql = build_sql(quoted);
        match run_sql_external(state, conn, sql, max_rows).await {
            Ok(result) => return Ok(result),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.expect("variants is non-empty"))
}

/// `database` 是否是外部 catalog 的全限定名（`catalog.库`，catalog_level 方言，
/// 如 Doris 的 `mysql_216.test` / 指向 ClickHouse 的 `clickhouse_catalog.oish_dws`）。
/// `internal` 是默认 catalog，树上虽然也带前缀，但走的仍是普通切库路径。
pub(crate) fn external_catalog_of(dialect: &Dialect, database: &str) -> Option<(String, String)> {
    if !dialect.catalog_level() {
        return None;
    }
    database
        .split_once('.')
        .filter(|(c, d)| !c.is_empty() && !d.is_empty() && !c.eq_ignore_ascii_case("internal"))
        .map(|(c, d)| (c.to_string(), d.to_string()))
}

/// 模糊搜索可用的文本列类型（CHAR 会命中 VARCHAR/NCHAR/CHAR…，TEXT 命中 LONGTEXT…）。
fn is_textual_col_type(type_name: &str) -> bool {
    let t = type_name.to_ascii_uppercase();
    [
        "CHAR", "TEXT", "CLOB", "STRING", "JSON", "UUID", "ENUM", "XML", "NCHAR",
    ]
    .iter()
    .any(|probe| t.contains(probe))
}

/// 外部 catalog 表的列元数据：`desc \`catalog\`.\`db\`.\`table\``。
///
/// 为什么不用 `<catalog>.information_schema.columns`：JDBC catalog 上它会报
/// 「Internal error processing describeTables」（实验室 Doris 2.1.8 实测）；
/// 而 `desc` 三段名可用，返回 Field/Type/Null/Key/Default/Extra。
pub(crate) async fn external_table_columns(
    state: &AppState,
    id: &str,
    dialect: &Dialect,
    catalog: &str,
    db: &str,
    table: &str,
) -> XResult<Vec<(String, String, bool)>> {
    let sql = format!(
        "desc {}.{}.{}",
        dialect.quote(catalog),
        dialect.quote(db),
        dialect.quote(table)
    );
    let result = run_sql_external(state, id, sql, 10_000).await?;
    Ok(rows_of(&result)
        .iter()
        .filter_map(|row| {
            let name = text_ci(row, "Field");
            if name.is_empty() {
                return None;
            }
            let type_name = text_ci(row, "Type");
            let nullable = text_ci(row, "Null").eq_ignore_ascii_case("yes");
            Some((name, type_name, nullable))
        })
        .collect())
}

/// 分页浏览表数据（`/data`）。
///
/// 界面语义（分页 / 关键字模糊 / 排序）在这里翻成 SQL —— 内核只认「一条语句」。
/// `keyword` 会横跨所有文本列做 `LIKE`，列清单从内核的列元数据取（不猜类型）。
pub async fn data(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    let page: u64 = params
        .get("page")
        .and_then(|p| p.parse().ok())
        .filter(|p| *p > 0)
        .unwrap_or(1);
    // 页大小上限与界面下拉同步（末尾三档 20 万 / 50 万 / 100 万）：
    // 这里原来卡 100_000，超了直接**静默**退回 50 行 —— 用户选「50万条/页」却只看到 50 行，
    // 还以为是表里只有数据。上限放到 100 万后，实际返回量仍由「返回行数上限」设置兜底。
    let size: u64 = params
        .get("size")
        .and_then(|s| s.parse().ok())
        .filter(|s| *s > 0 && *s <= 1_000_000)
        .unwrap_or(50);
    let keyword = params.get("keyword").unwrap_or_default();
    // 排序键（多列，按优先级）；空表示不排序
    let order_specs = parse_order_specs(&params);
    // 高级搜索的条件（界面「添加条件」那一排）：
    // `[{"join":"AND","col":"id","op":"eq","value":"548","colType":"number"}]`
    // 这个参数以前被整个丢掉 —— 界面点了「应用筛选」却还是整张表，就是这么来的。
    let filters: Vec<Value> = params
        .get("filters")
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();

    let (record, dialect) = ctx_of(&state, &id).await?;
    // Oracle / DM 的「库」层装的其实就是**模式**（`db_is_schema`），而这两个方言没有
    // 可切的会话模式 —— 表引用必须自己带模式前缀，否则按登录用户的默认模式解析。
    // 真机：达梦 `database=DEMO` 时 `select * from "USERS"` 报「无效的表或视图名[USERS]」，
    // 改成 `"DEMO"."USERS"` 立刻正常。其余方言这里恒为 `None`（由影子连接/use 决定目标库）。
    let scope_schema = super::scope_schema_of(&database, &dialect, record.config.database.as_deref());
    let table_ref = dialect.table_ref(scope_schema.as_deref(), &table);
    // 外部 catalog（catalog_level 方言，如 CH 的 unified catalog）：`database` 是
    // `catalog.db` 全限定名。JDBC 连接无法指定外部 catalog（切库那条路在 scope 里被
    // 明确拒绝），但 ClickHouse 支持**三段名** —— 不切库，直接在主连接上查全限定表。
    let external = external_catalog_of(&dialect, &database);
    // 引用候选：先试三段名（DataLake catalog 子库），失败换两段名（字面带点库名），见 run_external。
    let external_quoted: Vec<String> = match &external {
        Some((catalog, db)) => vec![
            format!(
                "{}.{}.{}",
                dialect.quote(catalog),
                dialect.quote(db),
                dialect.quote(&table)
            ),
            format!("{}.{}", dialect.quote(&database), dialect.quote(&table)),
        ],
        None => vec![table_ref.clone()],
    };
    // 所有筛选条件（关键字模糊 + 高级搜索）都汇到这里，统一用 AND 串起来；
    // 同一份 where 既给数据查询、也给总条数统计 —— 否则会出现"筛出 1 行、总数说 10000"。
    let mut where_groups: Vec<String> = Vec::new();

    if !keyword.trim().is_empty() {
        // 外部 catalog 的列元数据走 desc 三段名（内核的列缓存里没有它）；普通表照旧走内核。
        let text_columns: Vec<String> = if let Some((catalog, db)) = &external {
            external_table_columns(&state, &id, &dialect, catalog, db, &table)
                .await?
                .into_iter()
                .filter(|(_, type_name, _)| is_textual_col_type(type_name))
                .map(|(name, _, _)| name)
                .collect()
        } else {
            let engine = state.engine();
            let name = table.clone();
            let connection = id.clone();
            let columns = blocking(move || engine.list_columns_fresh(&connection, &name)).await?;
            columns
                .iter()
                .filter(|c| {
                    is_textual_col_type(
                        c.type_name
                            .clone()
                            .unwrap_or_default()
                            .to_ascii_uppercase()
                            .as_str(),
                    )
                })
                .map(|c| c.name.clone())
                .collect()
        };
        if text_columns.is_empty() {
            return Err(XError::bad_request(format!(
                "表「{table}」没有可作为模糊匹配的文本列，无法按关键字筛选"
            )));
        }
        let pattern = dialect.literal(&format!("%{}%", escape_like(&keyword)));
        let esc = like_escape(&dialect);
        let conditions: Vec<String> = text_columns
            .iter()
            // 列名同样是目录元数据（含外部 catalog 的 desc 结果）→ 逐字加引号
            .map(|c| format!("{} like {pattern}{esc}", dialect.quote_exact(c)))
            .collect();
        where_groups.push(format!("({})", conditions.join(" or ")));
    }
    // 高级搜索：条件本身由 filter_clause 负责翻译与括号；这里只把它并进 where
    if let Some(clause) = filter_clause(&dialect, &filters) {
        where_groups.push(clause);
    }

    let where_suffix = if where_groups.is_empty() {
        String::new()
    } else {
        format!(" where {}", where_groups.join(" and "))
    };
    // where + 排序 = 数据查询的公共尾巴；分页子句在 build_paged 里按方言补；
    // count 不带排序/分页，只用 where。
    let mut tail = where_suffix.clone();

    if !order_specs.is_empty() {
        // 多列：`order by "age" asc, "name" desc` —— 顺序即优先级，方向已白名单。
        // 列名走 **quote_exact**：表头点排序传过来的是库里真实列名（/columns 的返回值），
        // 用 quote() 会在上折方言里把 `day` 折成 `DAY`，小写引号列必然 ORA-00904。
        let parts: Vec<String> = order_specs
            .iter()
            .map(|(col, dir)| format!("{} {dir}", dialect.quote_exact(col)))
            .collect();
        tail.push_str(&format!(" order by {}", parts.join(", ")));
    } else if dialect.needs_order_by_for_paging() {
        // SQL Server 的 OFFSET/FETCH 必须有 ORDER BY
        tail.push_str(" order by (select null)");
    }
    // Oracle 11g 不认 `offset … fetch …`（12c 语法 → ORA-00933，豫园实测），走 ROWNUM
    // 包装（wrap_paging）；其余方言照旧追加 limit_clause。
    let paging_offset = page.saturating_sub(1).saturating_mul(size);
    let build_paged = |q: &str| {
        let base = format!("select * from {q}{tail}");
        dialect
            .wrap_paging(&base, paging_offset, size)
            .unwrap_or_else(|| format!("{base} {}", dialect.limit_clause(paging_offset, size)))
    };

    let result = match &external {
        Some(_) => run_external(&state, &id, &external_quoted, build_paged, size as usize).await,
        None => {
            run_sql_in(
                &state,
                &id,
                &database,
                // 表名同样来自树（目录元数据）→ 逐字加引号，
                // 否则带引号建的小写表在 Oracle 上被折成大写、报 ORA-00942；
                // Oracle / DM 还要带模式前缀（见上面 table_ref 的说明）
                build_paged(&table_ref),
                size as usize,
            )
            .await
        }
    };
    let result = match result {
        Ok(result) => result,
        Err(err) => return Ok(Json(shape::query_failure_json(&err.message, 0))),
    };
    let mut payload = shape::query_result_json(&result);
    // ROWNUM 包装（Oracle 11g 分页）会在结果里多一列辅助列 rn__，剥掉不展示。
    shape::strip_paging_aux(&mut payload);

    // 真实总条数：分页控件要用它。统计失败就当「未知」（-1），不假装是当前页的行数。
    // 必须带上与数据查询**同一份 where** —— 否则筛出 1 行而总数显示 10000。
    // 外部 catalog：count_sql(&table) 会把三段名整体 quote 成单段标识符，这里直接用
    // 候选引用；执行同样在主连接上（不切库）。
    let total = match if external.is_some() {
        run_external(
            &state,
            &id,
            &external_quoted,
            |q| format!("select count(*) as cnt from {q}{where_suffix}"),
            1,
        )
        .await
    } else {
        run_sql_in(
            &state,
            &id,
            &database,
            format!(
                "{}{where_suffix}",
                dialect.count_sql_in(scope_schema.as_deref(), &table)
            ),
            1,
        )
        .await
    } {
        Ok(count_result) => rows_of(&count_result)
            .first()
            .map(|row| {
                cell_int(row, "cnt")
                    .or_else(|| row.values().find_map(shape::value_to_i64))
                    .unwrap_or(-1)
            })
            .unwrap_or(-1),
        Err(_) => -1,
    };
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("totalCount".to_string(), json!(total));
        obj.insert("page".to_string(), json!(page));
        obj.insert("size".to_string(), json!(size));
    }
    Ok(Json(payload))
}

/// 一行数据 → 值字面量。
///
/// 字符串必须走 [`Dialect::literal`]：MySQL/MariaDB/Doris/ClickHouse 默认把 `\` 当转义符，
/// 这里若只做 `'` 翻倍，值 `C:\temp\` 会拼成 `'C:\temp\'` —— 收尾引号被吃掉，语句截断，
/// 后面的 `where` 条件被并进字符串（轻则报错，重则改错行）。以前就地编辑走的就是那条路。
fn sql_value(value: Option<&Value>, dialect: &crate::api::dialect::Dialect, type_hint: &str) -> String {
    match value {
        None | Some(Value::Null) => "NULL".to_string(),
        // 布尔取值走方言（PG 系 true/false，MySQL 系/SQL Server bit 1/0——见 bool_literal）
        Some(Value::Bool(b)) => dialect.bool_literal(*b).to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => {
            temporal_literal(dialect, type_hint, s).unwrap_or_else(|| dialect.literal(s))
        }
        Some(other) => dialect.literal(&other.to_string()),
    }
}

/// 定位条件的一列：`col = 值`，值是 NULL 时必须写 `col is null`。
///
/// `col = NULL` 在 SQL 里永远不成立 ⇒ 无主键表（用整行当定位条件）里只要有一列是 NULL，
/// 更新/删除就影响 0 行，而接口照样回「已提交 N 条语句」—— 用户以为存上了，刷新后改动没了。
fn match_condition(
    col: &str,
    value: Option<&Value>,
    dialect: &crate::api::dialect::Dialect,
    type_hint: &str,
) -> String {
    match value {
        // 定位条件里的列名来自行数据（= 结果集列名 = 目录元数据）→ 逐字加引号
        None | Some(Value::Null) => format!("{} is null", dialect.quote_exact(col)),
        _ => format!(
            "{} = {}",
            dialect.quote_exact(col),
            sql_value(value, dialect, type_hint)
        ),
    }
}

fn keys_of(row: &Map<String, Value>) -> Vec<String> {
    row.keys().cloned().collect()
}

/// 日期时间字符串 → 会话语义无关的字面量（只对 Oracle / 达梦）。
///
/// 为什么需要：`insert … values ('2026-02-01T00:00')` 里 Oracle 会拿**会话的
/// `NLS_DATE_FORMAT`**（默认 `DD-MON-RR`）去解析这个字符串 ⇒
/// `ORA-01861: 文字与格式字符串不匹配`。表格里改一行（哪怕改的不是日期列，
/// 整行原值也会原样回传）就整条失败，用户看到的是「保存不了」。
/// 豫园 Oracle 11g 实测复现，改成 `to_date/to_timestamp` 并显式给格式串即可，
/// 格式串与会话设置无关。
///
/// `hint` 既接受列的**真实类型名**（`date` / `timestamp(6)` / `datetime`），
/// 也接受前端高级搜索送的分类值（`date` / `datetime`）。
/// 认不出（非日期类型 / 值不是日期形状 / 其它方言）就返回 `None`，
/// 调用方退回原来的 `dialect.literal` —— 行为与改动前完全一致。
pub(crate) fn temporal_literal(
    dialect: &crate::api::dialect::Dialect,
    hint: &str,
    value: &str,
) -> Option<String> {
    dialect.temporal_row_literal(hint, value)
}

fn same_value(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), None) | (None, Some(Value::Null)) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// 表的就地编辑（新增 / 修改 / 删除）。
///
/// 与上游一致的取舍：**一行一条语句、顺序执行**。内核的安全闸门只允许一次执行一条，
/// 所以做不到「全部成功或全部回滚」—— 于是如实报告「执行到第几条失败”，绝不假装原子。
pub async fn data_save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let table = body
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if table.is_empty() {
        return Err(XError::bad_request("缺少 table"));
    }
    let (record, dialect) = ctx_of(&state, &id).await?;
    let pk_columns: Vec<String> = body
        .get("pkColumns")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let inserts = body
        .get("inserts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let updates = body
        .get("updates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let deletes = body
        .get("deletes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // 什么都没改：直接回「没有需要提交的变更」，不必为此再拉一次列元数据
    if inserts.is_empty() && updates.is_empty() && deletes.is_empty() {
        return Ok(Json(json!({
            "success": true,
            "columns": [], "columnTypes": [], "rows": [],
            "rowCount": 0, "totalCount": -1, "affectedRows": 0, "executeTime": 0,
            "message": "没有需要提交的变更", "notices": [], "hasMore": false,
        })));
    }
    // 就地编辑要落到目标库上（跨库时就是那条影子连接）
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let target = crate::api::scope::resolve(&state, &id, &database).await?;
    // Oracle / DM 的模式（`db_is_schema`）：取列元数据与拼语句都要用它，先算一次。
    let scope_schema = super::scope_schema_of(&database, &dialect, record.config.database.as_deref());
    // 列类型（小写列名 → 类型名）：Oracle / 达梦的日期列必须生成
    // `to_date(...)` / `to_timestamp(...)`，否则 `'2026-02-01T00:00'` 这种 ISO 串
    // 会被按**会话 NLS_DATE_FORMAT** 解析 ⇒ `ORA-01861: 文字与格式字符串不匹配`
    //（豫园 Oracle 11g 实测：在表格里改一行日期列就报这个）。
    // 取不到元数据就退回旧行为 —— 绝不因为「多知道一点」让保存失败。
    // 注意要用**带模式**的取列：达梦/Oracle 的 `getColumns` 只认会话当前模式，
    // 不带模式会拿到默认模式里同名表的列（或空），日期列就漏了 to_date 转换。
    let column_types: std::collections::HashMap<String, String> = {
        let engine = state.engine();
        let connection = target.clone();
        let name = table.clone();
        let schema = scope_schema.clone();
        match blocking(move || engine.list_columns_fresh_in(&connection, &name, schema.as_deref())).await
        {
            Ok(columns) => columns
                .into_iter()
                .filter_map(|column| {
                    column
                        .type_name
                        .map(|type_name| (column.name.to_ascii_lowercase(), type_name))
                })
                .collect(),
            Err(_) => std::collections::HashMap::new(),
        }
    };
    let type_of = |column: &str| -> String {
        column_types
            .get(&column.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    };
    // 就地编辑的目标表来自树（目录元数据）→ 逐字加引号；Oracle / DM 还要带模式前缀
    // （`db_is_schema`：不带前缀会落到登录用户的默认模式，UPDATE 0 行或报表不存在）
    let quoted = dialect.table_ref(scope_schema.as_deref(), &table);

    let mut statements: Vec<String> = Vec::new();

    for row in &inserts {
        let Some(obj) = row.as_object() else { continue };
        let columns = keys_of(obj);
        if columns.is_empty() {
            continue;
        }
        let names: Vec<String> = columns.iter().map(|c| dialect.quote_exact(c)).collect();
        let values: Vec<String> = columns
            .iter()
            .map(|c| sql_value(obj.get(c), &dialect, &type_of(c)))
            .collect();
        statements.push(format!(
            "insert into {quoted} ({}) values ({})",
            names.join(", "),
            values.join(", ")
        ));
    }

    for item in &updates {
        let original = item.get("original").and_then(Value::as_object);
        let row = item.get("row").and_then(Value::as_object);
        let (Some(original), Some(row)) = (original, row) else {
            continue;
        };
        let changed: Vec<String> = keys_of(row)
            .into_iter()
            .filter(|c| !same_value(original.get(c), row.get(c)))
            .collect();
        if changed.is_empty() {
            continue; // 改回原值等于没改，不留一条什么都没变的 UPDATE
        }
        let assignments: Vec<String> = changed
            .iter()
            .map(|c| {
                format!(
                    "{} = {}",
                    dialect.quote_exact(c),
                    sql_value(row.get(c), &dialect, &type_of(c))
                )
            })
            .collect();
        let keys: Vec<String> = if pk_columns.is_empty() {
            keys_of(original)
        } else {
            pk_columns.clone()
        };
        let conditions: Vec<String> = keys
            .iter()
            .map(|c| match_condition(c, original.get(c), &dialect, &type_of(c)))
            .collect();
        statements.push(dialect.update_sql(
            &quoted,
            &assignments.join(", "),
            &conditions.join(" and "),
        ));
    }

    for row in &deletes {
        let Some(obj) = row.as_object() else { continue };
        let keys: Vec<String> = if pk_columns.is_empty() {
            keys_of(obj)
        } else {
            pk_columns.clone()
        };
        let conditions: Vec<String> = keys
            .iter()
            .map(|c| match_condition(c, obj.get(c), &dialect, &type_of(c)))
            .collect();
        statements.push(dialect.delete_sql(&quoted, &conditions.join(" and ")));
    }

    // 有 update/delete 才有可能「语句执行成功但一行都没匹配上」
    let has_row_ops = !updates.is_empty() || !deletes.is_empty();
    // ClickHouse 的 update/delete 是异步 mutation：提交成功不代表已生效，驱动也不返回
    // 可靠的影响行数（通常是 0）。不能照搬「affected=0 ⇒ 没匹配到行」的提示误导用户。
    let async_mutations = has_row_ops && dialect.row_ops_are_async_mutations();

    if statements.is_empty() {
        return Ok(Json(json!({
            "success": true,
            "columns": [], "columnTypes": [], "rows": [],
            "rowCount": 0, "totalCount": -1, "affectedRows": 0, "executeTime": 0,
            "message": "没有需要提交的变更", "notices": [], "hasMore": false,
        })));
    }

    let engine = state.engine();
    let (affected, executed, error) = blocking(move || {
        let mut affected: i64 = 0;
        let mut executed = 0usize;
        for sql in statements {
            let request = QueryRequest {
                schema: None,
                read_only: None,
                connection: target.clone(),
                sql,
                options: QueryOptions {
                    max_rows: 1,
                    timeout_ms: 120_000,
                },
                execution_id: None,
                session: Some("internal:browse".to_string()),
                // 表格「就地编辑」提交的增删改 —— 界面功能驱动的写入，不进查询历史
                // （与编辑器里手写 INSERT/UPDATE 是两回事）
                internal: true,
            };
            match engine.execute(request, AccessContext::Web) {
                Ok(result) => {
                    affected += result.affected_rows.unwrap_or(0) as i64;
                    executed += 1;
                }
                Err(err) => return Ok((affected, executed, Some(err.message))),
            }
        }
        Ok((affected, executed, None))
    })
    .await?;

    let (success, message) = match &error {
        None => (
            true,
            // 影响 0 行不能只说「已提交」：无主键表/行被他人改过时，定位条件一条也没命中，
            // 网格里那行却已经本地改掉了 —— 用户以为存上了，刷新就回滚。
            if async_mutations {
                format!("已提交 {executed} 条 mutation，正在异步执行，请稍后刷新查看结果")
            } else if has_row_ops && affected == 0 {
                format!("已提交 {executed} 条语句，但未匹配到任何行（数据可能已被修改或删除，请刷新后重试）")
            } else {
                format!("已提交 {executed} 条语句，影响 {affected} 行")
            },
        ),
        // 同一口径：前端已经加了「保存失败：」这层动作前缀，这里再报「第几条失败（共几条…）」
        // 就是套娃（用户实测看到的原话是「保存失败：第 1 条失败（共 1 条，已成功 0 条）：…」）。
        // 只留原因；「已成功 N 条」只在真有语句生效时才补。
        Some(err) => {
            let reason = err.strip_prefix("SQL 执行失败：").unwrap_or(err);
            (
                false,
                if executed == 0 {
                    reason.to_string()
                } else {
                    format!("{reason}（前 {executed} 条已成功）")
                },
            )
        }
    };
    Ok(Json(json!({
        "success": success,
        "columns": [], "columnTypes": [], "rows": [],
        "rowCount": 0, "totalCount": -1,
        "affectedRows": affected,
        "executeTime": 0,
        "message": message,
        "notices": [], "hasMore": false,
    })))
}

// ------------------------------------------------------------------ 建表语句与对象

#[cfg(test)]
mod tests {
    use super::*;

    fn dialect(key: &str) -> crate::api::dialect::Dialect {
        let kind = dbmind_core::ConnectionKind::from_key(key).expect("未知的连接类型");
        crate::api::dialect::Dialect::new(kind)
    }
    /// 日期列的字面量必须显式给格式串（Oracle 会话 NLS 默认 `DD-MON-RR`）。
    ///
    /// 守着豫园 Oracle 11g 的真机故障：表格里改一行，`day` 列原值
    /// `2026-02-01T00:00` 原样回传 ⇒ `ORA-01861: 文字与格式字符串不匹配`。
    #[test]
    fn oracle_日期字面量走_to_date() {
        let oracle = dialect("oracle");
        assert_eq!(
            temporal_literal(&oracle, "date", "2026-02-01T00:00").as_deref(),
            Some("to_date('2026-02-01 00:00', 'YYYY-MM-DD HH24:MI')")
        );
        assert_eq!(
            temporal_literal(&oracle, "DATE", "2026-02-01").as_deref(),
            Some("to_date('2026-02-01', 'YYYY-MM-DD')")
        );
        // 带秒
        assert_eq!(
            temporal_literal(&oracle, "date", "2026-02-01 13:45:59").as_deref(),
            Some("to_date('2026-02-01 13:45:59', 'YYYY-MM-DD HH24:MI:SS')")
        );
        // TIMESTAMP 保留小数秒：固定列宽（FF）不会被截断
        assert_eq!(
            temporal_literal(&oracle, "timestamp(6)", "2026-02-01T13:45:59.123456").as_deref(),
            Some("to_timestamp('2026-02-01 13:45:59.123456', 'YYYY-MM-DD HH24:MI:SS.FF')")
        );
        // DATE 列丢掉小数秒（Oracle DATE 只到秒），但语句仍然合法
        assert_eq!(
            temporal_literal(&oracle, "date", "2026-02-01T13:45:59.123").as_deref(),
            Some("to_date('2026-02-01 13:45:59', 'YYYY-MM-DD HH24:MI:SS')")
        );
        // 前端高级搜索送的分类值同样认
        assert!(temporal_literal(&oracle, "datetime", "2026-02-01 10:00").is_some());
    }

    /// 认不出/不适用时必须返回 None（调用方退回普通字符串字面量，行为不变）。
    #[test]
    fn 日期字面量不误伤其它情况() {
        let oracle = dialect("oracle");
        // 非日期类型
        assert!(temporal_literal(&oracle, "varchar2(20)", "2026-02-01").is_none());
        assert!(temporal_literal(&oracle, "number(10,2)", "2026").is_none());
        // 非 Oracle 族：MySQL/PG 都认 ISO 串，别多套一层函数
        assert!(temporal_literal(&dialect("mysql"), "date", "2026-02-01").is_none());
        assert!(temporal_literal(&dialect("postgresql"), "timestamp", "2026-02-01").is_none());
        // 不是日期形状的值不猜
        assert!(temporal_literal(&oracle, "date", "02-01-2026").is_none());
        assert!(temporal_literal(&oracle, "date", "2026-02-01T10:00:00+08:00").is_none());
        assert!(temporal_literal(&oracle, "date", "CURRENT_DATE").is_none());
    }
}
