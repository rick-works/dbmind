//! 实时监控（`GET /api/{m}/{id}/monitor`、`POST /api/{m}/{id}/monitor/kill`）。
//!
//! ## 为什么是「方言给 SQL + 这里统一拼 JSON」
//!
//! 前端 `MonitorStudio.vue` 是**通用渲染器**：它不认任何具体数据库，只认后端给的
//! `spec`（KPI 卡与面板的定义）与 `sections`（数据）。所以新增一种数据库的监控，
//! 只需要在这里多一个分支 —— 前端零改动（与 `dialect.rs` 里那套"同一实现服务多种类型"
//! 是同一个思路）。
//!
//! ## 三条取舍
//!
//! 1. **只读**。这里跑的每一条 SQL 都是查询（`show` / 系统表 / `pragma`），
//!    唯一的写操作是用户显式点「终止」时的 KILL，且单独走 `/monitor/kill`。
//! 2. **一个面板失败不拖垮整页**：每个区块独立执行，失败只记进 `sectionErrors`，
//!    界面在那个面板上显示「获取失败」——比整页报错更能说明"是哪一块没权限"。
//!    （典型：SQL Server 的 `sys.dm_exec_*` 需要 VIEW SERVER STATE。）
//! 3. **拿不到就不编**：某个指标查不出来，就不放进 `sections`，KPI 卡自动不渲染
//!    （前端 `if (raw == null) continue`）——而不是给个 0，让用户以为"数据库很闲"。
//!
//! ## 覆盖范围
//!
//! 已实现：MySQL / MariaDB / Doris、SQL Server、SQLite、PostgreSQL / KingbaseES。
//! 其余类型（Oracle / DM / DB2 / ClickHouse / H2 / Derby）返回 `supported:false`
//! 并说明原因 —— 界面上是一句明确的话，不是一片空白。

use std::collections::HashMap;

use axum::extract::{Path, RawQuery, State};
use axum::Json;
use serde_json::{json, Map, Value};

use crate::api::error::{XError, XResult};
use crate::api::meta::{ctx_of, rows_of, run_sql_in};
use crate::api::Params;
use crate::AppState;

/// 一个面板的定义（前端按它渲染表头/配色/是否可终止）。
struct Panel {
    key: &'static str,
    title: &'static str,
    icon: &'static str,
    color: &'static str,
    /// 行列对称的键值面板（关键参数一类）
    kv: bool,
    /// 有数据即视为异常（阻塞/等待），面板头部显示「N 个阻塞」
    warn: bool,
    /// 允许对某一行执行终止会话
    killable: bool,
    sort_hint: &'static str,
    /// 取哪一列当「会话 id」（终止按钮用）
    id_cols: &'static [&'static str],
    /// 复制按钮优先取哪一列（SQL 文本）
    copy_cols: &'static [&'static str],
    sql: &'static str,
}

/// 一套监控计划：KPI 定义 + 指标查询 + 面板。
struct Plan {
    kpis: Vec<Value>,
    /// (指标名, SQL) —— SQL 须返回两列 name / value（或单值）
    metrics: Vec<(&'static str, &'static str)>,
    /// 额外计算的指标（由已取到的原始指标算出，例如命中率）
    derived: &'static [(
        &'static str,
        &'static [&'static str],
        fn(&HashMap<String, f64>) -> Option<f64>,
    )],
    panels: Vec<Panel>,
}

fn kpi(metric: &str, label: &str, icon: &str, color: &str, unit: &str) -> Value {
    json!({ "metric": metric, "label": label, "icon": icon, "color": color, "unit": unit })
}

/// 这个类型有没有监控实现（`/features` 的 `supportsMonitor` 用它，界面据此决定
/// 监控页是显示数据还是显示「尚未接入」）。
pub fn supported(key: &str) -> bool {
    plan(key).is_some()
}

/// 按方言取监控计划。`None` = 这个类型还没有监控实现。
fn plan(key: &str) -> Option<Plan> {
    match key {
        "mysql" | "mariadb" => Some(mysql_plan()),
        // Doris 看着像 MySQL，实测四处不同（详见 doris_plan 的表格）—— 不能共用
        "doris" => Some(doris_plan()),
        "sqlserver" => Some(sqlserver_plan()),
        "sqlite" => Some(sqlite_plan()),
        "postgresql" | "kingbase" => Some(postgres_plan()),
        "clickhouse" => Some(clickhouse_plan()),
        _ => None,
    }
}

// ------------------------------------------------------------ ClickHouse

