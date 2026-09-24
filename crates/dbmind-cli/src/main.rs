//! DBMind 命令行壳。
//!
//! 这一层**故意很薄**：解析参数 → 调 `DbMindEngine` → 渲染结果。
//! 任何业务判断（安全、超时、取消、历史）都在内核里，CLI 无权绕过。

use clap::{Parser, Subcommand};
use dbmind_core::{
    AccessContext, ConnectionConfig, ConnectionKind, DbMindEngine, DbMindError, ErrorCode, QueryRequest,
};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "dbmind",
    version = dbmind_core::VERSION,
    about = "DBMind —— 数据库工作台命令行",
    long_about = "所有命令都走同一内核，因此与桌面/Web/MCP 的行为完全一致。"
)]
struct Cli {
    /// 元数据库路径（默认 ~/.dbmind/dbmind.db，可用 DBMIND_HOME 整体重定向）
    #[arg(long, global = true)]
    store: Option<PathBuf>,

    /// 以 JSON 输出（便于脚本消费）
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 连接管理
    Conn {
        #[command(subcommand)]
        command: ConnCommand,
    },
    /// 执行 SQL（--sql 或 --file 二选一）
    Query {
        /// 连接名或 id
        #[arg(long)]
        conn: String,
        /// 直接给出 SQL
        #[arg(long)]
        sql: Option<String>,
        /// 从文件读取 SQL
        #[arg(long)]
        file: Option<PathBuf>,
        /// 单页最大行数
        #[arg(long, default_value_t = 2000)]
        max_rows: usize,
        /// 超时（毫秒）
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,
        /// 执行前先打印将要执行的语句
        #[arg(long)]
        echo: bool,
    },
    /// 列出表与视图
    Tables {
        #[arg(long)]
        conn: String,
        /// 绕过结构缓存，直接回源数据库
        #[arg(long)]
        refresh: bool,
    },
    /// 查看表结构
    Columns {
        #[arg(long)]
        conn: String,
        #[arg(long)]
        table: String,
        /// 绕过结构缓存，直接回源数据库
        #[arg(long)]
        refresh: bool,
    },
    /// 连接自检
    Test {
        #[arg(long)]
        conn: String,
    },
    /// 查询历史
    History {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        conn: Option<String>,
        /// 清空历史
        #[arg(long)]
        clear: bool,
    },
    /// 连接类型目录（来自 plugins/connection-types/*.yaml）
    Types,
    /// 外置驱动管理（agent 运行时用）
    Driver {
        #[command(subcommand)]
        command: DriverCommand,
    },
    /// 运行时摘要：存储位置、实现进度、安全开关
    Status,
    /// 读写设置（安全开关在此，壳层无法用命令行绕过）
    Setting {
        #[command(subcommand)]
        command: SettingCommand,
    },
}

#[derive(Subcommand)]
enum ConnCommand {
    /// 新增连接
    Add {
        #[arg(long)]
        name: String,
        /// 类型 key，见 `dbmind types`
        #[arg(long)]
        kind: String,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        database: Option<String>,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        password: Option<String>,
        /// 文件型数据库的路径
        #[arg(long)]
        file: Option<String>,
        /// 标记为只读（内核会在执行前拦截写语句）
        #[arg(long)]
        read_only: bool,
    },
    /// 列出连接
    List,
    /// 删除连接
    Remove { name_or_id: String },
    /// 切换只读标记
    ReadOnly {
        name_or_id: String,
        /// 传 false 可解除只读
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        value: bool,
    },
}

#[derive(Subcommand)]
enum DriverCommand {
    /// 列出需要外置驱动的类型、Maven 坐标与安装状态
    List,
    /// 从 Maven Central 下载该 agentKey 的驱动 jar 到驱动目录
    Fetch {
        /// 如 h2 / postgresql / mysql（见 `dbmind driver list`）
        agent_key: String,
    },
}

#[derive(Subcommand)]
enum SettingCommand {
    /// 列出全部设置
    List,
    /// 读取单项
    Get { key: String },
    /// 写入单项
    Set { key: String, value: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("[{}] {}", err.code_str(), err.message);
            if let Some(detail) = &err.detail {
                eprintln!("  提示: {detail}");
            }
            ExitCode::from(exit_code(&err))
        }
    }
}

