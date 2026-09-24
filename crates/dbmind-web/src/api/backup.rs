//! `/api/backup/*` —— 备份 / 还原（21 个端点）。
//!
//! ## 两条执行路线
//!
//!上游的备份/还原刻意做了「**优先本机命令行工具，装不上就回退内置引擎**」：
//!
//! | 引擎 | 做法 | 好处 / 代价 |
//! |---|---|---|
//! | `cli` | 调 `mysqldump` / `pg_dump` / `sqlcmd` / `sqlite3` | 与原生工具结果一致（含二进制备份），但要求本机装了客户端 |
//! | `builtin` | 内核读元数据 + 读数据，自己拼 SQL 转储 | 零依赖、任何平台都能用，但只覆盖表/视图，且是文本导出 |
//! | `auto` | 有工具走 `cli`，没有就问你一句，再没有就用 `builtin` | 界面上那三选（自动安装 / 我已装好 / 跳过）就是为它准备的 |
//!
//! 所以这一层真正的难点不是「跑命令」，而是**任务中途要能停下来等人做决定**：
//! 没有这个等待机制，界面上那三个按钮就只是三个没人等的按钮
//! （见 `tasks::Task::await_install_decision`）。
//!
//! ## 三个决定
//!
//! 1. **口令绝不进命令行参数**（走 `MYSQL_PWD` / `PGPASSWORD` 环境变量）。
//!    命令行参数在多数系统上对所有本机进程可见（`ps`、任务管理器），
//!    把口令写进去等于把它贴在了公告板上。
//! 2. **等待有超时**，超时按「跳过」处理：没人理的弹窗不该把任务永久挂住。
//! 3. **内置引擎的边界要写清楚**：只导出表（结构 + 数据）与视图，
//!    存储过程/函数/触发器/事件不在其中 —— 这一点会出现在任务的日志与结果说明里，
//!    绝不能让用户以为「备份完整」。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path as FsPath, PathBuf};
use std::process::Command;

use axum::extract::{Multipart, Path, RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use dbmind_core::{CellValue, ConnectionKind, TableKind};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::export::{ddl_of, sql_literal};
use crate::api::meta::run_sql_in;
use crate::api::tasks::{self, Artifact, Task};
use crate::api::{blocking, require_record, Params};
use crate::AppState;

/// 等用户做决定最多多久（超时按「跳过」处理）。
const INSTALL_WAIT: std::time::Duration = std::time::Duration::from_secs(300);
/// 命令行工具目录的环境变量（允许用户把工具装到非 PATH 目录里）。
const TOOL_DIR_ENVS: &[&str] = &["DBMIND_TOOL_DIR"];

// ------------------------------------------------------------------ 命令行引擎表

#[derive(Clone, Copy)]
struct CliSpec {
    /// 展示用引擎名
    engine: &'static str,
    /// 备份工具
    tool: &'static str,
    /// 备份工具的其它叫法（MariaDB 与 MySQL 就是一对）
    aliases: &'static [&'static str],
    /// 还原工具
    restore_tool: &'static str,
    /// 包名（brew / apt 用）
    package: &'static str,
    dump_hint: &'static str,
    restore_hint: &'static str,
}

fn cli_spec(kind: ConnectionKind) -> Option<CliSpec> {
    Some(match kind.key() {
        "mysql" | "mariadb" | "doris" => CliSpec {
            engine: "MySQL",
            tool: "mysqldump",
            aliases: &["mariadb-dump"],
            restore_tool: "mysql",
            package: "mysql-client",
            dump_hint: "mysqldump -h {host} -P {port} -u {user} --default-character-set={charset} \
                        --databases {database} > \"{filePath}\"",
            restore_hint: "mysql -h {host} -P {port} -u {user} --default-character-set={charset} \
                           {database} < \"{filePath}\"",
        },
        "postgresql" => CliSpec {
            engine: "PostgreSQL",
            tool: "pg_dump",
            aliases: &[],
            restore_tool: "psql",
            package: "postgresql-client",
            dump_hint: "pg_dump -h {host} -p {port} -U {user} -d {database} -f \"{filePath}\"",
            restore_hint: "psql -h {host} -p {port} -U {user} -d {database} -f \"{filePath}\"",
        },
        "kingbase" => CliSpec {
            engine: "KingbaseES",
            tool: "sys_dump",
            aliases: &[],
            restore_tool: "ksql",
            package: "kingbase-client",
            dump_hint: "sys_dump -h {host} -p {port} -U {user} -f \"{filePath}\" {database}",
            restore_hint: "ksql -h {host} -p {port} -U {user} -d {database} -f \"{filePath}\"",
        },
        "oracle" => CliSpec {
            engine: "Oracle",
            tool: "expdp",
            aliases: &[],
            restore_tool: "impdp",
            package: "oracle-instantclient",
            dump_hint: "expdp {user}/******@{host}:{port}/{database} \
                        DIRECTORY=DATA_PUMP_DIR DUMPFILE={filePath} FULL=Y",
            restore_hint: "impdp {user}/******@{host}:{port}/{database} \
                           DIRECTORY=DATA_PUMP_DIR DUMPFILE={filePath} FULL=Y",
        },
        "dm" => CliSpec {
            engine: "达梦 DM",
            tool: "dexp",
            aliases: &[],
            restore_tool: "dimp",
            package: "dm-client",
            dump_hint: "dexp USERID={user}/******@{host}:{port} FILE=\"{filePath}\" DIRECTORY=. FULL=Y",
            restore_hint: "dimp USERID={user}/******@{host}:{port} FILE=\"{filePath}\" DIRECTORY=. FULL=Y",
        },
        "sqlserver" => CliSpec {
            engine: "SQL Server",
            tool: "sqlcmd",
            aliases: &[],
            restore_tool: "sqlcmd",
            package: "mssql-tools18",
            dump_hint: "sqlcmd -S {host},{port} -U {user} \
                        -Q \"BACKUP DATABASE [{database}] TO DISK = N'{filePath}' WITH FORMAT\"",
            restore_hint: "sqlcmd -S {host},{port} -U {user} \
                           -Q \"RESTORE DATABASE [{database}] FROM DISK = N'{filePath}' WITH REPLACE\"",
        },
        "sqlite" => CliSpec {
            engine: "SQLite",
            tool: "sqlite3",
            aliases: &[],
            restore_tool: "sqlite3",
            package: "sqlite",
            dump_hint: "sqlite3 \"{sqliteFile}\" .dump > \"{filePath}\"",
            restore_hint: "sqlite3 \"{sqliteFile}\" < \"{filePath}\"",
        },
        // H2 / Derby / DB2 / ClickHouse 没有「可直接调用的本机客户端」共识，
        // 内置引擎就是它们的正路（这也是上游的做法）
        _ => return None,
    })
}