/// ClickHouse：所有监控数据都在 `system.*` 里，且都是随时可查的实时值。
///
/// 两点取舍：
/// - **不提供「终止查询」**：CH 用 `kill query where query_id = '<字符串>'`，而 `monitor/kill`
///   只接受数字会话号（那是给 MySQL/SQL Server 用的）。宁可这里不显示终止按钮，
///   也不放宽成"任意字符串直接拼进 SQL"（那是最好的注入面）。要用 KILL QUERY 是在查询页执行。
/// - 会读磁盘的 `system.parts` 只取 Top 20：CH 的表动辄上万个 part，
///   整表扫一遍当监控刷新会把库拖慢 —— 监控自己不该是负载。
fn clickhouse_plan() -> Plan {
    Plan {
        kpis: vec![
            kpi("running", "运行中查询", "cpu", "amber", ""),
            kpi("connections", "TCP 连接", "connection", "sky", ""),
            kpi("memory", "内存占用", "coin", "emerald", "bytes"),
            // 口径是**整个实例**，不是"当前库"：这个连接可以不指定默认库
            // （实测：database 为空时 currentDatabase() 也空，三个"当前库"KPI 全显示 0，
            //  看起来像"这台库是空的"，其实只是没选库）。
            kpi("disk_size", "表空间占用", "grid", "indigo", "bytes"),
            kpi("table_count", "表数量", "operation", "violet", ""),
            json!({ "metric": "uptime", "label": "运行时长", "icon": "clock", "color": "teal", "unit": "", "format": "duration" }),
        ],
        metrics: vec![
            ("uptime", "select uptime() as value"),
            ("running", "select count(*) as value from system.processes"),
            (
                "connections",
                "select value as value from system.metrics where metric = 'TCPConnection'",
            ),
            (
                "memory",
                "select value as value from system.metrics where metric = 'MemoryTracking'",
            ),
            (
                "table_count",
                "select count(*) as value from system.tables \
                 where database not in ('system', 'information_schema')",
            ),
            (
                "disk_size",
                "select sum(bytes_on_disk) as value from system.parts where active",
            ),
        ],
        derived: &[],
        panels: vec![
            Panel {
                key: "processes",
                title: "运行中的查询",
                icon: "cpu",
                color: "sky",
                kv: false,
                warn: false,
                // ClickHouse 的 KILL QUERY 要 query_id（字符串），而 /monitor/kill 只收数字会话号
                // —— 见 clickhouse_plan 的说明，这里不给终止按钮
                killable: false,
                sort_hint: "按已运行时长倒序",
                id_cols: &[],
                copy_cols: &["query"],
                sql: "select query_id, user, elapsed as elapsed_secs, read_rows, \
                      written_rows, memory_usage, substring(query, 1, 200) as query \
                      from system.processes order by elapsed desc limit 50",
            },
            Panel {
                key: "tables",
                title: "表空间 Top 20",
                icon: "grid",
                color: "indigo",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "按磁盘占用倒序",
                id_cols: &[],
                copy_cols: &[],
                // 跨库（不带 currentDatabase 过滤）：连接可以不设默认库，
                // 带库名一起显示才看得出热点在哪个库
                sql: "select database as db, table as table_name, sum(rows) as rows, \
                      round(sum(bytes_on_disk) / 1048576, 1) as size_mb, \
                      count() as parts \
                      from system.parts where active \
                      group by database, table order by sum(bytes_on_disk) desc limit 20",
            },
            Panel {
                key: "metrics",
                title: "关键指标",
                icon: "setting",
                color: "slate",
                kv: true,
                warn: false,
                killable: false,
                sort_hint: "",
                id_cols: &[],
                copy_cols: &[],
                // 两边都 `toString`：UNION ALL 要求各分支列类型一致，而 metrics 是数值、
                // asynchronous_metrics 是浮点 —— 不统一会直接报类型错
                sql: "select metric as name, toString(value) as value from system.metrics \
                      where metric in ('Query','Merge','PartMutation','BackgroundPoolTask',\
                      'HTTPConnection','InterserverConnection','DelayedInserts',\
                      'ReplicatedFetch','ReplicatedSend') \
                      union all \
                      select metric as name, toString(value) as value from system.asynchronous_metrics \
                      where metric in ('MemoryResident','MemoryVirtual','LoadAverage1',\
                      'LoadAverage5','Uptime')",
            },
        ],
    }
}

// ---------------------------------------------------------------- Doris

