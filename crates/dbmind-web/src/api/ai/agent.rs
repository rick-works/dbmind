//! `/api/ai/agent` —— **智能体：把「问答」变成「自己去查」**。
//!
//! 前 100 多个 AI 接口都是「一问一答」：把已知的元数据塞进提示词，模型吐一段文本。
//! 但很多问题（「哪张表最能反映业务活跃度？」「上周的订单为什么少了？」）的答案
//! **不在提示词里，而在数据里** —— 除非模型能自己动手查，否则它只能猜，而猜数据库的问题
//! 比不回答更糟。
//!
//! 所以这里是唯一一个**多轮循环**：模型提出工具调用 → 我们执行 → 把结果回灌 → 它继续。
//!
//! ## 三个工具，以及为什么只有三个
//!
//! | 工具 | 用途 |
//! |---|---|
//! | `list_tables` | 先有「库里有什么」，才谈得上问「哪张表…」 |
//! | `describe_table` | 列名/类型/主键 —— 写对 SQL 的前提 |
//! | `run_readonly_sql` | 真正取数（最多 100 行） |
//!
//! 不加「写库/改结构/删表」类工具是**刻意的**：智能体的价值在于把「你去看一眼数据」
//! 这件事自动化，而不是把改数据这件事自动化。真需要改，它给出 SQL，由用户自己执行 ——
//! 那时用户看得到、也拦得住。
//!
//! ## 两层只读
//!
//! 第一层是**语句分类**（`statement::is_read_only`）：不是只读就直接拒，并把原因当
//! 工具结果回给模型（它能据此改口，而不是撞墙后卡死）。
//! 第二层是**内核闸门**（`AccessContext::Ai`）：AI 通道在内核里默认按只读处理，
//! 即使第一层被绕过，引擎也会拦。两道锁不是冗余 —— 一层是「体验」、一层是「安全」，
//! 前者可以绕过（比如新语法），后者不行。
//!
//! ## 边界
//!
//! - 最多 `maxRounds` 轮（默认 6，上限 12）。用完还没结论就**如实说没查完**，
//!   并附上最后一轮的工具结果 —— 而不是硬编一个答案。
//! - 工具结果截断到 4000 字符：一次 `select *` 的返回能把上下文挤爆，
//!   那时模型会开始胡说，不如让它看到「结果被截断了，请加 where/limit」。

use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use dbmind_core::{is_read_only, AccessContext, AiJobRow, QueryOptions, QueryRequest};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::ai::tools::{columns_of, tables_of};
use crate::api::ai::{config, run_blocking};
use crate::api::blocking;
use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::shape;
use crate::AppState;

/// 默认轮数上限。按步骤做（看表 → 看结构 → 证据 → 给建议 → 出单）
/// 每步都可能要 1~2 次查询，6 轮会刚好卡在「还没下结论」上，给到 8；
/// 再多基本就是模型在绕圈（用尽时还会强制要一次结论，见 run_loop 末尾）。
const DEFAULT_ROUNDS: u32 = 8;
const MAX_ROUNDS: u32 = 12;
/// 单次工具结果回灌给模型的最大字符数。
///
/// 这个数不是「一条消息的上限」而是**复利**：工具结果会留在对话里，第 6 轮时前 5 次的
/// 结果都要重发一遍 —— 4000 字符 × 6 轮 ≈ 2.4 万字符（近 1 万 token）只为看过的行。
/// 3200 仍然能回几十行（够定位问题），但把复利砍掉两成。
const TOOL_RESULT_LIMIT: usize = 3200;
/// 工具查询最多取多少行。
const TOOL_ROWS: usize = 100;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentReq {
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    /// 提问：前端不同入口叫 `question` / `prompt` / `text`，都收
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub max_rounds: Option<u32>,
    /// 会话 id（可选）：变更单会挂到这个会话下，审计时能顺藤摸瓜找到当时的对话
    #[serde(default)]
    pub thread_id: Option<String>,
    /// 没有 thread_id 时是否新建会话（与 `chat` 同名字段同义）。
    /// 智能体以前完全不落会话，聊完切走/刷新，左栏「记录」里就什么都没有。
    #[serde(default)]
    pub persist: bool,
    /// 界面语言（`en` / `zh`）：英文界面下智能体的回答与变更单说明也用英文
    #[serde(default)]
    pub lang: Option<String>,
    /// 聚焦表（界面底栏勾选的表）：**用户显式限定候选**。
    ///
    /// <p>以前这里根本没接收（serde 把未知字段丢掉），于是「用户重新选了表」对智能体
    /// 完全不可见 —— 它只能从历史和表清单里猜，继续操作上一轮那张表（真机反馈）。
    #[serde(default)]
    pub tables: Vec<String>,
    /// 知识库范围（与 `chat` 同义）：null = 全部库、[] = 本次不注入、[...] = 指定库。
    ///
    /// <p>深度思考以前完全不接这个字段 —— 底栏选了「全部资料库」它照样不查，
    /// 选项摆在那儿等于没接（用户要求「底栏每个选项都要生效」）。
    #[serde(default)]
    pub kb_ids: Option<Value>,
    /// 对话里选中的插件技能 id：提示词拼进 system（与 `chat` 同一套 augment_system）。
    /// 选着技能切到深度思考，技能不能凭空失效。
    #[serde(default)]
    pub plugin_skills: Vec<String>,
    /// 之前的对话轮次（`[{role, content}]`）。
    ///
    /// **智能体以前完全不收历史**：用户说「继续」它只能回答「上次的上下文我这边没有」，
    /// 然后再从头体检一遍 —— 又贵又不接话。多轮是对话的基本预期，这里收下并按
    /// 双预算裁剪（智能体还有工具结果要占上下文，所以比普通对话更紧）。
    #[serde(default)]
    pub history: Vec<config::Msg>,
}

impl AgentReq {
    fn question(&self) -> XResult<String> {
        for candidate in [&self.question, &self.prompt, &self.text] {
            if let Some(text) = candidate {
                if !text.trim().is_empty() {
                    return Ok(text.trim().to_string());
                }
            }
        }
        Err(XError::bad_request("请提供问题（question）"))
    }
}