/// 在 PATH（以及 `DBMIND_TOOL_DIR`）里找一个可执行文件。
fn find_tool(names: &[&str]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for key in TOOL_DIR_ENVS {
        if let Some(value) = std::env::var_os(key) {
            dirs.extend(std::env::split_paths(&value));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    // Windows 的可执行文件要带后缀才认（PATHEXT 就是干这个的）
    let suffixes: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .map(|item| item.trim().to_ascii_lowercase())
            .filter(|item| !item.is_empty())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in dirs {
        for name in names {
            for suffix in &suffixes {
                let candidate = dir.join(format!("{name}{suffix}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// 本机能不能自动安装（以及用什么命令装）。
///
/// Windows 一律返回 None：没有「一处命令装遍所有包管理器」的共识，
/// 与其猜一个（winget? choco? scoop?）不如老实告诉用户手动装。
fn install_command(spec: CliSpec, restore: bool) -> Option<String> {
    let tool = if restore { spec.restore_tool } else { spec.tool };
    if cfg!(windows) {
        return None;
    }
    if cfg!(target_os = "macos") {
        if find_tool(&["brew"]).is_some() {
            return Some(format!("brew install {}", spec.package));
        }
        return None;
    }
    for (manager, flag) in [("apt-get", "install -y"), ("dnf", "install -y"), ("yum", "install -y")] {
        if find_tool(&[manager]).is_some() {
            return Some(format!("sudo -n {manager} {flag} {}", spec.package));
        }
    }
    let _ = tool;
    None
}

/// 「没有可自动安装」时给出的可操作说明。
fn install_guidance(spec: CliSpec, tool: &str) -> String {
    if cfg!(windows) {
        format!(
            "当前系统为 Windows，命令行工具的安装方式因来源而异，无法替你决定，故不自动执行安装命令。\n\
             请手动安装 {tool}（例如 `winget install {package}`，或从官网下载客户端），\n\
             装好后点「我已手动装好，重新检测」；也可以点「跳过」用内置引擎完成本次操作。",
            package = spec.package
        )
    } else if cfg!(target_os = "macos") {
        format!(
            "本机为 macOS，但未检测到 Homebrew，无法自动安装 {tool}。\n\
             请先安装 Homebrew，或手动安装 {tool} 后点「我已手动装好，重新检测」；\
             也可以点「跳过」用内置引擎完成本次操作。"
        )
    } else {
        format!(
            "本机为 Linux 但未检测到 apt/dnf/yum 包管理器，无法自动安装 {tool}。\n\
             请手动安装 {tool} 后点「我已手动装好，重新检测」；\
             也可以点「跳过」用内置引擎完成本次操作。"
        )
    }
}

// ------------------------------------------------------------------ 请求

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackupRequest {
    #[serde(default)]
    connection_id: String,
    #[serde(default)]
    database: Option<String>,
    #[serde(default)]
    target_dir: Option<String>,
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    cli: Option<CliTarget>,
    #[serde(default)]
    tables: Vec<String>,
    #[serde(default)]
    views: Vec<String>,
    #[serde(default)]
    functions: Vec<String>,
    #[serde(default)]
    procedures: Vec<String>,
    #[serde(default)]
    triggers: Vec<String>,
    #[serde(default)]
    events: Vec<String>,
    #[serde(default)]
    options: Option<BackupOptions>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct CliTarget {
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct BackupOptions {
    #[serde(default = "yes")]
    include_drop: bool,
    #[serde(default = "yes")]
    include_data: bool,
    #[serde(default = "yes")]
    include_views: bool,
    #[serde(default)]
    include_procedures: bool,
    #[serde(default)]
    include_triggers: bool,
    #[serde(default)]
    include_events: bool,
    #[serde(default)]
    batch_size: Option<u64>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRequest {
    #[serde(default)]
    connection_id: String,
    #[serde(default)]
    database: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    cli: Option<CliTarget>,
}

// ------------------------------------------------------------------ 任务视图

/// 任务快照 →上游的备份/还原视图。
fn task_view(task: &Task, restore: bool) -> Value {
    let done = task.done();
    let total = task.snapshot()["total"].as_i64().unwrap_or(0);
    let progress = if total > 0 {
        ((done as f64 / total as f64) * 100.0).round().min(100.0) as i64
    } else {
        0
    };
    let snapshot = task.snapshot();
    let status = snapshot["status"].clone();
    let message = snapshot["message"].clone();
    let mut view = Map::new();
    view.insert("taskId".to_string(), snapshot["taskId"].clone());
    view.insert("status".to_string(), status.clone());
    view.insert("progress".to_string(), json!(progress));
    view.insert("message".to_string(), message.clone());
    view.insert(
        "error".to_string(),
        if status == json!("error") { message } else { Value::Null },
    );
    let artifact = task.artifact();
    view.insert(
        "fileName".to_string(),
        artifact
            .as_ref()
            .map(|artifact| json!(artifact.filename))
            .unwrap_or(Value::Null),
    );
    if let Some(result) = task.result() {
        view.insert(
            "resultDetail".to_string(),
            result.get("detail").cloned().unwrap_or(Value::Null),
        );
        view.insert("result".to_string(), result);
    }
    view.insert(
        "install".to_string(),
        task.install_prompt().unwrap_or(Value::Null),
    );
    if restore {
        view.insert("mode".to_string(), json!("restore"));
    }
    Value::Object(view)
}

// ------------------------------------------------------------------ 文件系统类端点

/// `GET /api/backup/default-dir` —— 默认备份目录。
pub async fn default_dir() -> XResult<Json<Value>> {
    let dir = default_backup_dir()?;
    Ok(Json(json!({ "dir": dir.display().to_string() })))
}

fn default_backup_dir() -> XResult<PathBuf> {
    let dir = dbmind_core::paths::home_dir().join("backups");
    dbmind_core::paths::ensure_dir(&dir)?;
    Ok(dir)
}

/// `POST /api/backup/check-dir` —— 目录可用性校验。
pub async fn check_dir(Json(body): Json<Value>) -> XResult<Json<Value>> {
    let raw = body
        .get("dir")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if raw.is_empty() {
        return Ok(Json(json!({
            "dir": raw,
            "exists": false,
            "writable": false,
            "message": "请填写备份目录",
        })));
    }
    let dir = PathBuf::from(&raw);
    let exists = dir.is_dir();
    // 「能不能写」要真的写一下才知道：只判断目录存在，会在只读目录/无权限目录上放行
    let writable = if exists {
        let probe = dir.join(format!(".dbmind-write-test-{}", tasks::stamp()));
        std::fs::write(&probe, b"ok").is_ok() && std::fs::remove_file(&probe).is_ok()
    } else {
        dbmind_core::paths::ensure_dir(&dir).is_ok()
    };
    let message = if !exists {
        if writable {
            "目录不存在，将自动创建".to_string()
        } else {
            "目录不存在，且无法创建（父目录不可写？）".to_string()
        }
    } else if writable {
        "目录可用".to_string()
    } else {
        "目录存在但没有写权限".to_string()
    };
    Ok(Json(json!({
        "dir": raw,
        "exists": exists,
        "writable": writable,
        "message": message,
    })))
}

/// `GET /api/backup/browse-dirs` / `browse-files` —— 目录浏览。
///
/// 返回形状由界面定：`path` / `parent` / `dirs[]` / `files[]` / `exists` / `home`。
/// `path` 为空 = 磁盘根列表（Windows 的「此电脑」），此时 `parent` 也为空 ——
/// 界面据此显示「.. 返回上级」是否可用。
pub async fn browse_dirs(RawQuery(raw): RawQuery) -> XResult<Json<Value>> {
    browse(raw, false).await
}

pub async fn browse_files(RawQuery(raw): RawQuery) -> XResult<Json<Value>> {
    browse(raw, true).await
}

async fn browse(raw: Option<String>, file_mode: bool) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let path = params.get("path").unwrap_or_default();
    let home = std::env::var("USERPROFILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var("HOME").ok().filter(|value| !value.trim().is_empty()))
        .unwrap_or_default();

    if path.trim().is_empty() {
        // 磁盘根列表（Windows 的「此电脑」；类 Unix 就是 /）
        let dirs = if cfg!(windows) {
            (b'A'..=b'Z')
                .map(|letter| format!("{}:\\", letter as char))
                .filter(|root| PathBuf::from(root).is_dir())
                .collect::<Vec<_>>()
        } else {
            vec!["/".to_string()]
        };
        return Ok(Json(json!({
            "path": "",
            "parent": Value::Null,
            "dirs": dirs,
            "files": [],
            "exists": true,
            "home": home,
        })));
    }

    let dir = PathBuf::from(path.trim());
    if !dir.is_dir() {
        return Ok(Json(json!({
            "path": path,
            "parent": Value::Null,
            "dirs": [],
            "files": [],
            "exists": false,
            "home": home,
        })));
    }
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // 只跳过字面上的 "." 与 ".." —— 它们本来也不会被 read_dir 返回。
            // 这里**不能**按"点开头"过滤：那会连真实存在的目录一起吞掉，
            // 例如 `.dbmind`（本项目的数据目录，用户选库文件时正需要它）、
            // `.ssh`、`.config`、`.vscode` …（实测就是它们全都列不出来）。
            if name == "." || name == ".." {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => dirs.push(name),
                Ok(_) => {
                    if file_mode {
                        files.push(name);
                    }
                }
                Err(_) => {}
            }
        }
    }
    dirs.sort_by_key(|name| name.to_ascii_lowercase());
    files.sort_by_key(|name| name.to_ascii_lowercase());
    files.truncate(2000);

    // parent：盘符根（D:\）没有上级 —— 界面此时把「返回上级」当成「回到磁盘列表」
    let parent = dir.parent().map(|parent| {
        if parent.as_os_str().is_empty() {
            String::new()
        } else {
            parent.display().to_string()
        }
    });

    Ok(Json(json!({
        "path": dir.display().to_string(),
        "parent": parent,
        "dirs": dirs,
        "files": files,
        "exists": true,
        "home": home,
    })))
}

// ------------------------------------------------------------------ CLI 能力

/// `GET /api/backup/cli-guide` —— 命令行能力与命令参考。
pub async fn cli_guide(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let conn = Params::parse(raw.as_deref())
        .get("connectionId")
        .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))?;
    let record = require_record(&state, &conn).await?;
    let kind = record.kind();
    let type_code = kind.key().to_ascii_uppercase();
    let base = json!({
        "connectionType": type_code,
        "connectionLabel": kind.key(),
        "connectionId": record.id,
        "host": record.config.host.clone().unwrap_or_default(),
        "port": record.config.port,
        "username": record.config.username.clone().unwrap_or_default(),
        "sqliteFile": record.config.file_path.clone().unwrap_or_default(),
        "database": record.config.database.clone().unwrap_or_default(),
        // 本项目不做 SSH 隧道（内核的连接模型里没有这一层），如实给 false
        "isSsh": false,
    });
    let Some(spec) = cli_spec(kind) else {
        let mut view = base;
        view["cliSupported"] = json!(false);
        view["toolDisplay"] = Value::Null;
        view["dumpHint"] = Value::Null;
        view["restoreHint"] = Value::Null;
        view["note"] = json!(format!(
            "当前类型（{type_code}）没有可直接调用的本机命令行引擎，请使用「内置引擎」或「自动」。"
        ));
        return Ok(Json(view));
    };
    let mut view = base;
    view["cliSupported"] = json!(true);
    view["toolDisplay"] = json!(spec.tool);
    view["engineDisplay"] = json!(spec.engine);
    view["dumpHint"] = json!(spec.dump_hint);
    view["restoreHint"] = json!(spec.restore_hint);
    view["note"] = json!(format!(
        "将调用本机 {} 执行，复用已保存的连接信息；口令通过环境变量注入，不出现在命令里。",
        spec.tool
    ));
    Ok(Json(view))
}

/// `GET /api/backup/cli-tool-status` —— 工具是否可用（向导第 2 步的预检）。
pub async fn cli_tool_status(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let conn = params
        .get("connectionId")
        .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))?;
    let restore = params.get("mode").as_deref() == Some("restore");
    let record = require_record(&state, &conn).await?;
    let kind = record.kind();
    let Some(spec) = cli_spec(kind) else {
        return Ok(Json(json!({
            "cliSupported": false,
            "connectionLabel": kind.key(),
            "engineDisplay": Value::Null,
            "tool": Value::Null,
            "toolNames": [],
            "present": false,
            "canAutoInstall": false,
            "installCommand": Value::Null,
            "message": format!(
                "当前类型（{}）没有可直接调用的本机命令行引擎，请使用「内置引擎」或「自动」。",
                kind.key().to_ascii_uppercase()
            ),
        })));
    };
    let tool = if restore { spec.restore_tool } else { spec.tool };
    let mut names: Vec<&str> = vec![tool];
    if !restore {
        names.extend_from_slice(spec.aliases);
    }
    let found = find_tool(&names);
    let command = install_command(spec, restore);
    let message = if found.is_some() {
        format!("已检测到 {tool}")
    } else {
        let mut text = format!(
            "当前「{}」操作优先使用命令行工具 {tool}（{} 官方客户端），以获得与原生工具一致的结果，\
             但本机未检测到该工具。",
            if restore { "还原" } else { "备份" },
            spec.engine
        );
        text.push('\n');
        text.push_str(&install_guidance(spec, tool));
        text
    };
    Ok(Json(json!({
        "cliSupported": true,
        "connectionLabel": kind.key(),
        "engineDisplay": spec.engine,
        "tool": tool,
        "toolNames": names,
        "present": found.is_some(),
        "toolPath": found.map(|path| path.display().to_string()),
        "canAutoInstall": command.is_some(),
        "installCommand": command,
        "message": message,
    })))
}

/// `POST /api/backup/cli-tool-install` —— 自动安装（尽力而为）。
pub async fn cli_tool_install(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let conn = params
        .get("connectionId")
        .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))?;
    let restore = params.get("mode").as_deref() == Some("restore");
    let record = require_record(&state, &conn).await?;
    let kind = record.kind();
    let Some(spec) = cli_spec(kind) else {
        return Err(XError::bad_request(format!(
            "当前类型（{}）没有命令行引擎，无需安装",
            kind.key().to_ascii_uppercase()
        )));
    };
    let tool = if restore { spec.restore_tool } else { spec.tool };
    let Some(command) = install_command(spec, restore) else {
        return Ok(Json(json!({
            "success": false,
            "present": false,
            "tool": tool,
            "detail": install_guidance(spec, tool),
        })));
    };
    let output = run_shell(&command)?;
    let names: Vec<&str> = vec![tool];
    let present = find_tool(&names).is_some();
    Ok(Json(json!({
        "success": present || output.0 == 0,
        "present": present,
        "tool": tool,
        "installCommand": command,
        "detail": format!("执行 `{command}`（退出码 {}）\n{}", output.0, output.1),
    })))
}

/// 跑一条 shell 命令，返回 (退出码, 输出)。
fn run_shell(command: &str) -> XResult<(i32, String)> {
    if cfg!(windows) {
        return Err(XError::bad_request(
            "Windows 上不自动执行安装命令：安装方式因来源而异，请手动安装后重试",
        ));
    }
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|err| XError::internal(format!("执行安装命令失败：{err}")))?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.code().unwrap_or(-1), text.trim().to_string()))
}