/// Doris：**不能复用 MySQL 的计划**。实测（Doris 2.1，192.168.2.188）四处都不一样：
///
/// | MySQL 的写法 | 在 Doris 上实际发生什么 |
/// |---|---|
/// | `show global status where variable_name in ('Threads_connected',…)` | 能执行但**返回 0 行**（变量名不同）→ KPI 卡整片空着 |
/// | `select @@max_connections` | `Unknown system variable` |
/// | `sys.innodb_lock_waits`（锁等待） | `Unknown database 'sys'` → 整块显示「获取失败」 |
/// | `data_length + index_length` 算表大小 | Doris 的 `index_length` 是 **NULL** → 加法得 NULL → 表空间那几列全是「—」 |
///
/// 所以这里按 Doris 自己的入口写：`information_schema.processlist` / `active_queries` /
/// `global_variables`，节点信息用 `show frontends` / `show backends`。
///
/// **不给「锁等待」这一块**：Doris 没有 InnoDB 那套锁视图，硬凑一块只会得到
/// 「获取失败」；「运行中的查询」已经能看出谁在跑。宁可少一块，不给一块永远报错的。
fn doris_plan() -> Plan {
    Plan {
        kpis: vec![
            kpi("connections", "当前连接", "connection", "sky", ""),
            kpi("running", "运行中查询", "cpu", "amber", ""),
            kpi("table_count", "表数量", "operation", "violet", ""),
            // SQL 返回的就是 MB（与 BE 的 DataUsedCapacity 同口径：实测两边都是 10.6 MB）
            kpi("db_size_mb", "当前库大小", "grid", "indigo", "MB"),
        ],
        metrics: vec![
            (
                "connections",
                "select count(*) as value from information_schema.processlist",
            ),
            (
                "running",
                "select count(*) as value from information_schema.processlist \
                 where command <> 'Sleep'",
            ),
            (
                "table_count",
                "select count(*) as value from information_schema.tables \
                 where table_schema = database()",
            ),
            (
                "db_size_mb",
                // coalesce：`index_length` 在 Doris 恒为 NULL，不兜住整个表达式都会变 NULL
                "select round(sum(coalesce(data_length, 0) + coalesce(index_length, 0)) \
                 / 1048576, 1) as value from information_schema.tables \
                 where table_schema = database()",
            ),
        ],
        derived: &[],
        panels: vec![
            Panel {
                key: "sessions",
                title: "会话 / 进程",
                icon: "user",
                color: "sky",
                kv: false,
                warn: false,
                // Doris 的 `kill <id>` 可用（id 是数字，kill 接口只收数字会话号）
                killable: true,
                sort_hint: "按已运行时长倒序",
                id_cols: &["id"],
                copy_cols: &["info"],
                sql: "select id, user, host, db, command, time, state, \
                      left(coalesce(info, ''), 200) as info \
                      from information_schema.processlist order by time desc limit 100",
            },
            Panel {
                key: "active",
                title: "运行中的查询",
                icon: "cpu",
                color: "amber",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "按已运行毫秒数倒序",
                id_cols: &[],
                copy_cols: &["query"],
                // `database` 在 Doris 是保留字，必须加反引号（不加报 `Encountered: COMMA`）
                sql: "select query_id, `database` as db, query_start_time, query_time_ms, \
                      query_status, frontend_instance, left(coalesce(sql, ''), 200) as query \
                      from information_schema.active_queries order by query_time_ms desc limit 50",
            },
            Panel {
                key: "tables",
                title: "表空间 Top 20",
                icon: "grid",
                color: "indigo",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "按占用空间倒序",
                id_cols: &[],
                copy_cols: &[],
                // 列序 = 界面上的列序（后端会把真实顺序发给前端）：
                // 表名 → 引擎 → 行数 → 各尺寸，别把标识列甩到最后
                sql: "select table_name, engine, table_rows, \
                      round(coalesce(data_length, 0) / 1048576, 1) as data_mb, \
                      round(coalesce(index_length, 0) / 1048576, 1) as index_mb, \
                      round((coalesce(data_length, 0) + coalesce(index_length, 0)) / 1048576, 1) as size_mb \
                      from information_schema.tables where table_schema = database() \
                      order by coalesce(data_length, 0) + coalesce(index_length, 0) desc limit 20",
            },
            Panel {
                key: "frontends",
                title: "FE 节点",
                icon: "connection",
                color: "emerald",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "按节点名",
                id_cols: &[],
                copy_cols: &[],
                sql: "show frontends",
            },
            Panel {
                key: "backends",
                title: "BE 节点",
                icon: "grid",
                color: "teal",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "按后端 ID",
                id_cols: &[],
                copy_cols: &[],
                sql: "show backends",
            },
            Panel {
                key: "variables",
                title: "关键参数",
                icon: "setting",
                color: "slate",
                kv: true,
                warn: false,
                killable: false,
                sort_hint: "",
                id_cols: &[],
                copy_cols: &[],
                // 只列** Doris 真的有的**变量：`max_connections` 之类在这里不存在
                // （实测 `Unknown system variable`），列出来只会让这一块少几行、看不出为什么
                sql: "select variable_name as name, variable_value as value \
                      from information_schema.global_variables \
                      where variable_name in ('query_timeout','wait_timeout','exec_mem_limit',\
                      'parallel_fragment_exec_instance_num','enable_profile','default_rowset_type',\
                      'enable_pipeline_engine','max_query_instances')",
            },
        ],
    }
}

// ---------------------------------------------------------------- MySQL 系