///
/// 说明文字压到一行：这份 schema 跟着**每一轮**请求走（默认最多 8 轮 = 白付 8 次），
/// 模型只需要知道「什么情况调、参数是什么」。删掉的是举例与客套，语义一条没少。
fn tool_defs() -> Value {
    json!([
        { "type": "function", "function": {
            "name": "list_tables",
            "description": "列出表/视图（可按关键字过滤）。不确定有哪些表时先调。",
            "parameters": { "type": "object", "properties": {
                "keyword": { "type": "string", "description": "关键字，可空" } }, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "describe_table",
            "description": "看表的字段与主键。写 SQL 前先调。",
            "parameters": { "type": "object", "properties": {
                "table": { "type": "string", "description": "表名" } }, "required": ["table"] }
        }},
        { "type": "function", "function": {
            "name": "run_readonly_sql",
            "description": "跑只读 SQL，最多回 100 行；select/show/explain/desc 之外会被拒。",
            "parameters": { "type": "object", "properties": {
                "sql": { "type": "string", "description": "只读 SQL" } }, "required": ["sql"] }
        }},
        { "type": "function", "function": {
            "name": "patrol_tables",
            "description": "结构巡检（无主键/过度可空/超长文本），纯规则不耗 token。问「库有什么问题」先调。",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "suggest_indexes",
            "description": "给主键/索引建议 + 可执行 DDL（只读，不改库）。table 留空扫整库。",
            "parameters": { "type": "object", "properties": {
                "table": { "type": "string", "description": "只看这张表，可空" } }, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "sync_status",
            "description": "最近的同步任务（进度/失败行数/原因）。",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "backup_status",
            "description": "最近的备份/还原任务（成败/产物/耗时）。",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "propose_change",
            "description": "把 SQL 变成待确认的变更单（不执行）。改数据/改结构必须先调；返回单号、风险等级、撤销语句；同一句 SQL 重复提返回已有单号。",
            "parameters": { "type": "object", "properties": {
                "sql": { "type": "string", "description": "单条 SQL" },
                "title": { "type": "string", "description": "做什么" },
                "reason": { "type": "string", "description": "依据" } }, "required": ["sql"] }
        }}
    ])
}

fn system_prompt(kind: &str, database: &str, table_hint: &str, focus: &[String]) -> String {
    let db = if database.trim().is_empty() {
        "连接默认库".to_string()
    } else {
        database.to_string()
    };
    // 聚焦表 = 用户显式限定：只说这是候选、其余「不提供」是不够的 ——
    // 智能体会从历史与表清单里挑别的表接着做（用户：「我表都重新选了，它还在操作旧表」）。
    // 所以这里写成硬约束，并要求换表前先问。
    // 文案口径：这份 system 跟着**每一轮**请求走（默认最多 8 轮），
    // 所以只留「必须遵守的约束」，删掉解释性的重复与举例（语义一条不少）。
    let focus_note = if focus.is_empty() {
        String::new()
    } else {
        format!(
            "【聚焦表】只用：{}。历史里出现过的其它表一律不用；需要别的表先问用户。\n",
            focus.join("、")
        )
    };
    format!(
        "你是数据库智能体：先看真实库，再回答。连接类型：{kind}；当前库：{db}。\n\
         {focus_note}{table_hint}\n\
         规则：\n\
         1. 写 SQL 前先 list_tables / describe_table 看清表和字段，不猜表名列名。\n\
         2. 取数用 run_readonly_sql（写/改/删会被拒）；**改数据或改结构一律走 propose_change\n\
            出待确认变更单**，绝不自己执行，也不要把 SQL 甩给用户自己贴\n\
            （那样没有风险分级、撤销语句和审计记录）。\n\
         3. 问「库里有什么问题 / 哪张表慢」：先 patrol_tables、suggest_indexes，\n\
            再跑几条 SQL 拿证据，最后把值得做的改动出成变更单。\n\
         4. 同步看 sync_status，备份看 backup_status，别猜。\n\
         5. SQL 用上面的方言；结果被截断就加 where / limit 再查。\n\
         6. 结论只来自工具返回的数据，**不编数字**，查不到就说查不到。\n\
         7. 按步骤讲清这件事怎么做的（顺序随问题，不套模板）。**一步做一步说**：\n\
            先写这一步的加粗小标题（**① 看清表结构**，编号从 ① 连续）和要查什么，\n\
            再调这一步的 1~2 个工具，看到结果先写结论（关键数字 / 哪几列 / 判断），再进下一步；\n\
            别一口气跑完所有工具最后再补报告。工具结果回来后直接接着写，别重复刚写过的标题。\n\
         8. **不重复**：工具原文、变更单里的 SQL / 依据 / 风险 / 撤销，界面已经显示，\n\
            回答里不再抄；同一件事只说一次，只讲「为什么这么做、看到了什么、结论是什么」。\n\
         9. 要改数据 / 改结构：先 propose_change，再写「这一步要你点『允许』，批准后我接着做」，\n\
            然后停下等确认；批准续跑后从这一步往下做，不从头上再查一遍。\n\
         10. 结尾给结论与下一步（不超过 3 条）。\n\
         11. 有对话历史就接着最近一条结论做，不说「我没有上文」；确实分不清就先按最近结论做一步，再问一句确认。\n\
         \n\
         格式（按内容选，不要一律大段文字）：几行几列的数据（字段清单 / 统计 / 对比 / 候选索引）\n\
         用 markdown 表格；SQL / DDL 放围栏代码块，一条一块；步骤标题一行加粗；\n\
         风险 / 注意 / 结论 / 下一步用引用块起头「> 风险：…」（界面渲染成彩色提示卡）；\n\
         验证写成「**验证**：…」+ 关键数字，别只说「已验证」。\n\
         只有 propose_change 真的返回成功才说「变更单已生成」；返回已有单号就说\n\
         「这张单之前提过（单号 …），可直接在卡片上点允许」，不再复述 SQL / 依据。\n\
         \n\
         节奏：每步 1~2 次查询，整件事 3~6 步；能下结论就停，别为「步骤完整」反复查同一张表；\n\
         轮次快用完先给结论，没查到的直接说没查到。"
    )
}

/// 助手消息只保留必要字段（避免把 `refusal`/`audio` 之类原样回灌）。
fn assistant_message(message: &Value) -> Value {
    let mut out = json!({ "role": "assistant" });
    out["content"] = message.get("content").cloned().unwrap_or(json!(""));
    if let Some(calls) = message.get("tool_calls") {
        out["tool_calls"] = calls.clone();
    }
    out
}

fn truncate(text: String) -> String {
    if text.chars().count() <= TOOL_RESULT_LIMIT {
        return text;
    }
    let head: String = text.chars().take(TOOL_RESULT_LIMIT).collect();
    format!("{head}\n…（结果超过 {TOOL_RESULT_LIMIT} 字符已截断，请加 where / limit 缩小范围）")
}

/// 把查询结果渲染成模型读得懂的紧凑文本（TSV 比 JSON 省一半 token）。
///
/// 命中「字段脱敏」规则的列在这里打码：智能体自己 `select` 一次就能拿到真实行，
/// 光给上下文里的样例值脱敏等于没脱。
///
/// `table` 是调用方从工具 SQL 保守解析出的单表名：单表 SELECT 能命中钉表规则，
/// JOIN / 子查询等解析不出时传 None —— 只应用「表为空 = 任意表」的规则，
/// 免得分不清是哪张表时把无关列也打成 `***`，反而让模型失明。
fn render_rows(
    result: &dbmind_core::QueryResult,
    masks: &crate::api::ai::mask::MaskConfig,
    mask_key: &crate::api::ai::mask::MaskKey,
    conn: &str,
    database: &str,
    table: Option<&str>,
) -> String {
    let names: Vec<String> = result.columns.iter().map(|column| column.name.clone()).collect();
    let mut out = names.join("\t");
    for row in &result.rows {
        out.push('\n');
        out.push_str(
            &row.iter()
                .enumerate()
                .map(|(index, cell)| {
                    let text = match shape::cell_to_value(cell) {
                        Value::Null => String::new(),
                        Value::String(text) => text,
                        other => other.to_string(),
                    };
                    match names.get(index).and_then(|name| {
                        let hit = masks.method_for(
                            crate::api::ai::mask::MaskChannel::Llm, conn, database, table, name);
                        hit.or_else(|| {
                            // Doris catalog 的 `catalog.库`：兼容按裸库名配的规则
                            let (_, bare) = database.rsplit_once('.')?;
                            if bare.is_empty() { None } else {
                                masks.method_for(crate::api::ai::mask::MaskChannel::Llm,
                                    conn, bare, table, name)
                            }
                        })
                    }) {
                        Some(method) => crate::api::ai::mask::mask_value(&text, method, mask_key),
                        None => text,
                    }
                })
                .collect::<Vec<_>>()
                .join("\t"),
        );
    }
    if result.rows.is_empty() {
        out.push_str("\n（没有数据）");
    } else {
        out.push_str(&format!("\n（{} 行）", result.rows.len()));
    }
    out
}

/// 以 **AI 通道** 身份执行一条只读语句。
async fn run_ai_sql(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
) -> XResult<dbmind_core::QueryResult> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let engine = state.engine();
    let request = QueryRequest {
        schema: None,
        read_only: None,
        connection: target,
        sql,
        options: QueryOptions {
            max_rows: TOOL_ROWS,
            timeout_ms: 60_000,
        },
        execution_id: None,
        // 会话名带 ai: 前缀：日志与宿主会话里能一眼看出「这条是智能体跑的」
        session: Some("ai:agent".to_string()),
        // **不进查询历史**：这些是智能体自己发的探测语句，不是用户敲的 SQL。
        // 首页「最近查询」的语义是「**我**执行过什么」—— 智能体一轮对话要跑十几条，
        // 混进去就把用户自己的操作淹掉了（对话本身留在 AI 面板里，查得到）。
        internal: true,
    };
    blocking(move || engine.execute(request, AccessContext::Ai)).await
}

/// 结构化结果 → 给模型看的紧凑 JSON（pretty 只为可读，不额外增加含义）。
fn to_text(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// 执行一次工具调用，返回 `(是否成功, 给模型看的文本)`。
///
/// `changes` 是**出参**：`propose_change` 生成的变更单同时要回给前端（用户要点按钮），
/// 所以顺手收进来，最后随响应一起返回 —— 不靠模型在回答里「复述」id。
async fn execute_tool(
    state: &AppState,
    conn: &str,
    database: &str,
    protocol: dbmind_core::RuntimeProtocol,
    thread_id: &str,
    name: &str,
    args: &Value,
    changes: &mut Vec<Value>,
    emit: Option<&Emit>,
) -> (bool, String) {
    match name {
        "list_tables" => {
            let keyword = args
                .get("keyword")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            match tables_of(state, conn, database).await {
                Ok(tables) => {
                    let lines: Vec<String> = tables
                        .iter()
                        .filter(|table| {
                            keyword.is_empty() || table.name.to_ascii_lowercase().contains(&keyword)
                        })
                        .take(200)
                        .map(|table| {
                            format!(
                                "{}\t{}\t{}",
                                table.name,
                                if table.kind == dbmind_core::TableKind::View {
                                    "视图"
                                } else {
                                    "表"
                                },
                                table
                                    .row_estimate
                                    .map(|rows| format!("约 {rows} 行"))
                                    .unwrap_or_else(|| "行数未知".to_string())
                            )
                        })
                        .collect();
                    if lines.is_empty() {
                        (true, "（没有匹配的表）".to_string())
                    } else {
                        (true, format!("表名\t类型\t行数估算\n{}", lines.join("\n")))
                    }
                }
                Err(err) => (false, err.message),
            }
        }
        "describe_table" => {
            let table = args.get("table").and_then(Value::as_str).unwrap_or("");
            if table.trim().is_empty() {
                return (false, "缺少 table 参数".to_string());
            }
            match columns_of(state, conn, database, table.trim()).await {
                Ok(columns) => {
                    if columns.is_empty() {
                        return (false, format!("表 {table} 不存在或没有列信息"));
                    }
                    let lines: Vec<String> = columns
                        .iter()
                        .map(|column| {
                            format!(
                                "{}\t{}\t{}\t{}",
                                column.name,
                                column.type_name.clone().unwrap_or_else(|| "?".to_string()),
                                if column.nullable { "可空" } else { "非空" },
                                if column.primary_key { "主键" } else { "" }
                            )
                        })
                        .collect();
                    (true, format!("字段\t类型\t可空\t标记\n{}", lines.join("\n")))
                }
                Err(err) => (false, err.message),
            }
        }
        "run_readonly_sql" => {
            let sql = args.get("sql").and_then(Value::as_str).unwrap_or("").trim();
            if sql.is_empty() {
                return (false, "缺少 sql 参数".to_string());
            }
            // 第一层：语句分类（复用内核的判定，规则只有一份）。理由要说清楚 ——
            // 模型看到原因才能换个写法继续，而不是原地重试同一条被拒的语句
            if !is_read_only(protocol, sql) {
                return (
                    false,
                    "拒绝执行：这个工具只允许只读查询（select / show / explain / desc / with）。\
                     如果你需要修改数据，请把 SQL 写在最终回答里，由用户自己确认执行。"
                        .to_string(),
                );
            }
            match run_ai_sql(state, conn, database, sql.to_string()).await {
                Ok(result) => {
                    // 每次工具调用读一次规则：本地元数据库的主键查询，代价远小于一次工具往返；
                    // 换来的好处是「用户刚在设置里改了规则，下一条工具结果立刻生效」。
                    let masks = crate::api::ai::mask::load(state).await;
                    let mask_key = crate::api::ai::mask::load_key(state, &masks).await;
                    let table = dbmind_core::mask::single_table_of(sql);
                    (true, render_rows(&result, &masks, &mask_key, conn, database, table.as_deref()))
                }
                Err(err) => (false, err.message),
            }
        }
        "patrol_tables" => match crate::api::ai::tools::patrol_scan(state, conn, database).await {
            Ok(value) => (true, to_text(&value)),
            Err(err) => (false, err.message),
        },
        "suggest_indexes" => {
            let only = args
                .get("table")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let only = if only.is_empty() { None } else { Some(only) };
            match crate::api::ai::tools::index_suggestions(state, conn, database, only.as_deref()).await {
                Ok(value) => (true, to_text(&value)),
                Err(err) => (false, err.message),
            }
        }
        "sync_status" => {
            let rows = state.tasks.recent(&["sync"], 5);
            if rows.is_empty() {
                (true, "最近没有同步任务".to_string())
            } else {
                (true, to_text(&Value::Array(rows)))
            }
        }
        "backup_status" => {
            let rows = state.tasks.recent(&["backup", "restore"], 5);
            if rows.is_empty() {
                (true, "最近没有备份或还原任务".to_string())
            } else {
                (true, to_text(&Value::Array(rows)))
            }
        }
        "propose_change" => {
            let sql = args.get("sql").and_then(Value::as_str).unwrap_or("").trim();
            if sql.is_empty() {
                return (false, "缺少 sql 参数".to_string());
            }
            let title = args.get("title").and_then(Value::as_str).unwrap_or("");
            let reason = args.get("reason").and_then(Value::as_str).unwrap_or("");
            match crate::api::ai::teammate::create_change(
                state, conn, database, thread_id, "", title, reason, sql,
            )
            .await
            {
                Ok((id, risk, undo)) => {
                    let change = json!({
                        "id": id,
                        "title": if title.trim().is_empty() { "AI 建议的变更" } else { title.trim() },
                        "sql": sql,
                        "risk": risk,
                        "undoSql": undo,
                        "status": "pending",
                    });
                    // 变更单一生成就推给前端：用户能在它「说话」的同时就看到审批卡片，
                    // 而不是等整轮跑完
                    emit_event(emit, json!({ "t": "change", "change": change.clone() }));
                    changes.push(change);
                    let reversible = if undo.is_some() {
                        "有撤销语句，执行后可以撤销。"
                    } else {
                        "这个动作没有可靠的撤销语句，执行前请先备份。"
                    };
                    let extra = if risk == "high" {
                        "，高危单还需要再点一次「确认」"
                    } else {
                        ""
                    };
                    (
                        true,
                        format!(
                            "已生成待确认的变更单 {id}（风险 {risk}）。它**尚未执行**：{reversible}\
                             请在回答里提示用户点「批准」{extra}。"
                        ),
                    )
                }
                Err(err) => (false, err.message),
            }
        }
        other => (false, format!("未知工具：{other}")),
    }
}

/// 一次循环的产出。
struct LoopOutcome {
    answer: String,
    rounds: u32,
    stopped: String,
    /// 每轮「说了话又去调工具」的那段叙述（`{ round, text }`）。
    /// 落进 message.meta.texts：刷新/重开记录时按「步骤文字 → 该步的工具」重建时间线，
    /// 否则工具会被整堆到最前面（用户反馈「刷新以后位置变了」）。
    texts: Vec<Value>,
    /// 整个循环的 token 用量（各轮相加；上游一个都没给就是 None）
    usage: Option<config::TokenUsage>,
}

/// 落一次 checkpoint：状态 + 已走步骤 + 当前答案。
///
/// 没有这一步，「第二天回来接着跑」就无从谈起 —— 中断时内存里的 messages/steps 全丢，
/// 待办里只剩一句「失败」，点进去什么都看不到。
fn checkpoint_job(job_id: &str, status: &str, steps: &[Value], result: &str) {
    if job_id.is_empty() {
        return;
    }
    let Some(store) = dbmind_core::global_store() else {
        return;
    };
    let text = serde_json::to_string(steps).unwrap_or_else(|_| "[]".to_string());
    // 写不进去就等于没有「续跑」的依据（界面照样显示已结束，点进去什么都没有）：
    // 不让它影响主流程，但必须留痕，否则之后排查「为什么没有 checkpoint」无从下手。
    if let Err(err) = store.ai_job_update(job_id, status, &text, result) {
        tracing::warn!(job = %job_id, status = %status, error = ?err, "待办 checkpoint 写入失败");
    }
}

/// 多轮循环本体：**首次运行与中断续跑共用这一份**。
///
/// 参数多是因为它就是在操作一套运行态（消息、步骤、变更单、预算）；
/// 拆成结构体只会让调用点多一层包装，读起来更绕。
/// SSE 帧通道：`Some` = 流式接口（每走一步推一帧给前端），`None` = 老的一次性接口。
///
/// 元素是 `Result<String, io::Error>`（不是裸 String）：`Body::from_stream` 要的正是
/// 这个形状，省掉一层再包装。
type Emit = tokio::sync::mpsc::UnboundedSender<Result<String, std::io::Error>>;

/// 推一帧（`data:{json}\n\n`，与 chat/stream 同一套帧格式）。
fn emit_event(emit: Option<&Emit>, payload: Value) {
    if let Some(tx) = emit {
        let _ = tx.send(Ok(format!("data:{payload}\n\n")));
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    state: &AppState,
    conn: &str,
    database: &str,
    protocol: dbmind_core::RuntimeProtocol,
    thread_id: &str,
    model: &config::ModelEntry,
    system: &str,
    messages: &mut Vec<Value>,
    steps: &mut Vec<Value>,
    changes: &mut Vec<Value>,
    max_rounds: u32,
    job_id: &str,
    emit: Option<&Emit>,
) -> XResult<LoopOutcome> {
    let mut answer = String::new();
    let mut rounds_used = 0u32;
    let mut stopped = "finished";
    let mut texts: Vec<Value> = Vec::new();
    let mut usage_sum: Option<config::TokenUsage> = None;

    for round in 1..=max_rounds {
        rounds_used = round;
        emit_event(
            emit,
            json!({ "t": "round", "round": round, "maxRounds": max_rounds }),
        );
        let model_call = model.clone();
        let system_call = system.to_string();
        let messages_call = messages.clone();
        let emit_call = emit.cloned();
        // 同步 HTTP 在 blocking 线程池里跑：tokio 的工作线程不能被一次模型调用占住。
        // 流式接口用 chat_tools_stream：模型的每段话边说边推给前端（"深度思考"的过程感
        // 就靠这个），工具调用的参数照样在流末尾拼齐。
        let message = match run_blocking(move || match &emit_call {
            Some(tx) => {
                let frames = tx.clone();
                config::chat_tools_stream(
                    &model_call,
                    &system_call,
                    &messages_call,
                    &tool_defs(),
                    |delta| {
                        frames.send(Ok(format!(
                            "data:{}\n\n",
                            json!({ "t": "delta", "d": delta })
                        ))).is_ok()
                    },
                )
            }
            None => config::chat_tools(&model_call, &system_call, &messages_call, &tool_defs()),
        })
        .await
        {
            Ok(message) => message,
            Err(err) => {
                // 模型调用失败（断网 / 上游 5xx / 超时）：把已走到的步骤落库，
                // 状态置 failed —— 界面据此显示「失败可重试」，点一下能接着跑
                checkpoint_job(job_id, "failed", steps, &err.message);
                return Err(err);
            }
        };
        // 各轮 token 相加：一次回答往往要调好几轮模型，只报最后一轮会严重低估
        // （用户看到的「这次花了多少」应该是整次回答的总量）。
        if let Some(one) = message.get("usage").filter(|value| !value.is_null()) {
            let num = |key: &str| one.get(key).and_then(Value::as_u64).unwrap_or(0);
            let sum = usage_sum.get_or_insert_with(config::TokenUsage::default);
            sum.prompt_tokens += num("promptTokens");
            sum.completion_tokens += num("completionTokens");
            sum.total_tokens += num("totalTokens");
        }
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        messages.push(assistant_message(&message));

        if calls.is_empty() {
            answer = content;
            stopped = "finished";
            break;
        }
        // 这一轮「说了话又去调工具」：模型刚流出去的那段是**思考过程**，不是结论 ——
        // 前端据此把它收进时间线（留在正文里会让用户以为答完了）。
        emit_event(
            emit,
            json!({ "t": "round_end", "round": round, "calls": calls.len(), "thought": content }),
        );
        // 同一段叙述也按轮次记下来：刷新后要靠它把「这一步说了什么」放回工具前面
        if !content.is_empty() {
            texts.push(json!({ "round": round, "text": &content }));
        }
        for call in calls {
            let name = call
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // arguments 是**字符串**（OpenAI 规范），有些实现直接给对象，两种都认
            let args = match call
                .get("function")
                .and_then(|function| function.get("arguments"))
            {
                Some(Value::String(text)) => {
                    serde_json::from_str::<Value>(text).unwrap_or_else(|_| json!({}))
                }
                Some(value @ Value::Object(_)) => value.clone(),
                _ => json!({}),
            };
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("call")
                .to_string();
            let started = Instant::now();
            // 工具开始跑就先推一帧「正在做什么」：真实执行可能要几秒（describe/巡检/跑 SQL），
            // 等结果出来再显示等于让用户干等
            emit_event(
                emit,
                json!({ "t": "step_start", "round": round, "tool": name, "args": args }),
            );
            let (ok, result) = execute_tool(
                state, conn, database, protocol, thread_id, &name, &args, changes, emit,
            )
            .await;
            let duration_ms = started.elapsed().as_millis() as u64;
            let step = json!({
                "round": round,
                "tool": name,
                "args": args,
                "ok": ok,
                "durationMs": duration_ms,
                "result": result.chars().take(600).collect::<String>(),
            });
            emit_event(emit, json!({ "t": "step", "step": step }));
            steps.push(step);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "name": name,
                "content": truncate(result),
            }));
        }
        // 每轮结束落一次：即使下一轮模型调用挂掉，这一轮的成果也保住了
        checkpoint_job(job_id, "running", steps, &answer);
        if round == max_rounds {
            stopped = "maxRounds";
            answer = content;
        }
    }

    // 轮次用尽却一句话没说：**再要一次结论**（这次不给工具）。
    // 旧行为是把最后一条工具结果原样贴给用户（像 dump），看着就是「查了半天没回答」。
    if stopped == "maxRounds" && answer.trim().is_empty() {
        let system_call = system.to_string();
        let model_call = model.clone();
        let mut forced = messages.clone();
        forced.push(json!({
            "role": "user",
            "content": "工具轮数已经用完，不要再调用任何工具。\
                        基于上面已经查到的结果，按步骤给出结论：做了哪几步、每步看到什么、\
                        结论是什么、接下来该做什么；没查到的部分明确说「没查到」，不要猜。"
        }));
        let emit_call = emit.cloned();
        let text = run_blocking(move || match &emit_call {
            Some(tx) => {
                let frames = tx.clone();
                config::chat_tools_stream(
                    &model_call,
                    &system_call,
                    &forced,
                    &json!([]),
                    |delta| {
                        frames.send(Ok(format!(
                            "data:{}\n\n",
                            json!({ "t": "delta", "d": delta })
                        ))).is_ok()
                    },
                )
            }
            None => config::chat_tools(&model_call, &system_call, &forced, &json!([])),
        })
        .await
        .map(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();
        if !text.trim().is_empty() {
            answer = text;
            stopped = "concluded";
        }
    }

    Ok(LoopOutcome {
        answer,
        rounds: rounds_used,
        stopped: stopped.to_string(),
        texts,
        usage: usage_sum,
    })
}

/// 没形成结论时如实说没查完：附最后一轮的工具结果，用户接着问一句就能续上。
fn unfinished_note(steps: &[Value], max_rounds: u32) -> String {
    let last = steps
        .last()
        .and_then(|step| step.get("result"))
        .and_then(Value::as_str)
        .unwrap_or("");
    format!(
        "已达到最大工具调用轮数（{max_rounds}），还没有形成结论。\n最后一次「{}」返回：\n{}",
        steps
            .last()
            .and_then(|step| step.get("tool"))
            .and_then(Value::as_str)
            .unwrap_or("工具"),
        last
    )
}

// ------------------------------------------------------------------ 变更单兜底

/// 从回答里的 ```sql 代码块捞出「写库语句」，补建变更单。
///
/// 工具协议要求改数据必须调 `propose_change`，但模型偶尔会把 SQL 直接贴在回答里。
/// 用户的规则是「只要有对库的实际操作（增删改）都算」—— 这里按语句分类兜住：
/// 已经出过单的同一句 SQL 不重复建，执行仍然要人点。
async fn harvest_changes(
    state: &AppState,
    conn: &str,
    database: &str,
    thread_id: &str,
    protocol: dbmind_core::RuntimeProtocol,
    answer: &str,
    changes: &mut Vec<Value>,
    emit: Option<&Emit>,
) {
    for block in sql_blocks(answer) {
        for stmt in dbmind_core::split_statements(protocol, &block) {
            let sql = stmt.trim();
            if sql.is_empty() || dbmind_core::is_read_only(protocol, sql) {
                continue;
            }
            // 同一条 SQL 已经在本次工具调用里出过单就不重复建（空白/大小写/末尾分号不影响判断）
            let key = crate::api::ai::teammate::norm_sql(sql);
            let seen = changes.iter().any(|change| {
                change
                    .get("sql")
                    .and_then(Value::as_str)
                    .is_some_and(|existing| crate::api::ai::teammate::norm_sql(existing) == key)
            });
            if seen {
                continue;
            }
            let title: String = sql.chars().take(40).collect();
            match crate::api::ai::teammate::create_change(
                state,
                conn,
                database,
                thread_id,
                "",
                &title,
                "回答里给出的修改语句（自动转为待确认的变更单）",
                sql,
            )
            .await
            {
                Ok((id, risk, undo)) => {
                    let change = json!({
                        "id": id,
                        "title": title,
                        "sql": sql,
                        "risk": risk,
                        "undoSql": undo,
                        "status": "pending",
                    });
                    emit_event(emit, json!({ "t": "change", "change": change.clone() }));
                    changes.push(change)
                }
                Err(err) => {
                    // 建单失败不能把这一轮回答也弄失败：它只是补充
                    tracing::warn!(error = %err.message, "为回答里的写 SQL 补建变更单失败");
                }
            }
        }
    }
}

/// 取出回答里的 ```sql 代码块（无语言标记的裸代码块也算 —— 模型常这么写）。
fn sql_blocks(answer: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_fence = false;
    let mut keep = false;
    let mut buf = String::new();
    for line in answer.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_fence {
                if keep && !buf.trim().is_empty() {
                    blocks.push(buf.clone());
                }
                in_fence = false;
                keep = false;
                buf.clear();
            } else {
                let lang = trimmed.trim_start_matches('`').trim().to_ascii_lowercase();
                keep = lang.is_empty() || lang.starts_with("sql") || lang.starts_with("mysql");
                in_fence = true;
                buf.clear();
            }
            continue;
        }
        if in_fence {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    blocks
}

/// `POST /api/ai/agent`：一次性返回（旧的调用方式，保留兼容）。
pub async fn run(State(state): State<AppState>, Json(req): Json<AgentReq>) -> XResult<Json<Value>> {
    Ok(Json(run_core(state, req, None).await?))
}

/// `POST /api/ai/agent/stream`：同一套逻辑的流式版本 —— 轮次、思考、工具调用、变更单
/// 边走边推，结论逐字吐出（"深度思考"的界面就靠这些帧）。
pub async fn run_stream(State(state): State<AppState>, Json(req): Json<AgentReq>) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, std::io::Error>>();
    let frames = tx.clone();
    tokio::spawn(async move {
        match run_core(state, req, Some(&frames)).await {
            Ok(result) => {
                // done 帧带完整结果：前端拿它做收尾（步骤、变更单、jobId、threadId 都在里面）
                emit_event(Some(&frames), json!({ "t": "done", "result": result }));
            }
            Err(err) => {
                // 错误也走帧：HTTP 头已经发出去了（200 SSE），只能靠帧告知，别让界面一直转圈
                emit_event(
                    Some(&frames),
                    json!({ "t": "error", "error": err.message, "code": err.code }),
                );
            }
        }
    });
    sse_response(rx)
}