// ------------------------------------------------------------------ 备份

/// `POST /api/backup/start` —— 备份（异步任务）。
pub async fn start(
    State(state): State<AppState>,
    Json(body): Json<BackupRequest>,
) -> XResult<Json<Value>> {
    if body.connection_id.trim().is_empty() {
        return Err(XError::bad_request("缺少 connectionId 参数"));
    }
    let record = require_record(&state, &body.connection_id).await?;
    let kind = record.kind();
    let database = body
        .database
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| record.config.database.clone())
        .unwrap_or_default();
    if database.trim().is_empty() && kind.key() != "sqlite" {
        return Err(XError::bad_request(
            "缺少 database 参数：备份需要明确备份哪个库",
        ));
    }
    let target_dir = match body.target_dir.clone().filter(|value| !value.trim().is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => default_backup_dir()?,
    };
    dbmind_core::paths::ensure_dir(&target_dir)?;

    let engine = body.engine.clone().unwrap_or_else(|| "auto".to_string());
    let options = body.options.clone().unwrap_or_default();
    let spec = cli_spec(kind);
    // 命令行连接参数覆盖（界面允许为命令行单独填一套连接信息）
    let cli_override = body.cli.clone();

    // 命令行是否真的能用（决定 auto 是走 cli 还是问用户/回退内置）
    let cli_available = spec
        .map(|spec| find_tool(&[spec.tool]).is_some())
        .unwrap_or(false);
    let use_cli = match engine.as_str() {
        "cli" => true,
        "builtin" => false,
        // auto：有工具就走命令行（更接近原生），没有则由任务去问用户
        _ => cli_available,
    };

    let registry = state.tasks.clone();
    let task = registry.spawn("backup", "backup_", move |task| {
        let state = state.clone();
        async move {
            task.set_total(100);
            let file_name = format!(
                "{}_{}.sql",
                if database.trim().is_empty() {
                    "sqlite".to_string()
                } else {
                    safe_name(&database)
                },
                tasks::stamp()
            );
            let path = target_dir.join(&file_name);
            let label = record.config.name.clone();

            if use_cli {
                if let Some(spec) = spec {
                    match run_cli_backup(
                                    &state,
                                    task.clone(),
                                    &record.id,
                                    &spec,
                                    &path,
                                    &database,
                                    cli_override.as_ref(),
                                )
                                .await {
                        Ok(log) => {
                            task.step(log);
                            task.set_done(100);
                            let detail = format!("命令行备份完成（{}）", spec.tool);
                            task.set_message(format!("备份完成：{}", path.display()));
                            task.set_result(json!({ "detail": detail }));
                            task.set_artifact(Artifact {
                                path,
                                media_type: "text/plain; charset=UTF-8".to_string(),
                                filename: file_name,
                            });
                            return Ok(Some(json!({ "detail": detail })));
                        }
                        Err(err) => {
                            task.log(format!("命令行备份失败：{}", err.message));
                            // 允许回退（auto）时继续走内置；明确选了 cli 就直接失败
                            if engine != "auto" {
                                return Err(err.message);
                            }
                            task.step("命令行不可用，改用内置引擎");
                        }
                    }
                }
            } else if spec.is_some() && engine != "builtin" {
                // 类型支持命令行但本机没有工具 ⇒ 问用户（界面上的三选）
                if let Some(spec) = spec {
                    let command = install_command(spec, false);
                    let prompt = json!({
                        "engineDisplay": spec.engine,
                        "tool": spec.tool,
                        "canAutoInstall": command.is_some(),
                        "installCommand": command,
                        "message": install_guidance(spec, spec.tool),
                    });
                    task.set_install_prompt(prompt);
                    task.step(format!("等待确认：本机未检测到 {}", spec.tool));
                    let decision = task.await_install_decision(INSTALL_WAIT).await;
                    task.clear_install_prompt();
                    match decision.as_str() {
                        "confirm" => {
                            if let Some(command) = command {
                                match run_shell(&command) {
                                    Ok((code, text)) => {
                                        task.log(format!("执行 `{command}`（退出码 {code}）"));
                                        if !text.is_empty() {
                                            task.log(text);
                                        }
                                    }
                                    Err(err) => task.log(format!("安装失败：{}", err.message)),
                                }
                            }
                            if find_tool(&[spec.tool]).is_some() {
                                task.step("工具已就绪，改用命令行");
                                match run_cli_backup(
                                    &state,
                                    task.clone(),
                                    &record.id,
                                    &spec,
                                    &path,
                                    &database,
                                    cli_override.as_ref(),
                                )
                                .await {
                                    Ok(log) => {
                                        task.step(log);
                                        task.set_done(100);
                                        let detail = format!("命令行备份完成（{}）", spec.tool);
                                        task.set_message(format!("备份完成：{}", path.display()));
                                        task.set_result(json!({ "detail": detail }));
                                        task.set_artifact(Artifact {
                                            path,
                                            media_type: "text/plain; charset=UTF-8".to_string(),
                                            filename: file_name,
                                        });
                                        return Ok(Some(json!({ "detail": detail })));
                                    }
                                    Err(err) => {
                                        task.log(format!("命令行备份失败：{}，回退内置引擎", err.message));
                                    }
                                }
                            } else {
                                task.log("安装后仍未检测到工具，回退内置引擎");
                            }
                        }
                        "manual" => {
                            if find_tool(&[spec.tool]).is_some() {
                                task.step("已检测到工具，改用命令行");
                                match run_cli_backup(
                                    &state,
                                    task.clone(),
                                    &record.id,
                                    &spec,
                                    &path,
                                    &database,
                                    cli_override.as_ref(),
                                )
                                .await {
                                    Ok(log) => {
                                        task.step(log);
                                        task.set_done(100);
                                        let detail = format!("命令行备份完成（{}）", spec.tool);
                                        task.set_message(format!("备份完成：{}", path.display()));
                                        task.set_result(json!({ "detail": detail }));
                                        task.set_artifact(Artifact {
                                            path,
                                            media_type: "text/plain; charset=UTF-8".to_string(),
                                            filename: file_name,
                                        });
                                        return Ok(Some(json!({ "detail": detail })));
                                    }
                                    Err(err) => task.log(format!("命令行备份失败：{}，回退内置引擎", err.message)),
                                }
                            } else {
                                task.log("仍未检测到工具，回退内置引擎");
                            }
                        }
                        "cancel" => return Err("任务已取消".to_string()),
                        // 跳过 / 超时
                        _ => task.step("改用内置引擎"),
                    }
                }
            }

            // 内置引擎
            let summary = builtin_backup(
                &state,
                task.clone(),
                &record.id,
                &database,
                &body,
                &options,
                &path,
                &file_name,
            )
            .await
            .map_err(|err| err.message)?;
            let detail = format!(
                "内置引擎备份完成：{} 张表 / {} 行（连接 {label}）",
                summary.tables, summary.rows
            );
            for notice in &summary.notices {
                task.log(notice.clone());
            }
            task.set_done(100);
            task.set_message(format!("备份完成：{}", path.display()));
            task.set_result(json!({ "detail": detail, "notices": summary.notices }));
            task.set_artifact(Artifact {
                path,
                media_type: "text/plain; charset=UTF-8".to_string(),
                filename: file_name,
            });
            Ok(Some(json!({ "detail": detail })))
        }
    });

    Ok(Json(task_view(&task, false)))
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    if cleaned.trim().is_empty() {
        "backup".to_string()
    } else {
        cleaned
    }
}