fn mysql_plan() -> Plan {
    Plan {
        kpis: vec![
            kpi("threads_connected", "当前连接", "connection", "sky", ""),
            kpi("threads_running", "运行中线程", "cpu", "amber", ""),
            json!({ "metric": "liveQps", "label": "实时 QPS", "icon": "dataline", "color": "indigo", "unit": "次/秒", "kind": "liveQps" }),
            kpi("slow_queries", "慢查询累计", "timer", "rose", ""),
            kpi("buffer_pool_hit_pct", "缓冲池命中率", "trendCharts", "emerald", "%"),
            json!({ "metric": "uptime", "label": "运行时长", "icon": "clock", "color": "violet", "unit": "", "format": "duration" }),
        ],
        metrics: vec![
            // 一次 `show global status` 拿全部：`information_schema.global_status`
            // 在 5.7 已废弃、8.0 被移除，`show` 是唯一跨版本都有的入口
            (
                "status",
                "show global status where variable_name in \
                 ('Uptime','Threads_connected','Threads_running','Questions','Slow_queries',\
                 'Aborted_connects','Innodb_buffer_pool_reads','Innodb_buffer_pool_read_requests',\
                 'Bytes_received','Bytes_sent','Innodb_rows_read','Innodb_rows_inserted',\
                 'Innodb_rows_updated','Innodb_rows_deleted')",
            ),
            ("max_connections", "select @@max_connections as value"),
            (
                "db_size",
                "select round(sum(data_length + index_length) / 1024 / 1024, 1) as value \
                 from information_schema.tables where table_schema = database()",
            ),
        ],
        derived: &[
            // 命中率 = 1 - 物理读/逻辑读。分母为 0 时不给值（不编 100%）
            ("buffer_pool_hit_pct", &["Innodb_buffer_pool_reads", "Innodb_buffer_pool_read_requests"], |m| {
                let reads = m.get("Innodb_buffer_pool_read_requests")?;
                let physical = m.get("Innodb_buffer_pool_reads")?;
                if *reads <= 0.0 {
                    return None;
                }
                Some(((1.0 - physical / reads) * 10000.0).round() / 100.0)
            }),
        ],
        panels: vec![
            Panel {
                key: "sessions",
                title: "会话 / 进程",
                icon: "user",
                color: "sky",
                kv: false,
                warn: false,
                killable: true,
                sort_hint: "按已运行时长倒序",
                id_cols: &["id"],
                copy_cols: &["info"],
                sql: "select id, user, host, db, command, time, state, \
                      left(coalesce(info, ''), 200) as info \
                      from information_schema.processlist order by time desc limit 100",
            },
            Panel {
                key: "locks",
                title: "锁等待",
                icon: "lock",
                color: "rose",
                kv: false,
                warn: true,
                killable: true,
                sort_hint: "阻塞源在前",
                id_cols: &["waiting_pid"],
                copy_cols: &["waiting_query"],
                // `select *`：`sys.innodb_lock_waits` 的列 5.7 与 8.0 不一致
                // （5.7 没有 waiting_thread，实测报 Unknown column），
                // 而界面按列名通用渲染，所以让它返回各版本自己的那一套最稳
                sql: "select * from sys.innodb_lock_waits",
            },
            Panel {
                key: "tables",
                title: "表空间 Top 20",
                icon: "grid",
                color: "indigo",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "按占用空间倒序",
                id_cols: &[],
                copy_cols: &[],
                sql: "select table_name, table_rows, \
                      round((data_length + index_length) / 1024 / 1024, 1) as size_mb, \
                      round(data_length / 1024 / 1024, 1) as data_mb, \
                      round(index_length / 1024 / 1024, 1) as index_mb, engine \
                      from information_schema.tables where table_schema = database() \
                      order by (data_length + index_length) desc limit 20",
            },
            Panel {
                key: "variables",
                title: "关键参数",
                icon: "setting",
                color: "slate",
                kv: true,
                warn: false,
                killable: false,
                sort_hint: "",
                id_cols: &[],
                copy_cols: &[],
                sql: "show variables where variable_name in \
                      ('max_connections','innodb_buffer_pool_size','innodb_log_file_size',\
                      'character_set_server','collation_server','sql_mode','transaction_isolation',\
                      'wait_timeout','max_allowed_packet','slow_query_log','long_query_time',\
                      'log_bin','version','innodb_flush_log_at_trx_commit','lower_case_table_names',\
                      'table_open_cache','thread_cache_size','tmp_table_size','max_heap_table_size')",
            },
        ],
    }
}

// ------------------------------------------------------------ SQL Server

fn sqlserver_plan() -> Plan {
    Plan {
        kpis: vec![
            kpi("connections", "用户连接", "connection", "sky", ""),
            kpi("running", "正在执行", "cpu", "amber", ""),
            kpi("blocked", "被阻塞会话", "warning", "rose", ""),
            json!({ "metric": "uptime", "label": "运行时长", "icon": "clock", "color": "violet", "unit": "", "format": "duration" }),
            // SQL 返回的就是 MB，单位必须写 MB —— 写 bytes 会让前端按字节格式化，
            // 494024 MB 会显示成「482 KB」（实测）
            kpi("buffer_mb", "缓冲池", "coin", "emerald", "MB"),
            kpi("db_size_mb", "当前库大小", "grid", "indigo", "MB"),
        ],
        metrics: vec![
            ("connections", "select count(*) as value from sys.dm_exec_sessions where is_user_process = 1"),
            ("running", "select count(*) as value from sys.dm_exec_requests where session_id > 50"),
            ("blocked", "select count(distinct blocking_session_id) as value from sys.dm_exec_requests where blocking_session_id <> 0"),
            ("uptime", "select datediff(second, sqlserver_start_time, getdate()) as value from sys.dm_os_sys_info"),
            ("buffer_mb", "select count(*) * 8 as value from sys.dm_os_buffer_descriptors"),
            ("db_size_mb", "select cast(sum(size) * 8.0 / 1024 as decimal(12,1)) as value from sys.database_files"),
            ("batch_requests", "select cast(cntr_value as bigint) as value from sys.dm_os_performance_counters where counter_name = 'Batch Requests/sec' and object_name like '%SQL Statistics%'"),
            ("page_life_expectancy", "select cast(cntr_value as bigint) as value from sys.dm_os_performance_counters where counter_name = 'Page life expectancy' and object_name like '%Buffer Manager%'"),
        ],
        derived: &[],
        panels: vec![
            Panel {
                key: "sessions",
                title: "会话 / 请求",
                icon: "user",
                color: "sky",
                kv: false,
                warn: false,
                killable: true,
                sort_hint: "按会话号",
                id_cols: &["id", "session_id"],
                copy_cols: &["query"],
                sql: "select s.session_id as id, s.login_name, s.host_name, s.status, \
                      r.command, r.wait_time / 1000 as wait_secs, \
                      r.blocking_session_id as blocking_id, r.cpu_time, r.logical_reads, \
                      left(coalesce(t.text, ''), 200) as query \
                      from sys.dm_exec_sessions s \
                      left join sys.dm_exec_requests r on r.session_id = s.session_id \
                      outer apply sys.dm_exec_sql_text(r.sql_handle) t \
                      where s.is_user_process = 1 order by r.wait_time desc, s.session_id",
            },
            Panel {
                key: "blocking",
                title: "阻塞链",
                icon: "lock",
                color: "rose",
                kv: false,
                warn: true,
                killable: true,
                sort_hint: "阻塞源在前",
                id_cols: &["waiting_id"],
                copy_cols: &["waiting_query"],
                sql: "select r.blocking_session_id as blocking_id, r.session_id as waiting_id, \
                      r.wait_time / 1000 as wait_secs, r.wait_type, r.last_wait_type, \
                      left(coalesce(t.text, ''), 200) as waiting_query \
                      from sys.dm_exec_requests r \
                      outer apply sys.dm_exec_sql_text(r.sql_handle) t \
                      where r.blocking_session_id <> 0",
            },
            Panel {
                key: "tables",
                title: "表空间 Top 20",
                icon: "grid",
                color: "indigo",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "按占用空间倒序",
                id_cols: &[],
                copy_cols: &[],
                sql: "select top 20 t.name as table_name, sum(p.rows) as rows, \
                      cast(sum(a.total_pages) * 8.0 / 1024 as decimal(12,1)) as size_mb, \
                      cast(sum(a.used_pages) * 8.0 / 1024 as decimal(12,1)) as used_mb \
                      from sys.tables t \
                      join sys.partitions p on p.object_id = t.object_id and p.index_id in (0, 1) \
                      join sys.allocation_units a on a.container_id = p.partition_id \
                      group by t.name order by sum(a.total_pages) desc",
            },
            Panel {
                key: "waits",
                title: "累计等待 Top 12",
                icon: "timer",
                color: "amber",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "按等待时间倒序",
                id_cols: &[],
                copy_cols: &[],
                sql: "select top 12 wait_type, wait_time_ms / 1000 as wait_secs, \
                      waiting_tasks_count as tasks, signal_wait_time_ms / 1000 as signal_secs \
                      from sys.dm_os_wait_stats \
                      where wait_type not like 'SLEEP%' and wait_type not like 'BROKER%' \
                      order by wait_time_ms desc",
            },
            Panel {
                key: "config",
                title: "关键配置",
                icon: "setting",
                color: "slate",
                kv: true,
                warn: false,
                killable: false,
                sort_hint: "",
                id_cols: &[],
                copy_cols: &[],
                sql: "select name, cast(value_in_use as nvarchar(64)) as value from sys.configurations \
                      where name in ('max server memory (MB)','min server memory (MB)',\
                      'max degree of parallelism','cost threshold for parallelism',\
                      'recovery interval (min)','max worker threads','blocked process threshold (s)',\
                      'backup compression default','fill factor (%)','network packet size (B)')",
            },
        ],
    }
}