/// 把帧通道包成 SSE 响应（`chat/stream` 同款响应头）。
fn sse_response(
    rx: tokio::sync::mpsc::UnboundedReceiver<Result<String, std::io::Error>>,
) -> Response {
    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/event-stream; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, no-transform"),
            (header::CONNECTION, "keep-alive"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

async fn run_core(
    state: AppState,
    req: AgentReq,
    emit: Option<&Emit>,
) -> XResult<Value> {
    let conn = req
        .connection_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))?;
    let database = req.database.clone().unwrap_or_default();
    let question = req.question()?;
    let record = crate::api::require_record(&state, &conn).await?;
    let kind = record.kind();
    let dialect = Dialect::new(kind);
    let protocol = kind.protocol();
    // 预算闸门：降级时连「表清单预探」都省掉 —— 那也是一次元数据往返，
    // 而超预算时最该省的就是上下文
    let downgraded = crate::api::ai::chat::gate_budget()?;
    let model = config::resolve_with_budget(req.model_id.as_deref(), downgraded)?;
    let max_rounds = req.max_rounds.unwrap_or(DEFAULT_ROUNDS).clamp(1, MAX_ROUNDS);

    // 聚焦表：用户在底栏勾的表就是「本次只看这几张」，一个都不空
    let focus: Vec<String> = req
        .tables
        .iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    // 表清单先探一次当「开场提示」：能省掉一整轮工具往返，
    // 而且模型知道表名后，第一句 SQL 就不容易写错。
    // **点了聚焦表就只报这几张** —— 把全库清单一列，模型就会去清单里挑别的表。
    let table_hint = if downgraded {
        String::new()
    } else if !focus.is_empty() {
        format!("用户指定的表：{}（字段用 describe_table 看）", focus.join("、"))
    } else {
        match tables_of(&state, &conn, &database).await {
            Ok(tables) if !tables.is_empty() => format!(
                "库里已有的表（供参考，仍以 list_tables 的结果为准）：{}",
                tables
                    .iter()
                    .take(40)
                    .map(|table| table.name.clone())
                    .collect::<Vec<_>>()
                    .join("、")
            ),
            _ => String::new(),
        }
    };
    // 知识库：与普通对话同一口径。召回要读文件、（向量模式下）还要打一次 embeddings，
    // 都是阻塞操作 —— 必须 spawn_blocking，别卡住整个运行时；失败就当没有资料。
    let kb_note = {
        let ids = req.kb_ids.clone();
        let asked = question.clone();
        let pick = req.model_id.clone();
        match tokio::task::spawn_blocking(move || {
            crate::api::ai::kb::recall(&asked, &ids, pick.as_deref())
        })
        .await
        {
            Ok(Some(docs)) if !docs.trim().is_empty() => {
                format!("\n【用户资料（用户选定的知识库召回，可能与本问无关）】\n{docs}\n")
            }
            _ => String::new(),
        }
    };
    // 语言约束：智能体的回答/变更单说明同样跟随界面语言
    let system = crate::api::ai::with_language(
        format!(
            "{}{kb_note}",
            system_prompt(dialect.kind.key(), &database, &table_hint, &focus)
        ),
        req.lang.as_deref(),
    );
    // 底栏选中的插件技能：提示词拼进 system（与普通对话同一套），选着技能切深度思考不会失效
    let system = crate::api::plugins::augment_system(system, &req.plugin_skills, req.lang.as_deref());

    // 待办**在跑之前**就建出来：中断才有东西可「继续」。
    // 以前是跑完才落库，中途断掉等于什么都没发生 —— 第二天回来接不上。
    let job_id = {
        let job = AiJobRow {
            id: String::new(),
            title: question.chars().take(30).collect(),
            goal: question.clone(),
            thread_id: String::new(),
            connection_id: conn.clone(),
            database_name: database.clone(),
            kind: "agent".to_string(),
            status: "running".to_string(),
            steps: "[]".to_string(),
            result: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        dbmind_core::global_store()
            .and_then(|store| store.ai_job_add(&job).ok())
            .unwrap_or_default()
    };

    // 会话记账：与普通对话同一套口径（chat.rs 的 begin_turn / append_message）。
    // 先落用户这一句 —— 即使工具调用中途失败或用户点停止，问过什么也留得住。
    let thread_id = crate::api::ai::teammate::begin_turn(
        req.thread_id.as_deref(),
        req.persist,
        &conn,
        &database,
        &question,
    )
    .unwrap_or_default();
    if !thread_id.is_empty() {
        if let Some(store) = dbmind_core::global_store() {
            let _ = store.ai_job_set_thread(&job_id, &thread_id);
        }
        crate::api::ai::teammate::append_message(&thread_id, "user", &question, None, None);
    }
    // 历史对话：6 条 / 4000 字符封顶（比普通对话的 6000 更紧 —— 智能体自己还要装
    // 工具定义与工具结果，两者抢同一份上下文）。**只收 user/assistant**，客户端塞来的
    // system 一律丢弃，否则等于绕过这里的 system 规则。
    let past = crate::api::ai::bound_history(&req.history, 6, 4000, 2000);
    let mut messages: Vec<Value> = Vec::with_capacity(past.len() + 1);
    for msg in &past {
        messages.push(json!({ "role": msg.role, "content": msg.content }));
    }
    messages.push(json!({ "role": "user", "content": question }));
    let mut steps: Vec<Value> = Vec::new();
    let mut changes: Vec<Value> = Vec::new();

    let outcome = run_loop(
        &state,
        &conn,
        &database,
        protocol,
        &thread_id,
        &model,
        &system,
        &mut messages,
        &mut steps,
        &mut changes,
        max_rounds,
        &job_id,
        emit,
    )
    .await?;
    let mut answer = outcome.answer;
    if outcome.stopped == "maxRounds" && answer.trim().is_empty() {
        answer = unfinished_note(&steps, max_rounds);
    }
    // 兜底：模型偶尔把「改数据的 SQL」直接贴在回答里而不调 propose_change。
    // 口径是「只要有对库的实际操作（增删改）都算」—— 漏一张就等于这次改库没有风险分级、
    // 撤销语句和审计记录，所以这里补成变更单（执行仍然要人点）。
    harvest_changes(
        &state,
        &conn,
        &database,
        &thread_id,
        protocol,
        &answer,
        &mut changes,
        emit,
    )
    .await;

    // 回答也落会话：刷新/换页签回来点开这条记录，能看到完整的一问一答。
    // 工具轨迹与变更单一起进 message.meta（正文只留回答本身）：深度思考的结论离开
    // 「它查了什么、出了哪几张单」就没法复核 —— 只存正文等于把依据丢了（真机反馈）。
    if !thread_id.is_empty() {
        // jobId 也存进去：重开这条记录后点「允许」，前端还要拿它续跑（否则只能等下一轮提问）；
        // texts 是每一轮的步骤叙述，刷新后要靠它把「文字 → 工具」还原成原来的顺序。
        let meta = json!({
            "steps": &steps,
            "changes": &changes,
            "jobId": &job_id,
            "texts": &outcome.texts,
        });
        // usage 一起落库：不然刷新后右下角「消耗 N tokens」就没了
        crate::api::ai::teammate::append_message(
            &thread_id,
            "assistant",
            &answer,
            outcome.usage.as_ref(),
            Some(&meta),
        );
    }

    // 收尾：有待确认的变更单 ⇒ `waiting`（界面排在待办最上面）；纯查询 ⇒ `done`。
    // 顺手把变更单挂到这张 job 上，待办点进去才看得到它产出的单子。
    let status = if changes.is_empty() { "done" } else { "waiting" };
    checkpoint_job(&job_id, status, &steps, &answer);
    if let Some(store) = dbmind_core::global_store() {
        let ids: Vec<String> = changes
            .iter()
            .filter_map(|change| change.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let _ = store.ai_change_attach_job(&ids, &job_id);
    }

    Ok(json!({
        "success": true,
        "content": answer,
        "rounds": outcome.rounds,
        "maxRounds": max_rounds,
        "stopped": outcome.stopped,
        "steps": steps,
        "changes": changes,
        "jobId": job_id,
        // 整次回答的 token 用量（各轮相加）：前端据此在气泡底部显示「消耗 N tokens」
        "usage": outcome.usage,
        "threadId": if thread_id.is_empty() { Value::Null } else { json!(thread_id) },
        "model": model.model,
        // 降级标记：超预算时上下文被压掉，前端据此提示「本次未读表结构」
        "budget": if downgraded { json!("downgraded") } else { Value::Null },
        // 「只读」现在指**智能体自己不写库**：它能出的最后一招是一张待确认的变更单，
        // 执行必须由人在界面上点。留着这个字段是为了让前端文案不用改。
        "readOnly": true,
    }))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResumeReq {
    #[serde(default)]
    pub model_id: Option<String>,
    /// 这次最多再跑几轮（默认沿用 6）
    #[serde(default)]
    pub max_rounds: Option<u32>,
    /// 聚焦表：续跑时用户**可能已经换过表了**，带上当前选择才按新表继续。
    /// 不带的话只能沿用 job 里那张旧表的上下文（用户：「下面已经重新选表了，回复的还是旧表」）。
    #[serde(default)]
    pub tables: Vec<String>,
}

/// `POST /api/ai/jobs/{id}/resume` —— 接着上次跑。
///
/// 做法是**重放**：把已落库的步骤还原成 `assistant(tool_calls) + tool(result)` 消息对，
/// 再交给同一个 `run_loop` 继续。不重新执行已完成的工具（那些查询可能很贵，
/// 而结论已经在步骤里了），所以「续跑」的代价就是一轮模型调用。
pub async fn resume(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(req): Json<ResumeReq>,
) -> XResult<Json<Value>> {
    Ok(Json(resume_core(state, id, req, None).await?))
}

/// `POST /api/ai/jobs/{id}/resume/stream` —— 续跑的流式版本。
///
/// <p>「审批之后继续」走这里：用户在变更单上点批准/拒绝，前端立刻接着调它，
/// 新的思考过程与结论继续在这种对话里吐出来（而不是把整段结果一次性甩出来）。
pub async fn resume_stream(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(req): Json<ResumeReq>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, std::io::Error>>();
    let frames = tx.clone();
    tokio::spawn(async move {
        match resume_core(state, id, req, Some(&frames)).await {
            Ok(result) => emit_event(Some(&frames), json!({ "t": "done", "result": result })),
            Err(err) => emit_event(
                Some(&frames),
                json!({ "t": "error", "error": err.message, "code": err.code }),
            ),
        }
    });
    sse_response(rx)
}

async fn resume_core(
    state: AppState,
    id: String,
    req: ResumeReq,
    emit: Option<&Emit>,
) -> XResult<Value> {
    let store = dbmind_core::global_store().ok_or_else(|| XError::bad_request("数据库未就绪"))?;
    let job = store
        .ai_job_get(&id)?
        .ok_or_else(|| XError::bad_request("待办不存在（可能已被清理）"))?;
    if job.kind != "agent" {
        return Err(XError::bad_request("只有智能体任务可以继续跑"));
    }
    if job.connection_id.trim().is_empty() {
        return Err(XError::bad_request("这张待办没有关联连接，无法继续"));
    }

    let record = crate::api::require_record(&state, &job.connection_id).await?;
    let kind = record.kind();
    let dialect = Dialect::new(kind);
    let protocol = kind.protocol();
    let max_rounds = req.max_rounds.unwrap_or(DEFAULT_ROUNDS).clamp(1, MAX_ROUNDS);
    let model = config::resolve_with_budget(req.model_id.as_deref(), false)?;

    // 重放已走步骤。工具结果在落库时截到 600 字，重放时按同一口径告知模型
    // 「这是摘要」，免得它以为拿到了完整结果。
    let saved: Vec<Value> = serde_json::from_str(&job.steps).unwrap_or_default();
    let mut messages: Vec<Value> = vec![json!({ "role": "user", "content": job.goal })];
    let mut steps: Vec<Value> = Vec::new();
    for (index, step) in saved.iter().enumerate() {
        let name = step.get("tool").and_then(Value::as_str).unwrap_or("tool");
        let args = step.get("args").cloned().unwrap_or_else(|| json!({}));
        let result = step
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let call_id = format!("call_{index}");
        messages.push(json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
                }
            }]
        }));
        messages.push(json!({
            "role": "tool",
            "tool_call_id": format!("call_{index}"),
            "name": name,
            "content": truncate(result),
        }));
        steps.push(step.clone());
    }

    // 聚焦表以**这次请求带的**为准（用户可能中途换了表）：与首跑同一套硬约束，
    // 否则续跑会沿用历史里那张旧表（ai_jobs 表没有这列，只能靠请求带过来）
    let focus: Vec<String> = req
        .tables
        .iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    let table_hint = if !focus.is_empty() {
        format!("用户指定的表：{}（字段用 describe_table 看）", focus.join("、"))
    } else {
        match tables_of(&state, &job.connection_id, &job.database_name).await {
            Ok(tables) if !tables.is_empty() => format!(
                "库里已有的表（供参考，仍以 list_tables 的结果为准）：{}",
                tables
                    .iter()
                    .take(40)
                    .map(|table| table.name.clone())
                    .collect::<Vec<_>>()
                    .join("、")
            ),
            _ => String::new(),
        }
    };
    let system = system_prompt(dialect.kind.key(), &job.database_name, &table_hint, &focus);

    checkpoint_job(&id, "running", &steps, &job.result);
    let mut changes: Vec<Value> = Vec::new();
    // 首跑已回填 job.thread_id：续跑结论与变更单回到同一条会话
    let outcome = run_loop(
        &state,
        &job.connection_id,
        &job.database_name,
        protocol,
        &job.thread_id,
        &model,
        &system,
        &mut messages,
        &mut steps,
        &mut changes,
        max_rounds,
        &id,
        emit,
    )
    .await?;

    let mut answer = outcome.answer;
    if outcome.stopped == "maxRounds" && answer.trim().is_empty() {
        answer = unfinished_note(&steps, max_rounds);
    }
    // 与首跑同一条兜底，挂在原会话上
    harvest_changes(
        &state,
        &job.connection_id,
        &job.database_name,
        &job.thread_id,
        protocol,
        &answer,
        &mut changes,
        emit,
    )
    .await;
    let ids: Vec<String> = changes
        .iter()
        .filter_map(|change| change.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let _ = store.ai_change_attach_job(&ids, &id);
    // 状态要看**这张 job 下还挂着几张待确认单**，不能只看本次新产出的：
    // 续跑时变更单是上一轮就出好的，若按「本次新增」判断就会把 waiting 误写回 done，
    // 待办里那张单子也就从「等你确认」变成「已完成」了。
    let pending = store
        .ai_change_list(Some("pending"), 500)
        .unwrap_or_default()
        .iter()
        .filter(|change| change.job_id == id)
        .count();
    let status = if changes.is_empty() && pending == 0 {
        "done"
    } else {
        "waiting"
    };
    checkpoint_job(&id, status, &steps, &answer);

    // 续跑结论回写原会话：否则刷新后会话里只有首跑那一半
    if !job.thread_id.is_empty() {
        let meta = json!({
            "steps": &steps,
            "changes": &changes,
            "jobId": &id,
            "resumed": true,
        });
        crate::api::ai::teammate::append_message(
            &job.thread_id,
            "assistant",
            &answer,
            outcome.usage.as_ref(),
            Some(&meta),
        );
    }

    Ok(json!({
        "success": true,
        "resumed": true,
        "jobId": id,
        "content": answer,
        "rounds": outcome.rounds,
        "maxRounds": max_rounds,
        "stopped": outcome.stopped,
        "steps": steps,
        "changes": changes,
        "model": model.model,
        // 续跑这段的 token 用量：前端累加到底部脚注上（只加不换）
        "usage": outcome.usage,
        // 这一段的步骤叙述（首跑那条消息的 meta.texts 里已有旧的，续跑只补新的）
        "texts": outcome.texts,
        "readOnly": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 提示词预算账本：system 与工具定义**每一轮**都要重发（默认最多 8 轮），
    /// 多 100 token 就是一次对话多付 800。改文案后跑
    /// `cargo test -p dbmind-web prompt_budget -- --nocapture` 看实际数字，
    /// 断言是防回潮的护栏：确实需要加内容时同步调大并写明理由。
    #[test]
    fn prompt_budget() {
        let est = crate::api::ai::context::estimate_tokens;
        let system = system_prompt("mysql", "demo", "", &[]);
        let with_focus = system_prompt("mysql", "demo", "", &["t_order".into(), "t_item".into()]);
        let tools = tool_defs().to_string();
        let sys_tokens = est(&system);
        let tool_tokens = est(&tools);
        println!("[budget] system       {} 字符 ≈ {} tokens", system.chars().count(), sys_tokens);
        println!("[budget] system+focus {} 字符 ≈ {} tokens", with_focus.chars().count(), est(&with_focus));
        println!("[budget] tools        {} 字符 ≈ {} tokens", tools.chars().count(), tool_tokens);
        println!("[budget] 每轮固定开销 ≈ {} tokens", sys_tokens + tool_tokens);
        assert!(sys_tokens <= 800, "智能体 system 涨到 {sys_tokens} tokens");
        assert!(tool_tokens <= 580, "工具定义涨到 {tool_tokens} tokens");
    }
}