/// 拼一条命令行工具的调用（备份 / 还原共用）。
///
/// 两个必须守住的东西：
/// - **口令走环境变量**，绝不进 argv（argv 对本机所有进程可见）；
/// - **导出类工具的 stdout 必须重定向进文件**。早先的实现把 stdout 当日志收走了 ——
///   结果是「命令成功、日志漂亮、备份文件永远是空的」，这种错最难发现。
fn cli_command(
    kind: ConnectionKind,
    spec: &CliSpec,
    restore: bool,
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    database: &str,
    file_path: &str,
    sqlite_file: &str,
) -> XResult<Command> {
    if kind.key() == "sqlite" {
        // SQLite 的备份/还原就是「整个文件 ↔ SQL 文本」，shell 重定向最直接
        let template = if restore { spec.restore_hint } else { spec.dump_hint };
        let line = template
            .replace("{sqliteFile}", sqlite_file)
            .replace("{filePath}", file_path);
        let mut command = Command::new("sh");
        command.arg("-c").arg(line);
        return Ok(command);
    }
    let tool_name = if restore { spec.restore_tool } else { spec.tool };
    let tool = find_tool(&[tool_name])
        .ok_or_else(|| XError::internal(format!("未找到命令行工具 {tool_name}")))?;
    let mut command = Command::new(tool);
    match kind.key() {
        "mysql" | "mariadb" | "doris" => {
            command
                .arg(format!("-h{host}"))
                .arg(format!("-P{port}"))
                .arg(format!("-u{user}"));
            if !password.is_empty() {
                command.env("MYSQL_PWD", password);
            }
            if restore {
                command.arg(database);
                command.stdin(std::fs::File::open(file_path).map_err(|err| {
                    XError::internal(format!("打开备份文件失败：{err}"))
                })?);
            } else {
                command.arg("--databases").arg(database);
                command.stdout(
                    std::fs::File::create(file_path)
                        .map_err(|err| XError::internal(format!("创建备份文件失败：{err}")))?,
                );
            }
        }
        "postgresql" | "kingbase" => {
            command
                .arg("-h")
                .arg(host)
                .arg("-p")
                .arg(port.to_string())
                .arg("-U")
                .arg(user)
                .arg("-f")
                .arg(file_path)
                .arg(database);
            if !password.is_empty() {
                command.env("PGPASSWORD", password);
            }
        }
        "sqlserver" => {
            let statement = if restore {
                format!("RESTORE DATABASE [{database}] FROM DISK = N'{file_path}' WITH REPLACE")
            } else {
                format!("BACKUP DATABASE [{database}] TO DISK = N'{file_path}' WITH FORMAT")
            };
            command
                .arg("-S")
                .arg(format!("{host},{port}"))
                .arg("-U")
                .arg(user)
                .arg("-Q")
                .arg(statement);
            if !password.is_empty() {
                command.env("SQLCMDPASSWORD", password);
            }
        }
        other => {
            return Err(XError::internal(format!(
                "命令行拼接还不支持 {other}（请改用内置引擎）"
            )))
        }
    }
    Ok(command)
}