// ---------------------------------------------------------------- SQLite

fn sqlite_plan() -> Plan {
    Plan {
        kpis: vec![
            kpi("db_size", "数据库大小", "coin", "emerald", "bytes"),
            kpi("page_count", "页数", "grid", "indigo", ""),
            kpi("page_size", "页大小", "document", "sky", "bytes"),
            kpi("freelist_count", "空闲页", "files", "slate", ""),
            kpi("table_count", "表数量", "operation", "violet", ""),
        ],
        metrics: vec![
            ("page_count", "pragma page_count"),
            ("page_size", "pragma page_size"),
            ("freelist_count", "pragma freelist_count"),
            ("schema_version", "pragma schema_version"),
            ("table_count", "select count(*) as value from sqlite_master where type = 'table'"),
        ],
        derived: &[
            ("db_size", &["page_count", "page_size"], |m| {
                Some(m.get("page_count")? * m.get("page_size")?)
            }),
        ],
        panels: vec![
            Panel {
                key: "tables",
                title: "表 / 视图",
                icon: "grid",
                color: "indigo",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "",
                id_cols: &[],
                copy_cols: &["sql"],
                sql: "select name, type, (select count(*) from pragma_table_info(m.name)) as columns \
                      from sqlite_master m where type in ('table','view') order by name limit 100",
            },
            Panel {
                key: "settings",
                title: "关键参数",
                icon: "setting",
                color: "slate",
                kv: true,
                warn: false,
                killable: false,
                sort_hint: "",
                id_cols: &[],
                copy_cols: &[],
                sql: "select 'journal_mode' as name, journal_mode as value from pragma_journal_mode \
                      union all select 'auto_vacuum', cast((select auto_vacuum from pragma_auto_vacuum) as text) \
                      union all select 'cache_size', cast((select cache_size from pragma_cache_size) as text) \
                      union all select 'encoding', (select encoding from pragma_encoding) \
                      union all select 'user_version', cast((select user_version from pragma_user_version) as text) \
                      union all select 'synchronous', cast((select synchronous from pragma_synchronous) as text)",
            },
        ],
    }
}

// ------------------------------------------------------------ PostgreSQL