fn exit_code(err: &DbMindError) -> u8 {
    match err.code {
        // 约定：取消 = 130（与 shell 的 Ctrl+C 一致），便于脚本判断
        ErrorCode::QueryCanceled => 130,
        ErrorCode::ConnNotFound => 2,
        ErrorCode::QueryInvalid | ErrorCode::ConnInvalid => 3,
        ErrorCode::SafetyReadOnly | ErrorCode::SafetyAiReadOnly | ErrorCode::SafetyProduction => 4,
        ErrorCode::QueryTimeout => 5,
        _ => 1,
    }
}

fn engine(store: &Option<PathBuf>) -> dbmind_core::Result<DbMindEngine> {
    match store {
        Some(path) => DbMindEngine::open(path),
        None => DbMindEngine::open_default(),
    }
}

fn run(cli: Cli) -> dbmind_core::Result<()> {
    let engine = engine(&cli.store)?;
    let json = cli.json;

    match cli.command {
        Command::Conn { command } => match command {
            ConnCommand::Add {
                name,
                kind,
                host,
                port,
                database,
                user,
                password,
                file,
                read_only,
            } => {
                let kind = engine.resolve_kind(&kind)?;
                let mut config = ConnectionConfig::new(name, kind);
                config.host = host;
                config.port = port;
                config.database = database;
                config.username = user;
                config.password = password;
                config.file_path = file;

                let record = engine.add_connection(config)?;
                if read_only {
                    engine.set_read_only(&record.id, true)?;
                }
                let record = engine.require_connection(&record.id)?;
                if json {
                    print_json(&record)?;
                } else {
                    println!(
                        "已添加连接 {}（{}，类型 {}）",
                        record.name(),
                        record.id,
                        record.kind().label()
                    );
                    if record.read_only {
                        println!("  · 已标记为只读：写语句会被内核拒绝");
                    }
                }
            }
            ConnCommand::List => {
                let list = engine.list_connections()?;
                if json {
                    print_json(&list)?;
                } else if list.is_empty() {
                    println!("（还没有连接，用 `dbmind conn add` 添加）");
                } else {
                    let rows: Vec<Vec<String>> = list
                        .iter()
                        .map(|c| {
                            vec![
                                c.name().to_string(),
                                c.kind().key().to_string(),
                                target_of(c),
                                if c.read_only { "只读".into() } else { "-".into() },
                                if c.kind().implemented() {
                                    "就绪".into()
                                } else {
                                    "未接入".into()
                                },
                            ]
                        })
                        .collect();
                    print_table(&["名称", "类型", "目标", "权限", "驱动"], rows);
                }
            }
            ConnCommand::Remove { name_or_id } => {
                let removed = engine.remove_connection(&name_or_id)?;
                if json {
                    print_json(&serde_json::json!({ "removed": removed }))?;
                } else if removed {
                    println!("已删除连接 {name_or_id}");
                } else {
                    println!("连接 {name_or_id} 不存在");
                }
            }
            ConnCommand::ReadOnly { name_or_id, value } => {
                let record = engine.set_read_only(&name_or_id, value)?;
                if json {
                    print_json(&record)?;
                } else {
                    println!(
                        "连接 {} 现在{}",
                        record.name(),
                        if record.read_only { "是只读" } else { "可写" }
                    );
                }
            }
        },

        Command::Query {
            conn,
            sql,
            file,
            max_rows,
            timeout_ms,
            echo,
        } => {
            let sql = match (sql, file) {
                (Some(sql), _) => sql,
                (None, Some(path)) => std::fs::read_to_string(&path)?,
                (None, None) => {
                    return Err(DbMindError::new(
                        ErrorCode::QueryInvalid,
                        "请用 --sql 或 --file 提供 SQL",
                    ))
                }
            };
            if echo {
                eprintln!("-- 执行于连接 {conn}");
                eprintln!("{}", sql.trim());
            }
            let request = QueryRequest::new(conn, &sql)
                .with_options(dbmind_core::QueryOptions { max_rows, timeout_ms });
            let result = engine.execute(request, AccessContext::Cli)?;

            if json {
                print_json(&result)?;
            } else {
                render_result(&result);
            }
        }

        Command::Tables { conn, refresh } => {
            let tables = if refresh {
                engine.list_tables_fresh(&conn)?
            } else {
                engine.list_tables(&conn)?
            };
            if json {
                print_json(&tables)?;
            } else {
                if tables.is_empty() {
                    println!("（没有表）");
                } else {
                    let rows: Vec<Vec<String>> = tables
                        .iter()
                        .map(|t| vec![t.name.clone(), t.kind.as_str().to_string()])
                        .collect();
                    print_table(&["名称", "类型"], rows);
                }
                print_schema_cache(&engine, &conn);
            }
        }

        Command::Columns { conn, table, refresh } => {
            let columns = if refresh {
                engine.list_columns_fresh(&conn, &table)?
            } else {
                engine.list_columns(&conn, &table)?
            };
            if json {
                print_json(&columns)?;
            } else {
                let rows: Vec<Vec<String>> = columns
                    .iter()
                    .map(|c| {
                        vec![
                            c.name.clone(),
                            c.type_name.clone().unwrap_or_else(|| "-".into()),
                            if c.nullable {
                                "可空".into()
                            } else {
                                "非空".into()
                            },
                            if c.primary_key { "是".into() } else { "-".into() },
                            c.default_value.clone().unwrap_or_else(|| "-".into()),
                        ]
                    })
                    .collect();
                print_table(&["列", "类型", "空值", "主键", "默认值"], rows);
                print_schema_cache(&engine, &conn);
            }
        }

        Command::Test { conn } => {
            let report = engine.test_connection(&conn)?;
            if json {
                print_json(&report)?;
            } else {
                println!(
                    "{} 自检通过（{} 运行时，耗时 {} ms{}）",
                    report.kind.label(),
                    report.runtime_mode.as_str(),
                    report.latency_ms,
                    report
                        .server_version
                        .map(|v| format!("，{v}"))
                        .unwrap_or_default()
                );
            }
        }

        Command::History { limit, conn, clear } => {
            if clear {
                let removed = engine.clear_history()?;
                if json {
                    print_json(&serde_json::json!({ "cleared": removed }))?;
                } else {
                    println!("已清空 {removed} 条历史");
                }
            } else {
                let history = engine.history(limit, conn.as_deref())?;
                if json {
                    print_json(&history)?;
                } else if history.is_empty() {
                    println!("（没有历史）");
                } else {
                    let rows: Vec<Vec<String>> = history
                        .iter()
                        .map(|h| {
                            vec![
                                h.created_at.clone(),
                                h.connection_name.clone().unwrap_or_else(|| "-".into()),
                                h.status.as_str().to_string(),
                                h.row_count.to_string(),
                                format!("{} ms", h.duration_ms),
                                one_line(&h.sql, 48),
                            ]
                        })
                        .collect();
                    print_table(&["时间", "连接", "状态", "行数", "耗时", "SQL"], rows);
                }
            }
        }

        Command::Types => {
            let types = engine.types();
            if json {
                print_json(&types)?;
            } else {
                let rows: Vec<Vec<String>> = types
                    .iter()
                    .map(|t| {
                        vec![
                            t.key.to_string(),
                            t.label.to_string(),
                            t.runtime_mode.as_str().to_string(),
                            t.default_port
                                .map(|p| p.to_string())
                                .unwrap_or_else(|| "-".into()),
                            t.support_level.to_string(),
                            if t.implemented {
                                "已接入".into()
                            } else {
                                "未接入".into()
                            },
                        ]
                    })
                    .collect();
                print_table(&["key", "名称", "运行时", "默认端口", "支持级别", "状态"], rows);
                let implemented = types.iter().filter(|t| t.implemented).count();
                println!("\n共 {} 种类型，已接入 {} 种", types.len(), implemented);
            }
        }

        Command::Status => {
            let summary = engine.runtime_summary();
            if json {
                print_json(&summary)?;
            } else {
                println!("元数据库      : {}", summary.store_path);
                println!("连接数        : {}", summary.connection_count);
                println!(
                    "驱动实现进度  : {}/{}",
                    summary.implemented_types, summary.declared_types
                );
                println!(
                    "生产保护      : {}",
                    if summary.protect_production {
                        "开启"
                    } else {
                        "关闭"
                    }
                );
                println!(
                    "AI/MCP 可写   : {}",
                    if summary.ai_write_enabled {
                        "是"
                    } else {
                        "否（默认只读）"
                    }
                );
            }
        }

        Command::Driver { command } => match command {
            DriverCommand::List => {
                let report = engine.driver_report();
                let rows: Vec<Vec<String>> = report
                    .entries
                    .iter()
                    .map(|entry| {
                        vec![
                            entry.key.clone(),
                            entry.agent_key.clone().unwrap_or_else(|| "-".to_string()),
                            entry.host.clone().unwrap_or_else(|| "（无宿主）".to_string()),
                            entry
                                .artifact
                                .clone()
                                .unwrap_or_else(|| "（非 JDBC，宿主自带驱动）".to_string()),
                            if entry.host.is_none() {
                                "待接入".to_string()
                            } else if !entry.jdbc {
                                "宿主自带".to_string()
                            } else if entry.installed {
                                format!("已安装（{} jar）", entry.jar_count)
                            } else {
                                "未安装".to_string()
                            },
                        ]
                    })
                    .collect();
                if json {
                    print_json(&report)?;
                } else {
                    print_table(&["类型", "agentKey", "宿主", "Maven 坐标", "驱动"], rows);
                    println!();
                    // 宿主是复数：JDBC 宿主是通用的，MongoDB 等各有专属宿主
                    for agent in &report.agents {
                        if agent.ready {
                            println!(
                                "宿主 {}：就绪（{}）",
                                agent.id,
                                agent.java.clone().unwrap_or_default()
                            );
                        } else {
                            println!(
                                "宿主 {}：未就绪 —— {}",
                                agent.id,
                                agent.reason.clone().unwrap_or_default()
                            );
                        }
                    }
                    println!(
                        "已装驱动：{}/{} 个 agentKey；驱动目录 {}",
                        report.installed_count,
                        report.entries.len(),
                        dbmind_core::driver_dir("<agentKey>").display()
                    );
                }
            }
            DriverCommand::Fetch { agent_key } => {
                let artifact = engine
                    .types()
                    .iter()
                    .filter(|t| t.agent_key == Some(agent_key.as_str()))
                    .filter_map(|t| t.kind.jdbc_artifact())
                    .next()
                    .ok_or_else(|| {
                        DbMindError::new(
                            ErrorCode::DriverNotReady,
                            format!("没有 JDBC 类型使用 agentKey={agent_key}"),
                        )
                    })?;
                let target = fetch_driver(&agent_key, artifact)?;
                if json {
                    print_json(&serde_json::json!({ "agentKey": agent_key, "jar": target }))?;
                } else {
                    println!("已安装驱动 -> {target}");
                }
            }
        },
        Command::Setting { command } => match command {
            SettingCommand::List => {
                let settings = engine.settings()?;
                if json {
                    print_json(&settings)?;
                } else {
                    let rows: Vec<Vec<String>> =
                        settings.iter().map(|(k, v)| vec![k.clone(), v.clone()]).collect();
                    print_table(&["键", "值"], rows);
                }
            }
            SettingCommand::Get { key } => {
                let value = engine.get_setting(&key)?;
                if json {
                    print_json(&serde_json::json!({ "key": key, "value": value }))?;
                } else {
                    println!("{}", value.unwrap_or_else(|| "(未设置)".into()));
                }
            }
            SettingCommand::Set { key, value } => {
                engine.set_setting(&key, &value)?;
                println!("已设置 {key} = {value}");
            }
        },
    }

    Ok(())
}