/// 用命令行工具备份。
async fn run_cli_backup(
    state: &AppState,
    task: std::sync::Arc<Task>,
    conn: &str,
    spec: &CliSpec,
    path: &FsPath,
    database: &str,
    over: Option<&CliTarget>,
) -> XResult<String> {
    let record = require_record(state, conn).await?;
    let kind = record.kind();
    let host = resolve_cli(over.and_then(|o| o.host.clone()), record.config.host.clone())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = over
        .and_then(|o| o.port)
        .or(record.config.port)
        .unwrap_or(0);
    let user = resolve_cli(over.and_then(|o| o.username.clone()), record.config.username.clone())
        .unwrap_or_default();
    let password = resolve_cli(
        over.and_then(|o| o.password.clone()),
        record.config.password.clone(),
    )
    .unwrap_or_default();
    let file_path = path.display().to_string();
    let sqlite_file = record.config.file_path.clone().unwrap_or_default();

    let mut command = cli_command(
        kind,
        spec,
        false,
        &host,
        port,
        &user,
        &password,
        database,
        &file_path,
        &sqlite_file,
    )?;
    let tool = if kind.key() == "sqlite" { "sqlite3" } else { spec.tool };
    task.step(format!("执行 {tool}（备份到 {}）", path.display()));
    let output = command
        .output()
        .map_err(|err| XError::internal(format!("执行 {tool} 失败：{err}")))?;
    let mut text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    if !output.status.success() {
        return Err(XError::internal(format!(
            "{tool} 退出码 {}：{text}",
            output.status.code().unwrap_or(-1)
        )));
    }
    let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if size == 0 {
        return Err(XError::internal(format!(
            "{tool} 执行成功但备份文件是空的（0 字节），请检查账号权限"
        )));
    }
    Ok(format!("已用 {tool} 备份到 {}（{size} 字节）", path.display()))
}