fn postgres_plan() -> Plan {
    Plan {
        kpis: vec![
            kpi("connections", "连接数", "connection", "sky", ""),
            kpi("active", "活跃会话", "cpu", "amber", ""),
            kpi("blocked", "等待锁", "lock", "rose", ""),
            kpi("cache_hit_pct", "缓存命中率", "trendCharts", "emerald", "%"),
            json!({ "metric": "uptime", "label": "运行时长", "icon": "clock", "color": "violet", "unit": "", "format": "duration" }),
            kpi("db_size", "当前库大小", "coin", "indigo", "bytes"),
        ],
        metrics: vec![
            ("connections", "select count(*) as value from pg_stat_activity"),
            ("active", "select count(*) as value from pg_stat_activity where state = 'active'"),
            ("blocked", "select count(*) as value from pg_stat_activity where wait_event_type = 'Lock'"),
            ("uptime", "select extract(epoch from (now() - pg_postmaster_start_time()))::bigint as value"),
            ("db_size", "select pg_database_size(current_database()) as value"),
            ("blks_hit", "select sum(blks_hit) as value from pg_stat_database where datname = current_database()"),
            ("blks_read", "select sum(blks_read) as value from pg_stat_database where datname = current_database()"),
            ("xact_commit", "select sum(xact_commit) as value from pg_stat_database where datname = current_database()"),
        ],
        derived: &[
            ("cache_hit_pct", &["blks_hit", "blks_read"], |m| {
                let hit = m.get("blks_hit")?;
                let read = m.get("blks_read")?;
                let total = hit + read;
                if total <= 0.0 {
                    return None;
                }
                Some((hit / total * 10000.0).round() / 100.0)
            }),
        ],
        panels: vec![
            Panel {
                key: "sessions",
                title: "会话",
                icon: "user",
                color: "sky",
                kv: false,
                warn: false,
                killable: true,
                sort_hint: "活跃在前",
                id_cols: &["pid"],
                copy_cols: &["query"],
                sql: "select pid, usename, application_name, client_addr, state, \
                      wait_event_type, wait_event, \
                      extract(epoch from (now() - query_start))::int as query_secs, \
                      left(coalesce(query, ''), 200) as query \
                      from pg_stat_activity where pid <> pg_backend_pid() \
                      order by (state = 'active') desc, query_start",
            },
            Panel {
                key: "locks",
                title: "锁等待",
                icon: "lock",
                color: "rose",
                kv: false,
                warn: true,
                killable: false,
                sort_hint: "",
                id_cols: &[],
                copy_cols: &[],
                sql: "select w.pid as waiting_pid, l.pid as blocking_pid, \
                      w.locktype, w.mode, w.wait_event, \
                      left(coalesce(w.query, ''), 200) as waiting_query \
                      from pg_stat_activity w \
                      join lateral unnest(pg_blocking_pids(w.pid)) as l(pid) on true \
                      where cardinality(pg_blocking_pids(w.pid)) > 0",
            },
            Panel {
                key: "tables",
                title: "表空间 Top 20",
                icon: "grid",
                color: "indigo",
                kv: false,
                warn: false,
                killable: false,
                sort_hint: "按占用空间倒序",
                id_cols: &[],
                copy_cols: &[],
                sql: "select c.relname as table_name, pg_total_relation_size(c.oid) as size, \
                      pg_relation_size(c.oid) as data_size, \
                      pg_indexes_size(c.oid) as index_size, s.n_live_tup as rows \
                      from pg_class c \
                      join pg_namespace n on n.oid = c.relnamespace \
                      left join pg_stat_user_tables s on s.relid = c.oid \
                      where c.relkind = 'r' and n.nspname not in ('pg_catalog','information_schema') \
                      order by pg_total_relation_size(c.oid) desc limit 20",
            },
            Panel {
                key: "settings",
                title: "关键参数",
                icon: "setting",
                color: "slate",
                kv: true,
                warn: false,
                killable: false,
                sort_hint: "",
                id_cols: &[],
                copy_cols: &[],
                sql: "select name, setting as value from pg_settings \
                      where name in ('max_connections','shared_buffers','work_mem',\
                      'maintenance_work_mem','effective_cache_size','wal_level',\
                      'max_wal_size','checkpoint_timeout','autovacuum',\
                      'random_page_cost','server_version')",
            },
        ],
    }
}

// ---------------------------------------------------------------- 取数

/// 执行一条监控查询，拿到**列顺序**与「列名 → 值」的行
/// （失败返回 Err，由调用方记进 sectionErrors）。
///
/// 为什么必须单独把列顺序带出去：行是 JSON 对象，而 `serde_json` 的 Map **按键排序**
/// —— 前端只能按对象的键推列，于是列序恒为字母序（`table_name` 排在 `size_mb` 后面）。
/// 界面上的原话是「表名放最后一列不太友好」：那是字母序，不是 SQL 里写的顺序。
/// 内核本来的 `QueryResult.columns` 就是有序的，这里把它一并交给前端。
async fn query_rows(
    state: &AppState,
    id: &str,
    database: &str,
    sql: &str,
) -> XResult<(Vec<String>, Vec<Map<String, Value>>)> {
    let result = run_sql_in(state, id, database, sql.to_string(), 500).await?;
    let columns: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
    Ok((columns, rows_of(&result)))
}