/// 从 Maven Central 下载驱动 jar。
///
/// 网络能力只放在 CLI：内核保持无网络依赖，桌面/Web 壳将来要装驱动时
/// 复用同一个坐标（在 YAML 里）而不是各自发明下载逻辑。
fn fetch_driver(agent_key: &str, artifact: &str) -> dbmind_core::Result<String> {
    // 地址与文件名的拼法在核心里（`driver_artifact_url` / `driver_jar_name`）：
    // Web 壳也要下载驱动，两边各拼一遍早晚会漂。
    let url = dbmind_core::driver_artifact_url(artifact)?;
    let dir = dbmind_core::driver_dir(agent_key);
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(dbmind_core::driver_jar_name(artifact)?);

    println!("下载 {url}");
    let response = ureq::get(&url).call().map_err(|err| {
        DbMindError::new(ErrorCode::DriverNotReady, format!("下载失败：{err}")).with_detail(url.clone())
    })?;
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(&target)?;
    let bytes = std::io::copy(&mut reader, &mut file)?;
    drop(file);
    println!("已写入 {}（{} KB）", target.display(), bytes / 1024);
    Ok(target.display().to_string())
}

fn target_of(record: &dbmind_core::ConnectionRecord) -> String {
    if record.kind().local_file() {
        record.config.resolved_file().unwrap_or("-").to_string()
    } else {
        match (record.config.host.as_deref(), record.config.resolved_port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_string(),
            _ => "-".to_string(),
        }
    }
}