/// 用命令行工具还原。
async fn run_cli_restore(
    state: &AppState,
    task: std::sync::Arc<Task>,
    conn: &str,
    spec: &CliSpec,
    path: &FsPath,
    database: &str,
    over: Option<&CliTarget>,
) -> XResult<String> {
    let record = require_record(state, conn).await?;
    let kind = record.kind();
    let host = resolve_cli(over.and_then(|o| o.host.clone()), record.config.host.clone())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = over.and_then(|o| o.port).or(record.config.port).unwrap_or(0);
    let user = resolve_cli(over.and_then(|o| o.username.clone()), record.config.username.clone())
        .unwrap_or_default();
    let password = resolve_cli(
        over.and_then(|o| o.password.clone()),
        record.config.password.clone(),
    )
    .unwrap_or_default();
    let file_path = path.display().to_string();
    let sqlite_file = record.config.file_path.clone().unwrap_or_default();
    let mut command = cli_command(
        kind,
        spec,
        true,
        &host,
        port,
        &user,
        &password,
        database,
        &file_path,
        &sqlite_file,
    )?;
    let tool = if kind.key() == "sqlite" { "sqlite3" } else { spec.restore_tool };
    task.step(format!("执行 {tool} 还原 {}", path.display()));
    let output = command
        .output()
        .map_err(|err| XError::internal(format!("执行 {tool} 失败：{err}")))?;
    let mut text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    if !output.status.success() {
        return Err(XError::internal(format!(
            "{tool} 退出码 {}：{text}",
            output.status.code().unwrap_or(-1)
        )));
    }
    Ok(format!("已用 {tool} 还原完成（{}）", path.display()))
}

/// 命令行参数覆盖：请求里给了就用请求的，没给就用连接里存的。
fn resolve_cli(over: Option<String>, stored: Option<String>) -> Option<String> {
    over.filter(|value| !value.trim().is_empty()).or(stored)
}

struct BuiltinSummary {
    tables: u64,
    rows: u64,
    notices: Vec<String>,
}