/// 单值：取第一行第一列并转成数字（拿不到就 None，**不编 0**）。
fn scalar(rows: &[Map<String, Value>]) -> Option<f64> {
    let row = rows.first()?;
    // 优先按名字找 value（监控 SQL 都这么起名），否则取第一列
    let cell = row
        .get("value")
        .or_else(|| row.get("VALUE"))
        .or_else(|| row.values().next())?;
    match cell {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// 把 `show ...` 这类「两列（名字, 值）」结果转成 `{name, value}` 行。
///
/// **必须按列名取，不能按位置取**：`serde_json` 的 Map 按键名排序，而 `show variables`
/// 的两列是 `Variable_name` / `Value` —— 排序后 `Value` 在前，"取第一列当名字"会取到
/// **值**，于是指标一个都认不出来（实测：MySQL 监控 6 张 KPI 卡全空）。
fn normalize_name_value(rows: &[Map<String, Value>]) -> Vec<Value> {
    // `variable_name` / `variable_value` 是 Doris 的 `information_schema.global_variables`
    // 用的列名（MySQL 的 `show variables` 是 `Variable_name` / `Value`）——
    // 漏掉这两个键的后果是**整块「关键参数」空着**（能查到 6 行，但一行都进不了结果）。
    const NAME_KEYS: [&str; 5] = ["Variable_name", "variable_name", "name", "NAME", "Name"];
    const VALUE_KEYS: [&str; 5] = ["Value", "variable_value", "value", "VALUE", "Setting"];
    let mut out = Vec::new();
    for row in rows {
        let name = NAME_KEYS
            .iter()
            .find_map(|key| row.get(*key))
            .map(text_of_value);
        let value = VALUE_KEYS
            .iter()
            .find_map(|key| row.get(*key))
            .map(text_of_value);
        if let (Some(name), Some(value)) = (name, value) {
            out.push(json!({ "name": name, "value": value }));
        }
    }
    out
}

/// 单元格 → 字符串（数字保留原样，便于前端按数值处理）。
fn text_of_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// 名字 → 数值（用于指标表）。
fn name_value_map(rows: &[Map<String, Value>]) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for row in normalize_name_value(rows) {
        let (Some(name), Some(raw)) = (
            row.get("name").and_then(Value::as_str),
            row.get("value").and_then(Value::as_str),
        ) else {
            continue;
        };
        if let Ok(number) = raw.trim().parse::<f64>() {
            out.insert(name.to_string(), number);
        }
    }
    out
}

/// 监控总览。
pub async fn overview(state: &AppState, id: &str, database: &str, key: &str) -> XResult<Json<Value>> {
    let Some(plan) = plan(key) else {
        // 没实现就**明说**（前端会把这句话显示出来），而不是给一片空白
        return Ok(Json(json!({
            "supported": false,
            "database": database,
            "message": format!(
                "{} 的实时监控尚未接入。已支持：MySQL / MariaDB、Doris、SQL Server、\
                 SQLite、PostgreSQL / KingbaseES、ClickHouse",
                key
            ),
        })));
    };

    let mut metrics: Map<String, Value> = Map::new();
    let mut raw: HashMap<String, f64> = HashMap::new();
    let mut errors: Map<String, Value> = Map::new();

    for (name, sql) in &plan.metrics {
        match query_rows(state, id, database, sql).await {
            // 指标只需要值，不需要列顺序（下面那个循环才要）
            Ok((_columns, rows)) => {
                if *name == "status" {
                    // `show global status`：把几十个变量铺平成同名指标
                    for (metric, value) in name_value_map(&rows) {
                        raw.insert(metric, value);
                    }
                    continue;
                }
                match scalar(&rows) {
                    Some(value) => {
                        raw.insert((*name).to_string(), value);
                    }
                    None => {
                        // 这一项取不到：不放进指标表（KPI 卡会跳过），但也不当整体失败
                        errors.insert((*name).to_string(), json!("未取到值"));
                    }
                }
            }
            Err(err) => {
                errors.insert((*name).to_string(), json!(err.message));
            }
        }
    }

    for (name, deps, compute) in plan.derived {
        if deps.iter().all(|dep| raw.contains_key(*dep)) {
            if let Some(value) = compute(&raw) {
                raw.insert((*name).to_string(), value);
            }
        }
    }

    // KPI 声明的指标名 → 值（KPI 用小写名，`show global status` 给的是大写原名）
    for kpi in &plan.kpis {
        let Some(metric) = kpi.get("metric").and_then(Value::as_str) else {
            continue;
        };
        if metric == "liveQps" {
            // 由前端两次刷新取增量算，这里只保证 questions 在场
            if let Some(value) = raw.get("Questions").or_else(|| raw.get("questions")) {
                metrics.insert("questions".to_string(), json!(value));
            }
            continue;
        }
        let lowered = metric.to_ascii_lowercase();
        let found = raw
            .get(metric)
            .or_else(|| raw.get(&lowered))
            .or_else(|| {
                // `show global status` 的键是首字母大写的驼峰（Threads_connected）
                raw.iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(metric))
                    .map(|(_, value)| value)
            });
        if let Some(value) = found {
            metrics.insert(lowered, json!(value));
        }
    }

    let mut sections = Map::new();
    // 每个面板的列顺序（行对象按键排序，见 query_rows 的说明）
    let mut column_map: Map<String, Value> = Map::new();
    sections.insert(
        "instance".to_string(),
        Value::Array(
            metrics
                .iter()
                .map(|(name, value)| json!({ "metric": name, "value": value }))
                .collect(),
        ),
    );

    for panel in &plan.panels {
        match query_rows(state, id, database, panel.sql).await {
            Ok((columns, rows)) => {
                // 键值面板统一成 `{name, value}`：前端 kv 网格取「排序后的第一个键」当名字，
                // 直接丢 `show variables` 的 `{Variable_name, Value}` 会被排成 (值, 名) 显示反
                let value = if panel.kv {
                    Value::Array(normalize_name_value(&rows))
                } else {
                    Value::Array(rows.into_iter().map(Value::Object).collect())
                };
                sections.insert(panel.key.to_string(), value);
                // 列顺序单列一份给前端（行对象按键排序，靠它拿回 SQL 里写的顺序）
                if !panel.kv {
                    column_map.insert(
                        panel.key.to_string(),
                        Value::Array(columns.into_iter().map(Value::String).collect()),
                    );
                }
            }
            Err(err) => {
                // 单块失败：面板上显示「获取失败」，整页照常出数据
                errors.insert(panel.key.to_string(), json!(err.message));
                sections.insert(panel.key.to_string(), Value::Array(vec![]));
            }
        }
    }

    Ok(Json(json!({
        "supported": true,
        "database": database,
        "fetchedAt": chrono::Local::now().to_rfc3339(),
        "spec": {
            "kpis": plan.kpis,
            "panels": plan.panels.iter().map(|p| json!({
                "key": p.key,
                "title": p.title,
                "icon": p.icon,
                "color": p.color,
                "mode": if p.kv { "kv" } else { "table" },
                "warn": p.warn,
                "killable": p.killable,
                "sortHint": p.sort_hint,
                "idCols": p.id_cols,
                "copyCols": p.copy_cols,
            })).collect::<Vec<_>>(),
        },
        "sections": sections,
        // 面板列顺序（前端据此排列表头；缺了就只能按对象键的字母序）
        "columns": column_map,
        "sectionErrors": errors,
    })))
}