/// 打印结构缓存状态。
///
/// 缓存意味着**可能不是实时结构**，把这点藏起来就是骗人 —— 用户会以为
/// 「没看到那张新表」是权限或连接问题，而不是「等几秒再试」。
fn print_schema_cache(engine: &DbMindEngine, conn: &str) {
    let Ok(info) = engine.schema_cache_info(conn) else {
        return;
    };
    if info.entries == 0 {
        return;
    }
    let age = info
        .age_secs
        .map(|seconds| format!("{seconds} 秒前"))
        .unwrap_or_else(|| "时间未知".to_string());
    println!(
        "（结构来自缓存：{age}，{} 项；--refresh 可立即回源）",
        info.entries
    );
}

fn render_result(result: &dbmind_core::QueryResult) {
    if result.columns.is_empty() {
        // 读语句即使没有列（空结果集）也要表述成「行」：
        // 用「影响 N 行」会让人以为刚才执行的是写操作（Mongo 的空查询就会这样）
        if result.statement_kind == dbmind_core::StatementKind::Read {
            println!("0 行（耗时 {} ms）", result.duration_ms);
        } else {
            println!(
                "{}（影响 {} 行，耗时 {} ms）",
                result.statement_kind.as_str(),
                result.affected_rows.unwrap_or(0),
                result.duration_ms
            );
        }
        return;
    }

    let headers: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
    let rows: Vec<Vec<String>> = result
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.to_display()).collect())
        .collect();
    print_table(&headers, rows);

    println!(
        "\n{} 行（耗时 {} ms{}{}）",
        result.row_count,
        result.duration_ms,
        if result.truncated { "，已截断" } else { "" },
        result
            .affected_rows
            .map(|n| format!("，影响 {n} 行"))
            .unwrap_or_default()
    );
    for notice in &result.notices {
        println!("· {notice}");
    }
}

fn one_line(sql: &str, max: usize) -> String {
    let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn pad(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - len))
    }
}

/// 以等宽表格打印。表头做泛型是为了同时接受字面量与 `Vec<String>`（结果集列名）。
fn print_table<S: AsRef<str>>(headers: &[S], rows: Vec<Vec<String>>) {
    let headers: Vec<String> = headers.iter().map(|h| h.as_ref().to_string()).collect();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(one_line(cell, 80).chars().count());
            }
        }
    }
    let header_line = headers
        .iter()
        .enumerate()
        .map(|(i, h)| pad(h, widths[i]))
        .collect::<Vec<_>>()
        .join("  ");
    println!("{header_line}");
    println!(
        "{}",
        widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for row in rows {
        let line = row
            .iter()
            .enumerate()
            .map(|(i, c)| pad(&one_line(c, 80), widths.get(i).copied().unwrap_or(0)))
            .collect::<Vec<_>>()
            .join("  ");
        println!("{line}");
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> dbmind_core::Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    println!("{text}");
    Ok(())
}

// 未使用但保留：壳层若需要展示类型清单里的默认端口，可用它做提示
#[allow(dead_code)]
fn default_port_hint(kind: ConnectionKind) -> Option<u16> {
    kind.default_port()
}