/// 内置引擎备份：结构（+ 可选 DROP）+ 数据 + 视图。
#[allow(clippy::too_many_arguments)]
async fn builtin_backup(
    state: &AppState,
    task: std::sync::Arc<Task>,
    conn: &str,
    database: &str,
    body: &BackupRequest,
    options: &BackupOptions,
    path: &FsPath,
    file_name: &str,
) -> XResult<BuiltinSummary> {
    let record = require_record(state, conn).await?;
    let dialect = Dialect::new(record.kind());
    let mut notices = Vec::new();
    let batch = options.batch_size.unwrap_or(200).clamp(1, 5000) as usize;

    // 表清单：请求里指定了就用指定的，没指定就按连接/库内全部表
    let tables: Vec<String> = if !body.tables.is_empty() {
        body.tables.clone()
    } else {
        let target = crate::api::scope::resolve(state, conn, database).await?;
        crate::api::driver::ensure_for_connection(state, &target).await?;
        let engine = state.engine.clone();
        let found = blocking(move || engine.list_tables_fresh(&target)).await?;
        found
            .into_iter()
            .filter(|table| table.kind == TableKind::Table)
            .map(|table| table.name)
            .collect()
    };
    if tables.is_empty() {
        return Err(XError::bad_request("这个库里没有表可备份"));
    }
    task.set_total((tables.len() + 1) as i64);
    task.step(format!("共 {} 张表", tables.len()));

    let file = File::create(path).map_err(|e| XError::internal(format!("无法创建备份文件：{e}")))?;
    let mut out = BufWriter::new(file);
    writeln!(
        out,
        "-- DBMind 数据库备份\n-- 连接 {} ｜ 库 {} ｜ 时间 {}\n",
        record.config.name,
        if database.is_empty() { "(连接默认)" } else { database },
        tasks::stamp()
    )
    .map_err(|e| XError::internal(format!("写入备份文件失败：{e}")))?;

    // 视图
    if options.include_views {
        for view in &body.views {
            match dialect.ddl(view).sql() {
                Some(sql) => {
                    let result = run_sql_in(state, conn, database, sql, 5).await?;
                    let mut definition = None;
                    for row in result.rows {
                        for value in row {
                            if let CellValue::Text(text) = value {
                                if text.to_ascii_lowercase().contains("create") {
                                    definition = Some(text);
                                }
                            }
                        }
                    }
                    match definition {
                        Some(definition) => {
                            writeln!(out, "{};", definition.trim().trim_end_matches(';'))
                                .map_err(|e| XError::internal(e.to_string()))?;
                        }
                        None => notices.push(format!("视图 {view} 的定义没取到，未导出")),
                    }
                }
                None => notices.push(format!(
                    "视图 {view} 未导出：{} 拿不到完整视图定义",
                    record.kind().key().to_ascii_uppercase()
                )),
            }
        }
    }
    // 用户勾了、但内置引擎导不出来的对象：把**具体数量**说出来，
    // 而不是笼统一句「未包含某些对象」——用户想知道的是「少了几个」
    let mut missing: Vec<String> = Vec::new();
    for (count, label, wanted) in [
        (body.functions.len(), "函数", options.include_procedures),
        (body.procedures.len(), "存储过程", options.include_procedures),
        (body.triggers.len(), "触发器", options.include_triggers),
        (body.events.len(), "事件", options.include_events),
    ] {
        if count > 0 {
            missing.push(format!("{count} 个{label}"));
        } else if wanted {
            // 勾了「包含过程/触发器/事件」但一个都没选 ⇒ 按「全部」理解，同样导不出来
            missing.push(format!("全部{label}"));
        }
    }
    if !missing.is_empty() {
        notices.push(format!(
            "{} 的定义未包含在本次备份里（内置引擎只导出表与视图；命令行引擎可拿到完整备份）",
            missing.join("、")
        ));
    }

    let mut rows_total = 0u64;
    for (index, table) in tables.iter().enumerate() {
        task.check_canceled().map_err(XError::internal)?;
        task.set_phase(format!("导出 {}（{}/{}）", table, index + 1, tables.len()));
        let (ddl, synthesized) = ddl_of(state, conn, table, database).await?;
        if synthesized {
            notices.push(format!("表 {table} 的建表语句按列元数据拼出（可能少索引/外键）"));
        }
        if options.include_drop {
            writeln!(
                out,
                "drop table if exists {};",
                dialect.quote(table)
            )
            .map_err(|e| XError::internal(e.to_string()))?;
        }
        writeln!(out, "{};\n", ddl.trim().trim_end_matches(';'))
            .map_err(|e| XError::internal(e.to_string()))?;

        if options.include_data {
            let columns = {
                let target = crate::api::scope::resolve(state, conn, database).await?;
                let engine = state.engine.clone();
                let name = table.clone();
                blocking(move || engine.list_columns_fresh(&target, &name)).await?
            };
            if columns.is_empty() {
                notices.push(format!("表 {table} 没有列信息，数据未导出"));
            } else {
                let names: Vec<String> = columns.iter().map(|column| column.name.clone()).collect();
                let column_list = names
                    .iter()
                    .map(|name| dialect.quote(name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let base = format!("select * from {}", dialect.quote(table));
                let mut offset = 0u64;
                loop {
                    task.check_canceled().map_err(XError::internal)?;
                    let page = dialect.limit_clause(offset, batch as u64);
                    let sql = if dialect.needs_order_by_for_paging() {
                        format!("{base} order by (select null) {page}")
                    } else {
                        format!("select * from ({base}) dbmind_page {page}")
                    };
                    let result = run_sql_in(state, conn, database, sql, batch).await?;
                    let got = result.rows.len();
                    for row in &result.rows {
                        let values: Vec<String> = row
                            .iter()
                            .map(|cell| sql_literal(cell, dialect))
                            .collect();
                        writeln!(
                            out,
                            "insert into {} ({}) values ({});",
                            dialect.quote(table),
                            column_list,
                            values.join(", ")
                        )
                        .map_err(|e| XError::internal(e.to_string()))?;
                    }
                    rows_total += got as u64;
                    if got < batch {
                        break;
                    }
                    offset += batch as u64;
                }
            }
        }
        task.set_done(index as u64 + 1);
        task.log(format!("表 {table} 已导出"));
    }
    out.flush().map_err(|e| XError::internal(e.to_string()))?;
    task.set_done((tables.len() + 1) as u64);
    let _ = file_name;
    Ok(BuiltinSummary {
        tables: tables.len() as u64,
        rows: rows_total,
        notices,
    })
}

// ------------------------------------------------------------------ 还原

/// `POST /api/backup/restore/start-local` —— 用本机文件还原。
pub async fn restore_start_local(
    State(state): State<AppState>,
    Json(body): Json<RestoreRequest>,
) -> XResult<Json<Value>> {
    let file_path = body
        .file_path
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("缺少 filePath 参数（要还原哪个备份文件）"))?;
    spawn_restore(
        &state,
        body.connection_id.clone(),
        body.database.clone(),
        PathBuf::from(file_path),
        body.engine.clone().unwrap_or_else(|| "auto".to_string()),
        body.cli.clone(),
    )
}

/// `POST /api/backup/restore/start` —— multipart 上传备份文件后还原。
pub async fn restore_start(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> XResult<Json<Value>> {
    let mut connection_id = String::new();
    let mut database = String::new();
    let mut file_name = String::new();
    let mut bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| XError::bad_request(format!("解析上传表单失败：{e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        let source_name = field.file_name().map(str::to_string);
        let data = field
            .bytes()
            .await
            .map_err(|e| XError::bad_request(format!("读取上传内容失败：{e}")))?;
        match name.as_str() {
            "file" => {
                file_name = source_name.unwrap_or_default();
                bytes = Some(data.to_vec());
            }
            "connectionId" => connection_id = String::from_utf8_lossy(&data).trim().to_string(),
            "database" => database = String::from_utf8_lossy(&data).trim().to_string(),
            _ => {}
        }
    }
    let bytes = bytes.ok_or_else(|| XError::bad_request("没有收到上传的备份文件（字段名应为 file）"))?;
    if bytes.is_empty() {
        return Err(XError::bad_request("上传的备份文件是空的"));
    }
    // 落盘再还原：还原过程要多次读这个文件，留在内存里对大备份不合适
    let dir = dbmind_core::paths::work_dir().join("restore");
    dbmind_core::paths::ensure_dir(&dir)?;
    let path = dir.join(format!(
        "{}_{}",
        tasks::stamp(),
        safe_name(if file_name.is_empty() { "upload.sql" } else { &file_name })
    ));
    std::fs::write(&path, &bytes).map_err(|e| XError::internal(format!("保存上传文件失败：{e}")))?;
    spawn_restore(
        &state,
        connection_id,
        Some(database),
        path,
        "auto".to_string(),
        None,
    )
}

/// 起一个还原任务（上传与本机路径两条入口共用）。
fn spawn_restore(
    state: &AppState,
    connection_id: String,
    database: Option<String>,
    path: PathBuf,
    engine: String,
    cli: Option<CliTarget>,
) -> XResult<Json<Value>> {
    if connection_id.trim().is_empty() {
        return Err(XError::bad_request("缺少 connectionId 参数"));
    }
    if !path.is_file() {
        return Err(XError::bad_request(format!(
            "备份文件不存在：{}",
            path.display()
        )));
    }
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "backup.sql".to_string());
    let database = database.unwrap_or_default();
    let registry = state.tasks.clone();
    let state = state.clone();
    let registry2 = registry.clone();
    let task = registry2.spawn("restore", "restore_", move |task| {
        let state = state.clone();
        let connection_id = connection_id.clone();
        let path = path.clone();
        let database = database.clone();
        async move {
            task.set_total(100);
            // 明确选了命令行引擎：交给原生工具（它比「拆语句逐条执行」更能处理
            // 存储过程、分隔符变更、DELIMITER 这类文本还原搞不定的东西）
            if engine == "cli" {
                let kind = require_record(&state, &connection_id)
                    .await
                    .map_err(|err| err.message)?
                    .kind();
                match cli_spec(kind) {
                    Some(spec) => {
                        match run_cli_restore(
                            &state,
                            task.clone(),
                            &connection_id,
                            &spec,
                            &path,
                            &database,
                            cli.as_ref(),
                        )
                        .await
                        {
                            Ok(log) => {
                                task.step(log);
                                task.set_done(100);
                                let detail = format!("命令行还原完成（{}）", spec.restore_tool);
                                task.set_message(detail.clone());
                                task.set_result(json!({ "detail": detail }));
                                return Ok(Some(json!({ "detail": detail })));
                            }
                            Err(err) => {
                                return Err(format!(
                                    "命令行还原失败：{}（该类型可用内置引擎逐条执行语句）",
                                    err.message
                                ))
                            }
                        }
                    }
                    None => task.log(format!(
                        "{} 没有命令行引擎，改用内置引擎逐条执行",
                        kind.key().to_ascii_uppercase()
                    )),
                }
            }
            task.step(format!("读取备份文件 {file_name}"));
            let text = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| format!("读取备份文件失败：{e}"))?;
            // 按 SQL 协议切句：备份文件里就是一堆以 `;` 分隔的语句
            let statements =
                dbmind_core::split_statements(dbmind_core::RuntimeProtocol::Sql, &text);
            let usable: Vec<String> = statements
                .into_iter()
                .filter(|sql| !sql.trim().is_empty())
                .collect();
            if usable.is_empty() {
                return Err("备份文件里没有可执行的语句".to_string());
            }
            task.set_total(usable.len() as i64);
            task.step(format!("共 {} 条语句", usable.len()));
            let mut executed = 0u64;
            for (index, sql) in usable.iter().enumerate() {
                task.check_canceled()?;
                crate::api::meta::run_sql_in(&state, &connection_id, &database, sql.clone(), 1)
                    .await
                    .map_err(|err| {
                        format!(
                            "第 {} 条语句执行失败：{}\n语句（截断）：{}",
                            index + 1,
                            err.message,
                            sql.chars().take(300).collect::<String>()
                        )
                    })?;
                executed += 1;
                task.set_done(executed);
                if executed % 50 == 0 {
                    task.log(format!("已执行 {executed}/{} 条语句", usable.len()));
                }
            }
            let detail = format!(
                "还原完成：执行 {executed} 条语句（{}）",
                path.display()
            );
            task.set_message(detail.clone());
            task.set_result(json!({ "detail": detail }));
            Ok(Some(json!({ "detail": detail })))
        }
    });
    let _ = registry;
    Ok(Json(task_view(&task, true)))
}

// ------------------------------------------------------------------ 任务端点

/// `GET /api/backup/task/{taskId}` —— 备份进度。
pub async fn task_status(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    let Some(task) = state.tasks.get(&task_id) else {
        return Ok(Json(json!({
            "success": false,
            "status": "notfound",
            "message": "任务不存在或已过期（任务只保留 30 分钟）",
        })));
    };
    let mut view = task_view(&task, false);
    view["success"] = json!(true);
    Ok(Json(view))
}

/// `GET /api/backup/restore/task/{taskId}` —— 还原进度。
pub async fn restore_status(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    let Some(task) = state.tasks.get(&task_id) else {
        return Ok(Json(json!({
            "success": false,
            "status": "notfound",
            "message": "任务不存在或已过期（任务只保留 30 分钟）",
        })));
    };
    let mut view = task_view(&task, true);
    view["success"] = json!(true);
    Ok(Json(view))
}

/// `POST /api/backup/cancel/{taskId}` / `restore/cancel/{taskId}` —— 取消。
pub async fn cancel(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    match state.tasks.get(&task_id) {
        Some(task) => {
            task.cancel();
            Ok(Json(json!({ "success": true, "message": "已请求取消" })))
        }
        None => Ok(Json(json!({ "success": false, "message": "任务不存在或已过期" }))),
    }
}

/// `POST /api/backup/task/{taskId}/install-{confirm|manual|skip}`（还原侧同款）。
///
/// 三个按钮只是把「用户的决定」记到任务上，真正干活的是正在等待的任务体。
pub async fn install_decision(
    State(state): State<AppState>,
    Path((task_id, action)): Path<(String, String)>,
) -> XResult<Json<Value>> {
    let Some(task) = state.tasks.get(&task_id) else {
        return Ok(Json(json!({
            "success": false,
            "status": "notfound",
            "message": "任务不存在或已过期",
        })));
    };
    // 路径末段是 `install-confirm` / `install-manual` / `install-skip`；
    // 备份与还原两条路由共用这个处理器（决定本身与是哪个方向无关）
    let decision = action
        .trim_start_matches("install-")
        .trim_start_matches("restore-")
        .to_string();
    task.decide_install(&decision);
    let mut view = task_view(&task, false);
    view["success"] = json!(true);
    Ok(Json(view))
}

/// `GET /api/backup/download/{taskId}` —— 下载备份文件。
pub async fn download(State(state): State<AppState>, Path(task_id): Path<String>) -> Response {
    let Some(task) = state.tasks.get(&task_id) else {
        return (StatusCode::NOT_FOUND, "任务不存在或已过期").into_response();
    };
    let Some(artifact) = task.artifact() else {
        return (StatusCode::BAD_REQUEST, "备份文件尚未生成").into_response();
    };
    match std::fs::read(&artifact.path) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, artifact.media_type.clone()),
                (
                    header::CONTENT_DISPOSITION,
                    tasks::content_disposition(&artifact.filename),
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            format!("备份文件不存在：{}", artifact.path.display()),
        )
            .into_response(),
    }
}