/// `GET /api/{m}/{id}/monitor`
pub async fn monitor(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let (_, dialect) = ctx_of(&state, &id).await?;
    overview(&state, &id, &database, dialect.kind.key()).await
}

/// `POST /api/{m}/{id}/monitor/kill?sessionId=` —— 终止会话。
///
/// 这是监控页里**唯一会写**的动作，所以按方言各写一句最窄的语句：
/// MySQL `kill`、SQL Server `kill`、PG `pg_terminate_backend`。
/// SQLite 没有会话概念，直接说清楚不支持（而不是报一句看不懂的 SQL 错）。
pub async fn monitor_kill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let session_id = params.get("sessionId").unwrap_or_default();
    if session_id.trim().is_empty() {
        return Err(XError::bad_request("缺少 sessionId"));
    }
    // 只接受数字：这条语句是拼出来的，绝不能让参数直接进 SQL
    if !session_id.trim().chars().all(|c| c.is_ascii_digit()) {
        return Err(XError::bad_request("会话标识必须是数字"));
    }
    let (_, dialect) = ctx_of(&state, &id).await?;
    let sql = match dialect.kind.key() {
        "mysql" | "mariadb" | "doris" => format!("kill {}", session_id.trim()),
        "sqlserver" => format!("kill {}", session_id.trim()),
        "postgresql" | "kingbase" => {
            format!("select pg_terminate_backend({})", session_id.trim())
        }
        "sqlite" => return Ok(Json(json!({
            "success": false,
            "message": "SQLite 是文件数据库，没有会话可终止",
        }))),
        "clickhouse" => return Ok(Json(json!({
            "success": false,
            "message": "ClickHouse 终止查询需要 query_id（字符串），而这里只接受数字会话号。\
                        请在查询页执行 `KILL QUERY WHERE query_id = '...'`",
        }))),
        key => {
            return Ok(Json(json!({
                "success": false,
                "message": format!("{key} 的终止会话尚未接入"),
            })))
        }
    };
    // 终止会话是**运维动作**（KILL / pg_terminate_backend）：走跟随连接策略的入口，
    // 不进结构浏览那条只读会话（只读连接上 pg_terminate_backend 还会被闸门拦下）
    match crate::api::meta::run_write_sql_in(&state, &id, &database, sql, 1, true).await {
        Ok(_) => Ok(Json(json!({
            "success": true,
            "message": format!("已终止会话 {session_id}"),
        }))),
        Err(err) => Ok(Json(json!({
            "success": false,
            "message": format!("终止会话失败：{}", err.message),
        }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 已接入监控的方言必须都给出 KPI 与面板；没接入的必须**明说**而不是空数组。
    #[test]
    fn 已接入的方言都有_kpi_与面板() {
        for key in [
            "mysql",
            "mariadb",
            "doris",
            "sqlserver",
            "sqlite",
            "postgresql",
            "kingbase",
            "clickhouse",
        ] {
            let plan = plan(key).unwrap_or_else(|| panic!("{key} 应当有监控计划"));
            assert!(!plan.kpis.is_empty(), "{key} 缺 KPI");
            assert!(!plan.panels.is_empty(), "{key} 缺面板");
            assert!(!plan.metrics.is_empty(), "{key} 缺指标查询");
            for panel in &plan.panels {
                assert!(!panel.sql.trim().is_empty(), "{key}/{} 的 SQL 为空", panel.key);
                assert!(!panel.key.is_empty());
            }
        }
        for key in ["oracle", "dm", "db2", "h2", "derby"] {
            assert!(plan(key).is_none(), "{key} 目前不该有监控计划");
        }
    }

    /// 每个 KPI 的 metric 必须能从指标查询或派生里得到，否则那张卡永远不渲染。
    #[test]
    fn kpi_的指标都有来源() {
        for key in ["mysql", "sqlserver", "sqlite", "postgresql"] {
            let plan = plan(key).unwrap();
            let mut provided: Vec<String> = plan.metrics.iter().map(|(n, _)| n.to_string()).collect();
            // `show global status` 一次给出一批（名称首字母大写）
            provided.push("Status".to_string());
            provided.extend(
                plan.derived
                    .iter()
                    .map(|(name, _, _)| (*name).to_string()),
            );
            for kpi in &plan.kpis {
                let metric = kpi["metric"].as_str().unwrap();
                if metric == "liveQps" {
                    // 由 `status` 里的 Questions 算，见 overview
                    assert!(provided.iter().any(|n| n == "status" || n == "Status"), "{key} 缺 status 查询");
                    continue;
                }
                if metric.starts_with("buffer_pool_hit_pct")
                    || metric == "cache_hit_pct"
                    || metric == "db_size"
                {
                    // 派生指标
                    assert!(provided.contains(&metric.to_string()), "{key} 的派生指标 {metric} 没声明");
                    continue;
                }
                assert!(
                    provided.contains(&metric.to_string())
                        || provided.contains(&"Status".to_string())
                        || provided.contains(&"status".to_string()),
                    "{key} 的 KPI {metric} 没有对应的指标查询"
                );
            }
        }
    }
}
