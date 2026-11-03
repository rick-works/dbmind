//! **智能问数（ChatBI）**：自然语言 → 只读 SQL → 结果集 → 图表规格。
//!
//! ## 为什么不用现成的对话接口
//!
//! 普通对话（`chat`）把表结构塞进 system，让模型「自由发挥」写一段 markdown。问数要的是
//! **可执行的确定性产物**：一条能跑的 SQL + 一份能直接渲染的图表描述 + 一句带数字的结论。
//! 三者必须同源（图表字段得和 SQL 的列对得上），所以这里用一次「结构化输出」把计划一次拿全，
//! 而不是让人看模型自由创作、前端再猜它的意思。
//!
//! ## 一次问答的五步（`steps` 会原样回给界面做时间线）
//!
//! 1. **选表**：拉表清单，按问题里的词给表名打分，只把候选表的列结构（含 2 行样例）喂给模型 —
//!    整库结构动辄几万 token，而且表越多模型越容易在无关表上编 SQL。
//! 2. **计划**：模型只回一个 JSON（sql / charts，summary 留空）。
//! 3. **护栏**：生成的 SQL 必须过内核的只读判定（`dbmind_core::is_read_only`），
//!    只读之外的一律拒绝 —— 问数窗口绝不能变成「一句话删库」的入口。
//! 4. **执行**：复用 `meta::run_sql_in`（跨库走 scope 解析，行数由 `max_rows` 截断）。
//! 5. **结论**：把结果前几十行再喂一次模型，写一句带数字的话（见 [`NARRATE`]）——
//!    计划阶段模型还没见过数据，那时候写的「结论」只会是空话。
//!
//! 图表字段校验同样在服务端做：模型给的 `x` / `y[].field` 必须真出现在结果列里，
//! 对不上就降级成表格 —— 前端拿到的一定是「画得出来」的规格。

use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::ai::config::{self, Msg};
use crate::api::ai::tools::tables_of;
use dbmind_core::{TableInfo, TableKind};
use crate::api::ai::{bound_history, read_only, schema_context, with_language};
use crate::api::error::{XError, XResult};
use crate::api::require_record;
use crate::AppState;

/// 结果行数上限：图表最多也就几百个点，明细表再多也看不完。
const ASK_MAX_ROWS: usize = 2000;
/// 候选表数量的**上限**（不定死具体张数 —— 按问题命中强度自适应，见 [`pick_focus`]）。
/// 上限只约束取样成本：每张候选还要读字段 + `LIMIT 2` 样例；
/// token 侧另有 schema_context 的总字符预算兜底，超了会停在完整表上。
/// 用户在界面里**点名**的表不受这条约束 —— 显式意图不该被截掉。
const FOCUS_MAX: usize = 12;
/// 强命中太少时向下一档分数扩到的数量 —— 给模型留几张「相近的表」做对比与排除，
/// 也免得唯一候选一错，模型就只能编表名。
const FOCUS_MIN: usize = 3;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AskReq {
    #[serde(default)]
    pub connection_id: String,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub history: Vec<Msg>,
    #[serde(default)]
    pub lang: Option<String>,
    /// 客户端希望的行数上限（会在 [50, 20000] 内收口）
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// 用户点名的表（界面里 @ 表 / 从表详情跳过来时给）
    #[serde(default)]
    pub tables: Vec<String>,
}

// ------------------------------------------------------------------ 提示词

/// 系统提示词：只讲**契约**，不掺业务规则（业务规则跟着数据走，在 user 里）。
///
/// 字段名与前端 `chartSpec.js` 一一对应；改这里必须同步改前端，否则图表会「有规格、画不出」。
const SYSTEM: &str = r#"你是「智能问数」引擎：把业务问题变成**一条只读 SQL**，并给出最合适的图表。
只输出一个 JSON 对象（不要解释、不要 markdown 代码块），结构如下：

{
  "intent": "query" | "clarify",
  "sql": "SELECT ...",
  "charts": [
    {
      "type": "line|bar|area|pie|donut|scatter|funnel|gauge|table",
      "title": "图表标题（不超过 20 字）",
      "x": "结果里的列名",
      "y": [{"field": "结果里的列名", "label": "显示名", "unit": "元|%|笔|空", "axis": 0, "type": "line|bar"}],
      "series": "可选：长表转多系列用的列名",
      "stack": false,
      "sort": "none|x|y_desc|y_asc",
      "limit": 50
    }
  ],
  "summary": "一句话结论，必须带 1~3 个关键数字",
  "insights": ["最多 3 条，没内容就给空数组"],
  "followups": ["2~3 个自然的追问"],
  "thinking": "你的分析思路（3~5 句，第一人称）：如何理解这个问题、为什么选这几张表和这些字段、查询怎么走（分组/排序/过滤）、为什么选这种图表；intent=clarify 时说明缺什么",
  "clarify": "intent=clarify 时写给用户的反问"
}

硬性规则：
1. 只能写**只读查询**（SELECT / WITH ... SELECT）。禁止 INSERT/UPDATE/DELETE/DDL、禁止多语句、禁止注释。
2. 表名、列名必须来自给定的【表结构】，**不要臆造**；不确定就 intent=clarify。
3. 给结果列起中文别名（`AS 订单额`），图表直接用别名做轴名。
4. 时间趋势按时间列分组（必要时按天/月截断）并升序排序；分类对比按指标降序并 LIMIT。
5. 行数控制在 5000 以内；明细问题一定带 ORDER BY + LIMIT。
6. 图表类型选择：时间序列 → line（两个量纲差很多时第二个指标 axis=1 走右轴）；
   分类对比 → bar；占比 → pie/donut；相关性 → scatter；流程转化 → funnel；
   单个聚合值 → gauge；没有可画维度 → table。
7. x 与 y[].field 必须是**结果集的列名**（用第 3 条起好的别名），不能是 SQL 表达式。
8. summary / insights 这里可以留空 —— 结论由执行后另一步按真实结果写（你先写只会是空话）。
9. 与数据无关、或缺少关键信息（没有时间范围、口径不明）时：intent=clarify，clarify 写清楚缺什么。
   clarify 里举例提到的表名也**必须逐字来自【表清单】**——编造一个不存在的表名会把用户带进沟里。
10. thinking 必填且要具体（4 句左右，第一人称，像在心里自言自语）：①我怎么理解这个问题——指标是什么、按什么维度、什么时间范围，缺什么就明说；②为什么选这几张表和这些字段，排除了哪些相近的表；③SQL 的思路——怎么过滤、怎么分组、怎么排序、为什么这样控制行数；④为什么选这种图表类型而不是别的（时间序列为什么不用柱状、占比为什么不用条形）。不要复述 SQL 原文。
"#;

/// 结论提示词：**拿到真实结果之后**才调用。
///
/// 为什么要多花一次模型调用：计划阶段模型还没见过数据，让它写"结论"只能写出
/// 「记录数最多的类型为头部类型」这种没有数字的空话（真机实测）。而问数的价值恰恰在
/// 「华东区 10 月 1,284 万，环比 +12%」这一句 —— 它必须基于结果集。只喂前 30 行，成本很低。
const NARRATE: &str = r#"你是数据分析师。根据【问题】【SQL】【结果列】【结果数据】写一句结论。
只输出一个 JSON 对象：
{"summary":"一句话结论（必须包含具体数字：总量/极值/占比/环比等，60 字以内）","insights":["最多 3 条，每条要有数字"],"followups":["2~3 个可以继续追问的问题"],"thinking":"你读这批结果、下这个结论的思路（3 句左右，第一人称）：先看哪个数字、再和什么比（极值/均值/别的组）、核对后得出什么判断；如果结果和预期不符（比如全为空、只有一行）也要说明你是怎么确认的"}
要求：
- 用中文，像分析师跟同事说话，不复述 SQL、不解释过程。
- 数字要与结果一致，不要编造结果里没有的字段。
- 结果只有 1 行时 summary 就是那个值 + 口径；结果为空时直接说"没有符合条件的数据"。
- 数据是节选时只对给出的行下结论，**不要推测未展示的行**，也不要提"前 N 行 / 样本 / 已覆盖"这类过程话。
"#;

fn user_prompt(schema: &str, question: &str, hint: &str) -> String {
    let mut text = String::new();
    text.push_str(schema);
    text.push('\n');
    if !hint.is_empty() {
        text.push_str("【用户点名的表】");
        text.push_str(hint);
        text.push('\n');
    }
    text.push_str("【问题】");
    text.push_str(question);
    text.push('\n');
    text.push_str("请按契约输出 JSON。");
    text
}

// ------------------------------------------------------------------ 选表

/// 中文业务词 → 英文表名片段的粗粒度同义映射。
///
/// 问数的问题几乎都是中文，而表名几乎都是英文（sale / passenger / vip …）——
/// 没有这层映射，中文问题在英文表名上一个字都匹配不上，打分全 0，
/// 候选就只剩「字母序兜底」那一张（真机就出现过：426 张表只给了 aibee_data_day，
/// 销售类的 xingwei_kechuan_sales / bi_shop_sale 全都没进模型视野）。
/// 每条按「问题含这个词 → 表名含这些片段之一」记分，宁滥勿缺：多给一张真实存在的表，
/// 好过让模型对着唯一候选编出 etl_wide_sale 这种根本不存在的表名。
const SYNONYMS: &[(&str, &[&str])] = &[
    ("销售", &["sale"]),
    ("销售额", &["sale"]),
    ("营收", &["sale", "income", "revenue"]),
    ("收入", &["income", "revenue", "receive"]),
    ("营业额", &["sale", "income", "revenue"]),
    ("客流", &["passenger", "flow", "traffic"]),
    ("人流", &["passenger", "flow", "traffic"]),
    ("进店", &["passenger", "flow", "traffic"]),
    ("会员", &["vip", "member"]),
    ("商场", &["mall", "market", "shop", "store"]),
    ("门店", &["shop", "store"]),
    ("店铺", &["shop", "store"]),
    ("租户", &["tenant", "lease", "contract", "shop"]),
    ("租赁", &["lease", "rent"]),
    ("招商", &["lease", "contract", "investment"]),
    ("合同", &["contract"]),
    ("铺位", &["location", "unit", "shop"]),
    ("库存", &["stock", "inventory"]),
    ("订单", &["order"]),
    ("商品", &["product", "goods", "item", "sku"]),
    ("供应商", &["supplier", "vendor"]),
    ("退款", &["refund"]),
    ("结算", &["settle"]),
    ("缴纳", &["receive", "deposit", "payment"]),
    ("保证金", &["deposit"]),
    ("用户", &["user"]),
    ("员工", &["user", "staff", "employee"]),
    ("组织", &["organization", "org", "department", "dept"]),
    ("项目", &["project"]),
    ("财务", &["finance"]),
    ("预算", &["budget"]),
    ("发票", &["invoice"]),
    ("支付", &["pay", "payment"]),
];

/// 同义层记分：问题里每个命中的中文词，给「表名含其任一英文片段」的表 +10。
/// 一个中文词最多记一次（避免 sale/sales 这类重复片段翻倍）。
fn synonym_score(name_lower: &str, question: &str) -> i32 {
    let mut score = 0;
    for (word, fragments) in SYNONYMS {
        if !question.contains(word) {
            continue;
        }
        if fragments.iter().any(|f| name_lower.contains(f)) {
            score += 10;
        }
    }
    score
}

/// 注释层记分：表注释（中文）与中文问题的 **2-gram 重叠**。
///
/// 数据库里的表注释往往就是中文业务描述（「商场销售日汇总」「会员卡余额」），
/// 它和问题共享的二字片段就是直接证据 —— 这层**不需要词典**，什么行业的库都能用。
/// 单个 bigram 容易撞上「数据」「信息」这类万金油词，所以从第 2 个命中起才计分。
fn comment_score(comment: Option<&str>, question: &str) -> i32 {
    let Some(comment) = comment.map(str::trim).filter(|s| !s.is_empty()) else {
        return 0;
    };
    let cjk: Vec<char> = question
        .chars()
        .filter(|c| ('\u{4E00}'..='\u{9FFF}').contains(c))
        .collect();
    if cjk.len() < 2 {
        return 0;
    }
    let mut hits = 0usize;
    let mut seen = std::collections::HashSet::new();
    for pair in cjk.windows(2) {
        let gram: String = pair.iter().collect();
        if seen.insert(gram.clone()) && comment.contains(&gram) {
            hits += 1;
        }
    }
    if hits < 2 {
        return 0;
    }
    6 * hits.min(6) as i32
}

/// 表名 / 问题的粗粒度相关度。
///
/// 刻意**不引入分词库**：表名几乎都是 `order_detail`、`t_sale_2024` 这种，
/// 按 `_` 与非字母数字切开后与问题做子串匹配，命中率已经够用，而且零依赖、可预测。
/// 命中整表名给的权重最高（用户直接说了表名），命中词片段次之；
/// 中文问题再叠两层：[`SYNONYMS`] 同义映射（表名是英文时）与 [`comment_score`]
/// 注释 2-gram（注释是中文时）—— 后者是各行业库通用的直接证据。
fn score_table(table: &TableInfo, question_lower: &str, question: &str) -> i32 {
    let full = table.name.to_ascii_lowercase();
    if full.len() >= 3 && question_lower.contains(&full) {
        return 100;
    }
    let mut score = synonym_score(&full, question) + comment_score(table.comment.as_deref(), question);
    for part in full.split(|c: char| !c.is_alphanumeric()) {
        // 太短的片段（t / v2 / id）匹配上也没有信息量，反而会把一堆表拉进来
        if part.len() < 3 || part.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if question_lower.contains(part) {
            score += 10;
        }
    }
    score
}

/// 挑候选表。返回 (候选, 选法)，选法用于执行过程里向用户解释「凭什么挑这几张」。
///
/// - **用户点名了表（界面勾选的聚焦表）就只传点名的，不设上限** —— 勾选是显式限定；
/// - 没点名才自动判断，数量**按命中强度自适应**：
///   - 有命中：与最高分同档（`score*2 >= best`）的全部收下；不足 [`FOCUS_MIN`] 张
///     再向下一档分数补到 [`FOCUS_MIN`]，总数封顶 [`FOCUS_MAX`]。
///     问题越聚焦候选越少（点名整名 → 1 张），问得越宽泛候选面越大；
///   - 全部 0 分（问题与表名/同义词典都沾不上）：按**表名前缀轮转**取满 [`FOCUS_MAX`] 张
///     横切面 —— 让模型看到「这个库里都有哪些家族的表」，而不是字母序第一张；
///     有了真实字段在手，它至少能说出「库里没有销售表」，而不是编一个。
/// - 点名的表一张都没对上（选错了库/表已删）时退回自动判断，别拿着空候选去问模型。
fn pick_focus(tables: &[TableInfo], question: &str, named: &[String]) -> (Vec<String>, &'static str) {
    let mut picked: Vec<String> = Vec::new();
    for want in named {
        if let Some(hit) = tables.iter().find(|t| t.name.eq_ignore_ascii_case(want.trim())) {
            let hit = hit.name.clone();
            if !picked.contains(&hit) {
                picked.push(hit);
            }
        }
    }
    if !picked.is_empty() {
        // 点名了且至少对上一张：就按点名的来（一张没对上才走下面的自动判断）
        return (picked, "named");
    }

    // ---- 没点名（或点名全部没对上）：自动判断 ----
    let lower = question.to_ascii_lowercase();
    let mut ranked: Vec<(i32, &TableInfo)> = tables
        .iter()
        .map(|table| (score_table(table, &lower, question), table))
        .collect();
    // 同分时行数多的排前面：家族的「旗舰表」比空壳表更值得给模型看
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.row_estimate.unwrap_or(0).cmp(&a.1.row_estimate.unwrap_or(0)))
    });
    let best = ranked.first().map(|(s, _)| *s).unwrap_or(0);

    let mut push = |table: &TableInfo, picked: &mut Vec<String>| {
        if !picked.iter().any(|t| t.eq_ignore_ascii_case(&table.name)) {
            picked.push(table.name.clone());
        }
    };

    if best > 0 {
        // 与最高分同档的全收（best=100 的整名命中 → 只收 100 分的；
        // best=30 时 20 分也算强档），不足 FOCUS_MIN 再向下一档补
        for (score, table) in ranked.iter() {
            if picked.len() >= FOCUS_MAX || *score * 2 < best {
                break;
            }
            push(table, &mut picked);
        }
        for (score, table) in ranked.iter() {
            if picked.len() >= FOCUS_MIN || *score <= 0 {
                break;
            }
            push(table, &mut picked);
        }
        return (picked, "hit");
    }

    // 全部 0 分：按表名前缀分组轮转，取一份跨家族的横切面（每组先给行数最多的旗舰表）
    let prefix_of = |name: &str| -> String {
        name.split(|c: char| !c.is_ascii_alphanumeric())
            .find(|s| !s.is_empty() && s.chars().any(|c| c.is_ascii_alphabetic()))
            .unwrap_or(name)
            .to_ascii_lowercase()
    };
    let mut groups: Vec<(String, Vec<&TableInfo>)> = Vec::new();
    for table in tables {
        let p = prefix_of(&table.name);
        match groups.iter_mut().find(|(g, _)| *g == p) {
            Some((_, list)) => list.push(table),
            None => groups.push((p, vec![table])),
        }
    }
    for (_, list) in groups.iter_mut() {
        list.sort_by_key(|t| std::cmp::Reverse(t.row_estimate.unwrap_or(0)));
    }
    let mut round = 0;
    loop {
        if picked.len() >= FOCUS_MAX {
            break;
        }
        let mut progressed = false;
        for (_, list) in &groups {
            if let Some(table) = list.get(round) {
                push(table, &mut picked);
                progressed = true;
                if picked.len() >= FOCUS_MAX {
                    break;
                }
            }
        }
        if !progressed {
            break;
        }
        round += 1;
    }
    (picked, "fallback")
}

// ------------------------------------------------------------------ 计划解析

#[derive(Deserialize, Default)]
struct ChartY {
    #[serde(default)]
    field: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    axis: Option<u8>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize, Default)]
struct ChartSpec {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    x: String,
    #[serde(default)]
    y: Vec<ChartY>,
    #[serde(default)]
    series: Option<String>,
    #[serde(default)]
    stack: bool,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize, Default)]
struct Plan {
    #[serde(default)]
    intent: String,
    #[serde(default)]
    sql: String,
    #[serde(default)]
    charts: Vec<ChartSpec>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    insights: Vec<String>,
    #[serde(default)]
    followups: Vec<String>,
    #[serde(default)]
    clarify: String,
    /// 模型的分析思路（可选）：为什么这样理解问题、选这些表和字段、查询与图表怎么想。
    /// 只用于「执行过程」里给用户看，不参与任何计算，缺了也不影响主流程。
    #[serde(default)]
    thinking: String,
}

/// 把长文本按字符数截断（保留可读结尾），用于把模型思路塞进步骤时兜底。
fn clip(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() > max {
        format!("{}…", t.chars().take(max).collect::<String>())
    } else {
        t.to_string()
    }
}

/// 从问题里抽「分析信号」：时间范围、TopN、图表要求、指标词。
///
/// 纯本地、零成本 —— 作为「理解问题」这一步的产出展示给用户：
/// 让他看到口径是从哪里来的，而不是等模型答完才猜。抽不到就如实说没有。
fn question_signals(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // 时间范围：先认中文时间词，再退到显式日期/月份
    const TIME_WORDS: &[&str] = &[
        "最近7天", "最近30天", "最近一周", "最近一月", "近7天", "近30天", "近一周", "近一月",
        "近一年", "本周", "上周", "本月", "上月", "今年", "去年", "昨天", "今天",
    ];
    if let Some(word) = TIME_WORDS.iter().find(|w| question.contains(*w)) {
        out.push(format!("时间范围：{word}"));
    } else if let Ok(date) = regex::Regex::new(r"\d{4}-\d{1,2}(-\d{1,2})?") {
        if let Some(m) = date.find(question) {
            out.push(format!("时间范围：{}", m.as_str()));
        }
    }
    // TopN / 排名：写了数字就带上，没写就说明按默认 10 处理
    if let Ok(topn) = regex::Regex::new(r"(?i)top\s*(\d+)|前\s*(\d+)\s*(名|条|位|个)|排名") {
        if let Some(c) = topn.captures(question) {
            match c.get(1).or_else(|| c.get(2)) {
                Some(n) => out.push(format!("要求排名：Top {}", n.as_str())),
                None => out.push("要求排名（未写明取几名，默认 10）".to_string()),
            }
        }
    }
    // 图表要求：用户点名了形态，方案阶段就该照做
    const CHART_WORDS: &[(&str, &str)] = &[
        ("柱状图", "柱状图"), ("条形图", "条形图"), ("折线图", "折线图"), ("曲线图", "折线图"),
        ("趋势图", "趋势折线图"), ("饼图", "饼图"), ("环形图", "环形图"), ("散点图", "散点图"),
        ("漏斗图", "漏斗图"), ("仪表盘", "仪表盘"), ("数据表", "数据表"), ("表格", "数据表"),
    ];
    if let Some((_, label)) = CHART_WORDS.iter().find(|(w, _)| question.contains(w)) {
        out.push(format!("图表要求：{label}"));
    }
    // 指标词：命中的直接列出，帮用户核对「它理解的指标是不是我要的」
    const METRIC_WORDS: &[&str] = &[
        "销售额", "销量", "订单数", "数量", "金额", "占比", "环比", "同比", "总数", "总计",
        "平均", "增长率", "客流", "转化率", "毛利", "利润",
    ];
    let hits: Vec<&str> = METRIC_WORDS.iter().filter(|w| question.contains(*w)).copied().collect();
    if !hits.is_empty() {
        out.push(format!("指标：{}", hits.join("、")));
    }
    out
}

/// 图表类型的中文名（与前端 chartSpec 的 CHART_TYPES 对齐）。
fn chart_kind_label(kind: &str) -> &'static str {
    match kind {
        "line" => "折线图",
        "bar" => "柱状图",
        "area" => "面积图",
        "pie" => "饼图",
        "donut" => "环形图",
        "scatter" => "散点图",
        "funnel" => "漏斗图",
        "gauge" => "仪表盘",
        "table" => "数据表",
        _ => "图表",
    }
}

/// 把图表方案说成人话：折线图《近7天销售趋势》，x=日期，y=销售额 ——
/// 只说「1 个图表方案」用户根本不知道那是什么图（真机反馈）。
fn chart_summary(charts: &[ChartSpec]) -> String {
    charts
        .iter()
        .map(|c| {
            let mut parts = vec![format!("{}《{}》", chart_kind_label(&c.kind), c.title.trim())];
            if !c.x.trim().is_empty() {
                parts.push(format!("x={}", c.x.trim()));
            }
            let ys: Vec<String> = c
                .y
                .iter()
                .filter_map(|s| {
                    let name = s
                        .label
                        .as_deref()
                        .filter(|l| !l.trim().is_empty())
                        .unwrap_or(s.field.trim());
                    if name.is_empty() { None } else { Some(name.to_string()) }
                })
                .collect();
            if !ys.is_empty() {
                parts.push(format!("y={}", ys.join("/")));
            }
            parts.join("，")
        })
        .collect::<Vec<_>>()
        .join("；")
}

/// 从模型回复里抠出第一个 JSON 对象。
///
/// 列名的简短列举（最多 6 个）：步骤明细里要能看出"返回的是什么"，又不能长到一行放不下。
fn column_sample(columns: &[String]) -> String {
    let shown: Vec<&str> = columns.iter().take(6).map(String::as_str).collect();
    if columns.len() > shown.len() {
        format!("{} 等 {} 列", shown.join("、"), columns.len())
    } else {
        shown.join("、")
    }
}

/// 单元格值压缩成一行（用于"首行"摘要）：太长截断，NULL 保留字样。
fn short_value(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::String(s) => {
            let t = s.trim();
            if t.len() > 24 { format!("{}…", &t[..24]) } else { t.to_string() }
        }
        other => {
            let t = other.to_string();
            if t.len() > 24 { format!("{}…", &t[..24]) } else { t }
        }
    }
}

/// 现实里模型经常不守「只输出 JSON」：可能带 ```json 围栏、前置一句"好的"、后置解释。
/// 这里按第一个 `{` 到最后一个 `}` 截取，失败就返回 None（上层降级成原文展示）。
fn parse_plan(text: &str) -> Option<Plan> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Plan>(&text[start..=end]).ok()
}

/// 图表字段必须能在结果列里找到（忽略大小写），否则这张图前端画不出来。
fn chart_is_valid(spec: &ChartSpec, columns: &[String]) -> bool {
    let has = |field: &str| {
        let field = field.trim();
        !field.is_empty() && columns.iter().any(|c| c.eq_ignore_ascii_case(field))
    };
    if spec.y.is_empty() {
        return false;
    }
    if spec.kind == "table" {
        return true;
    }
    // 饼/漏斗/仪表盘只需要一个维度，x 可为空
    let needs_x = !matches!(spec.kind.as_str(), "pie" | "donut" | "funnel" | "gauge");
    (spec.x.is_empty() || !needs_x || has(&spec.x))
        && spec.y.iter().all(|y| has(&y.field))
        && spec.series.as_deref().map(|s| s.is_empty() || has(s)).unwrap_or(true)
}

/// 把模型给的规格规范化：白名单类型、字段名对齐真实列名（大小写）、补默认值。
fn normalize_chart(spec: ChartSpec, columns: &[String]) -> Value {
    const TYPES: [&str; 10] = [
        "line", "bar", "area", "pie", "donut", "scatter", "funnel", "gauge", "table", "heat",
    ];
    let mut kind = spec.kind.trim().to_ascii_lowercase();
    if !TYPES.contains(&kind.as_str()) {
        kind = "bar".to_string();
    }
    let fix = |field: &str| -> String {
        let field = field.trim();
        columns
            .iter()
            .find(|c| c.eq_ignore_ascii_case(field))
            .cloned()
            .unwrap_or_else(|| field.to_string())
    };
    let y: Vec<Value> = spec
        .y
        .iter()
        .filter(|item| !item.field.trim().is_empty())
        .map(|item| {
            json!({
                "field": fix(&item.field),
                "label": item.label.clone().filter(|l| !l.trim().is_empty()).unwrap_or_else(|| fix(&item.field)),
                "unit": item.unit.clone().unwrap_or_default(),
                "axis": if item.axis == Some(1) { 1 } else { 0 },
                "type": match item.kind.as_deref() {
                    Some("line") => "line",
                    Some("bar") => "bar",
                    _ => if kind == "line" || kind == "area" { "line" } else { "bar" },
                },
            })
        })
        .collect();
    json!({
        "type": kind,
        "title": if spec.title.trim().is_empty() { "查询结果".to_string() } else { spec.title.trim().to_string() },
        "x": fix(&spec.x),
        "y": y,
        "series": spec.series.as_deref().filter(|s| !s.trim().is_empty()).map(fix),
        "stack": spec.stack,
        "sort": spec.sort.unwrap_or_else(|| "none".to_string()),
        "limit": spec.limit.unwrap_or(50).clamp(1, 500),
    })
}

// ------------------------------------------------------------------ 结论

/// 结论阶段的三段文案（与 `/api/ai/ask` 返回体里的同名字段一一对应）。
#[derive(Deserialize, Default)]
struct Narrated {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    insights: Vec<String>,
    #[serde(default)]
    followups: Vec<String>,
    /// 模型读结果、下结论时的思路（可选），给「执行过程」里的深度思考用。
    #[serde(default)]
    thinking: String,
}

/// 喂给结论模型的样例行数上限：再多也不会改变一句结论，只是白花钱。
///
/// 取值要盖住典型的「分组结果」（Top10 / 按类别统计常常 20~50 行）：截在 30 行就会让
/// 模型只能说「已展示的 30 种各 1 条」，而不是「32 种各 1 条」（真机踩过）。
const NARRATE_ROWS: usize = 50;
/// 行数据的总字符预算：行数没到上限但每行很长（宽表、长文本）时按预算提前收手。
const NARRATE_CHARS_BUDGET: usize = 4000;
/// 单行序列化后的字符上限（宽表 + 长文本列，一行就能把上下文塞满）。
const NARRATE_ROW_CHARS: usize = 600;

/// 「读结果、写结论」：把问题 + SQL + 结果列 + 前 30 行交给模型，拿回带数字的结论。
///
/// 超时/失败都**不**冒泡 —— 调用方拿到空结构后退回计划里的话或本地算出来的结论。
/// 结论是锦上添花，绝不能因为它挂了让一次已经执行成功的查询报错。
async fn narrate(
    model: config::ModelEntry,
    lang: Option<&str>,
    question: &str,
    sql: &str,
    payload: &Value,
) -> (Narrated, Option<config::TokenUsage>) {
    let prompt = narrate_prompt(question, sql, payload);
    if prompt.is_empty() {
        return (Narrated::default(), None);
    }
    let system = with_language(NARRATE.to_string(), lang);
    let messages = vec![Msg {
        role: "user".to_string(),
        content: prompt,
    }];
    // 模型调用是阻塞的，与计划阶段一样挪到 blocking 线程池
    match crate::api::ai::run_blocking(move || config::chat_with_usage(&model, &system, &messages))
        .await
    {
        Ok((reply, usage)) => (parse_narrated(&reply).unwrap_or_default(), usage),
        Err(err) => {
            tracing::warn!(error = %err.message, "问数结论生成失败，退回计划/本地结论");
            (Narrated::default(), None)
        }
    }
}

/// 两次模型调用的用量相加（界面显示的「本次用了多少」应是整条流水线的总和）。
fn merge_usage(
    base: Option<config::TokenUsage>,
    extra: Option<config::TokenUsage>,
) -> Option<config::TokenUsage> {
    match (base, extra) {
        (Some(a), Some(b)) => Some(config::TokenUsage {
            prompt_tokens: a.prompt_tokens + b.prompt_tokens,
            completion_tokens: a.completion_tokens + b.completion_tokens,
            total_tokens: a.total_tokens + b.total_tokens,
        }),
        (None, Some(b)) => Some(b),
        (base, None) => base,
    }
}

/// 拼结论提示词的 user 部分；没有列时返回空串（调用方据此跳过这次模型调用）。
fn narrate_prompt(question: &str, sql: &str, payload: &Value) -> String {
    let columns: Vec<&str> = payload
        .get("columns")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if columns.is_empty() {
        return String::new();
    }
    let types: Vec<&str> = payload
        .get("columnTypes")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let labels: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, name)| match types.get(i).map(|t| t.trim()).filter(|t| !t.is_empty()) {
            Some(tpe) => format!("{name}({tpe})"),
            None => (*name).to_string(),
        })
        .collect();
    let rows = payload
        .get("rows")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let row_count = payload
        .get("rowCount")
        .and_then(Value::as_i64)
        .unwrap_or(rows.len() as i64);

    // 取样：先按行数封顶，再按字符预算收手（宽表/长文本列时行数远到不了上限）
    let mut samples: Vec<String> = Vec::new();
    let mut used = 0usize;
    for row in rows.iter().take(NARRATE_ROWS) {
        let line = truncate_chars(&serde_json::to_string(row).unwrap_or_default(), NARRATE_ROW_CHARS);
        used += line.chars().count();
        samples.push(line);
        if used >= NARRATE_CHARS_BUDGET {
            break;
        }
    }

    let mut text = String::new();
    text.push_str("【问题】");
    text.push_str(question);
    text.push_str("\n【SQL】");
    text.push_str(sql);
    text.push_str("\n【结果列】");
    text.push_str(&labels.join("、"));
    if samples.is_empty() {
        text.push_str(&format!("\n【结果数据】共 {row_count} 行（空结果集）：\n"));
    } else if (row_count as usize) > samples.len() {
        text.push_str(&format!(
            "\n【结果数据】共 {row_count} 行，下面是节选的前 {} 行（未展示的行不要推测）：\n",
            samples.len()
        ));
    } else {
        text.push_str(&format!("\n【结果数据】共 {row_count} 行，全部如下：\n"));
    }
    for line in samples {
        text.push_str(&line);
        text.push('\n');
    }
    text.push_str("请按契约输出 JSON。");
    text
}

/// 按**字符**（不是字节）截断，避免把一个中文字切成两半。
fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}

/// 从结论回复里抠出 JSON 对象（与 `parse_plan` 同一套容错：围栏、前置客套话都常见）。
fn parse_narrated(text: &str) -> Option<Narrated> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Narrated>(&text[start..=end]).ok()
}

/// 兜底结论：不调模型，直接从结果集算一句（模型不可用/超时的时候至少不说空话）。
fn local_summary(payload: &Value, columns: &[String]) -> String {
    let rows: &[Value] = payload
        .get("rows")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let row_count = payload
        .get("rowCount")
        .and_then(Value::as_i64)
        .unwrap_or(rows.len() as i64);
    if rows.is_empty() || row_count <= 0 {
        return "没有符合条件的数据。".to_string();
    }
    let shown: Vec<String> = columns
        .iter()
        .filter(|c| c.as_str() != "__row")
        .take(4)
        .cloned()
        .collect();

    // 单行：把值念出来就是最忠实的结论（「本月订单额 1,284 万」）
    if rows.len() == 1 {
        let parts: Vec<String> = shown
            .iter()
            .map(|name| format!("{} 为 {}", name, cell_text(rows[0].get(name.as_str()))))
            .collect();
        if !parts.is_empty() {
            return format!("当前口径下{}。", parts.join("，"));
        }
        return format!("共 {row_count} 行结果。");
    }

    // 多行：找一个能比的数值列，说清最大值落在哪一行、合计多少
    let measure = shown.iter().find(|c| is_numeric_column(payload, c)).cloned();
    let label = shown.iter().find(|c| !is_numeric_column(payload, c)).cloned();
    if let Some(measure) = measure {
        let mut best: Option<(f64, &Value)> = None;
        let mut total = 0.0f64;
        for row in rows.iter() {
            let Some(value) = as_number(row.get(measure.as_str())) else {
                continue;
            };
            total += value;
            if best.map(|(cur, _)| value > cur).unwrap_or(true) {
                best = Some((value, row));
            }
        }
        if let Some((value, row)) = best {
            // 有维度列就说「最高的是华东」，没有就只说数值（明细结果没有可比维度）
            let head = match &label {
                Some(name) if best_has_label(row, name) => format!(
                    "{}最高的是 {}（{}）",
                    measure,
                    cell_text(row.get(name.as_str())),
                    format_number(value)
                ),
                _ => format!("{}最大为 {}", measure, format_number(value)),
            };
            return format!(
                "共 {row_count} 行，{}；{}合计 {}。",
                head,
                measure,
                format_number(total)
            );
        }
    }
    format!("共 {row_count} 行结果（{} 列）。", shown.len())
}

/// 维度格是不是空的（空维度说「最高的是 —」比不说更糟）。
fn best_has_label(row: &Value, name: &str) -> bool {
    match row.get(name) {
        None | Some(Value::Null) => false,
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    }
}

/// 单元格转成人能读的短文本：空值写 ——，容器类型给一段 JSON。
fn cell_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "—".to_string(),
        Some(Value::String(text)) => truncate_chars(text, 40),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(num)) => num.to_string(),
        Some(other) => truncate_chars(&serde_json::to_string(other).unwrap_or_default(), 40),
    }
}

/// 把单元格读成数字：有的驱动把数值列当字符串返回（`"1,284"` 这种还带千分位）。
fn as_number(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(num)) => num.as_f64(),
        Some(Value::String(text)) => text.trim().replace(',', "").parse::<f64>().ok(),
        _ => None,
    }
}

/// 数字的原样展示：整数就不带小数点，小数最多留两位。
fn format_number(value: f64) -> String {
    if (value - value.round()).abs() < f64::EPSILON && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value:.2}")
    }
}

// ------------------------------------------------------------------ 主流程

/// 步骤事件的推送器。
///
/// 一次性接口传 [`Tracer::noop`]（什么都不发），流式接口传一对 channel —— 同一份
/// `run_ask` 因此既能等完整结果，也能让「读取表结构」这类步骤**在发生的当下**就出现在
/// 界面上。步骤的 label / 明细仍然只由真实执行结果决定，这里不提前猜任何文案。
#[derive(Clone)]
pub struct Tracer(Option<tokio::sync::mpsc::UnboundedSender<Result<String, std::io::Error>>>);

impl Tracer {
    fn noop() -> Self {
        Self(None)
    }

    /// 某一步开始：界面先看到「正在做什么」，跑完再被 [`push_step`] 换成真实结果。
    fn begin(&self, key: &str, label: &str) {
        self.begin_with(key, label, "", None);
    }

    /// 带「进行中内容」的开始帧：这一步**此刻就已经确定**的信息（比如执行前就写好的 SQL、
    /// 正在读哪几张表）趁它在跑就先推给界面 —— 等到跑完才给，用户看到的就是一闪而过。
    /// 前端约定：running 的那一步自动展开显示这些内容，跑完自动折叠。
    fn begin_with(&self, key: &str, label: &str, detail: &str, code: Option<&str>) {
        let mut ev = json!({ "type": "step", "key": key, "label": label, "status": "running" });
        if !detail.is_empty() {
            ev["detail"] = json!(detail);
        }
        if let Some(code) = code {
            ev["code"] = json!(code);
        }
        self.emit(ev);
    }

    fn emit(&self, value: Value) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(Ok(format!("data:{}\n\n", value)));
        }
    }
}

/// 记一步并**同时**推给流式端：`steps` 留给最终响应体，事件留给还在等的界面。
/// 推的帧把同一步的 `input / output / code` 也带上 —— 这样界面能在这一步完成的瞬间，
/// 把「模型写了什么 SQL、拿到了什么结果」一并点亮，而不用等整轮结束。
fn push_step(steps: &mut Vec<Value>, tracer: &Tracer, step: Value) {
    let mut ev = step.clone();
    ev["type"] = json!("step");
    ev["status"] = json!("done");
    tracer.emit(ev);
    steps.push(step);
}

/// `POST /api/ai/ask` —— 一次问数（等完整结果）。
pub async fn ask(State(state): State<AppState>, Json(req): Json<AskReq>) -> XResult<Json<Value>> {
    Ok(Json(run_ask(state, req, Tracer::noop()).await?))
}

/// `POST /api/ai/ask/stream` —— 一次问数，**过程实时推**（SSE）。
///
/// 帧格式与 `/api/ai/chat/stream` 一致（`data:{...}\n\n`）：
/// - `{"type":"step", "key", "label", "status":"running"|"done", "detail"?, "ms"?}`
/// - `{"answer": {...}}`：完整回答（与一次性接口的响应体逐字段相同），随后 `event:done`
/// - `{"error": "..."}`
///
/// 为什么值得单开一条：问数要跑 4~10 秒。全黑等 8 秒，和「一步步看到它在读表、执行、
/// 写结论」是完全不同的信任度 —— 后者才让人相信数字是真的查出来的。
pub async fn ask_stream(State(state): State<AppState>, Json(req): Json<AskReq>) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, std::io::Error>>();
    let tracer = Tracer(Some(tx.clone()));
    tokio::spawn(async move {
        let tail = match run_ask(state, req, tracer).await {
            Ok(body) => format!(
                "data:{}\n\nevent:done\ndata:{{\"done\":true}}\n\n",
                json!({ "answer": body })
            ),
            Err(err) => format!("data:{}\n\n", json!({ "error": err.message })),
        };
        let _ = tx.send(Ok(tail));
    });
    let body = Body::from_stream(tokio_stream::wrappers::UnboundedReceiverStream::new(rx));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream;charset=UTF-8")
        .header(header::CACHE_CONTROL, "no-cache")
        // 有反向代理时不让它缓冲，否则流式退化成「等完再一次性吐出」
        .header("X-Accel-Buffering", "no")
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 一次问数的主体：`ask` 与 `ask_stream` 共用，差别只在 `tracer`。
async fn run_ask(state: AppState, req: AskReq, tracer: Tracer) -> XResult<Value> {
    let question = req.question.trim().to_string();
    if question.is_empty() {
        return Err(XError::bad_request("请输入你想问的问题"));
    }
    let conn = req.connection_id.trim().to_string();
    if conn.is_empty() {
        return Err(XError::bad_request("缺少 connectionId 参数"));
    }
    let database = req.database.trim().to_string();
    let started = std::time::Instant::now();
    let mut steps: Vec<Value> = Vec::new();

    // 理解问题（本地、零成本）：把问题拆成指标/维度/时间/图表四个要素。
    // **只在真有可拆的信号时才出现** —— 「最近7天销售额画个柱状图」值得拆，
    // 「有多少订单」拆出来是空的，硬凑一步反而是噪音。步骤多少跟着内容走，不凑数。
    let signals = question_signals(&question);
    if !signals.is_empty() {
        let t_parse = std::time::Instant::now();
        let signals_text = signals.join("；");
        tracer.begin_with(
            "parse",
            "理解问题",
            &format!("把问题拆成指标 / 维度 / 时间 / 图表四要素，先抽出：{signals_text}"),
            None,
        );
        push_step(&mut steps, &tracer, json!({
            "key": "parse",
            "label": "理解问题",
            "detail": signals_text.clone(),
            "input": format!("原始问题：{}", question),
            "output": signals_text,
            "thinking": "先不碰数据库，把问题拆成四个要素：看什么指标（求和 / 计数 / 占比）、按什么维度分组（时间 / 类别）、什么时间范围（决定 WHERE 的边界）、想要什么形态的图。时间范围决定过滤条件，维度决定 GROUP BY，图表要求决定可视化形态；缺了哪一项，下一步要么按默认口径补齐，要么向用户反问，绝不瞎猜。",
            "ms": t_parse.elapsed().as_millis() as u64,
        }));
    }

    // 选表：连表清单都拿不到就不必问模型了（报错比"模型说没表"清楚）
    let t0 = std::time::Instant::now();
    tracer.begin_with("schema", "读取表结构", "正在列出这个库的表，按问题挑候选表…", None);
    let tables = tables_of(&state, &conn, &database).await?;
    let names: Vec<String> = tables.iter().map(|t| t.name.clone()).collect();
    if names.is_empty() {
        return Err(XError::bad_request(
            "这个库里没有可查询的表（或当前账号看不到任何表），换一个库再问",
        ));
    }
    let (focus, focus_how) = pick_focus(&tables, &question, &req.tables);
    // 候选确定后刷新一次进行中内容：让「正在读哪些表」实时可见，而不是黑盒等结果
    tracer.begin_with(
        "schema",
        "读取表结构",
        &format!("已挑出 {} 张候选（{}），正在读取字段、类型与样例行…", focus.len(), focus.join("、")),
        None,
    );
    // 用户勾了聚焦表 = 限定候选：表清单只列点名的表、明确其余不猜（省 token 且防跑偏）
    let schema = schema_context(&state, &conn, &database, &focus, !req.tables.is_empty()).await?;
    // 「凭什么挑这几张表」：表多的时候筛选逻辑才值得解释；就几张表时全量展开，一句带过。
    let schema_thinking = if names.len() > 30 {
        let named: Vec<String> = focus
            .iter()
            .filter(|t| req.tables.iter().any(|n| n.eq_ignore_ascii_case(t)))
            .cloned()
            .collect();
        let rest: Vec<String> = focus
            .iter()
            .filter(|t| !named.iter().any(|n| n.eq_ignore_ascii_case(t)))
            .cloned()
            .collect();
        let why = if named.is_empty() {
            let how = match focus_how {
                "hit" => "命中的按强度自适应收下（强档全要、太少再向下一档补几张扬长的），不是定死张数",
                _ => "问题与表名完全对不上，我按表名前缀轮转取了一批跨家族的表当横切面 —— 至少让下一步看到这个库里真实有什么字段，而不是凭空编",
            };
            format!(
                "问题里没有点名具体表，我把问题关键词与全部 {} 张表的表名逐个匹配打分（整表名命中权重最高；中文业务词按同义映射折算成英文片段再匹配，如 销售→sale、客流→passenger）。{}，本次 {} 张（{}）",
                names.len(),
                how,
                focus.len(),
                focus.join("、")
            )
        } else if rest.is_empty() {
            format!("你点名的表 {} 直接进候选", named.join("、"))
        } else {
            format!(
                "你点名的表 {} 直接进候选；{} 则是按关键词与全部 {} 张表名匹配打分挑出来的（整表名命中权重最高）",
                named.join("、"),
                rest.join("、"),
                names.len()
            )
        };
        format!(
            "{why}；没选中的表与本问题无关，一个字节都不读。再把候选表的字段、类型和样例行展开 —— 只有看到真实列名，下一步才不会凭空猜出「这张表里好像有个销售额字段」这种幻觉。"
        )
    } else {
        format!(
            "这个库一共 {} 张表，数量不多，{} 张候选（{}）的字段全部展开给下一步看，不必按相关度裁剪 —— 表少的时候，多看几张比挑错表更划算。",
            names.len(),
            focus.len(),
            focus.join("、")
        )
    };
    push_step(&mut steps, &tracer, json!({
        "key": "schema",
        "label": format!("读取表结构（{} 张候选表，共 {} 张）", focus.len(), names.len()),
        // 展开「执行过程」时给用户看的明细：到底读了哪些表，而不是一句笼统的"读取表结构"
        "detail": format!(
            "按问题相关度从 {} 张表里挑出 {} 张候选，并读取字段与样例（共 {} 个字段）",
            names.len(), focus.len(), schema.columns
        ),
        // 这一步的「输入/输出」：让用户看清「喂了什么、拿到了什么」
        "input": format!("问题：{}\n数据库：{}\n可查阅表：{} 张", question, database, names.len()),
        "output": format!("候选表：{}", focus.join("、")),
        // 分析思路（本地、可复现）：说清楚"凭什么挑这几张表"，而不是只给结果
        "thinking": schema_thinking,
        "ms": t0.elapsed().as_millis() as u64,
    }));

    // 计划：模型调用是阻塞的，必须挪到 blocking 线程池（见 ai 模块文件头决定 1）
    let t1 = std::time::Instant::now();
    tracer.begin_with(
        "plan",
        "生成查询与图表方案",
        "把候选表字段、问题口径与最近几轮追问交给模型：先判断要不要反问，再写只读 SQL、定分组与排序、挑图表形态",
        None,
    );
    let downgraded = crate::api::ai::chat::gate_budget()?;
    let model = config::resolve_with_budget(req.model_id.as_deref(), downgraded)?;
    // 结论阶段复用同一个模型（克隆一份，原值要 move 进下面的 blocking 闭包）
    let narrate_model = model.clone();
    let mut messages: Vec<Msg> = bound_history(&req.history, 4, 3000, 1500);
    messages.push(Msg {
        role: "user".to_string(),
        content: user_prompt(&schema.text, &question, &req.tables.join("、")),
    });
    let system = with_language(SYSTEM.to_string(), req.lang.as_deref());
    config::audit("ask", &question);
    let (reply, usage) =
        crate::api::ai::run_blocking(move || config::chat_with_usage(&model, &system, &messages))
            .await?;

    let plan = parse_plan(&reply).ok_or_else(|| {
        XError::internal(format!(
            "模型的回答不是合法的 JSON 方案，请重试或换一个模型。原文开头：{}",
            reply.chars().take(160).collect::<String>()
        ))
    })?;
    // 方案解析成功后才记这一步：解析失败时不该出现一个「已完成」的步骤
    let clarify = plan.intent.eq_ignore_ascii_case("clarify");
    // 模型自述的分析思路：这是「怎么分析的」的核心。模型没给就退到一段如实的本地描述
    let plan_thinking = {
        let from_model = clip(&plan.thinking, 1400);
        if !from_model.is_empty() {
            from_model
        } else if clarify {
            "问题缺少时间范围或指标口径，没法确定查询边界，先向用户反问把条件说清。".to_string()
        } else {
            format!(
                "读完候选表字段后，我按问题口径写出一条只读 SQL（{} 个字符），并按结果形态选了 {} 张图表方案（{}）。",
                plan.sql.trim().chars().count(),
                plan.charts.len(),
                plan.charts.iter().map(|c| chart_kind_label(&c.kind)).collect::<Vec<_>>().join("、")
            )
        }
    };
    // 图表方案说成人话：什么图、什么标题、x/y 是哪列 —— 「1 个图表方案」等于什么都没说
    let charts_text = chart_summary(&plan.charts);
    push_step(&mut steps, &tracer, json!({
        "key": "plan",
        "label": "生成查询与图表方案",
        "detail": if clarify {
            "判定问题缺少必要信息，改为向用户反问".to_string()
        } else {
            // 命中了哪几张候选表：比"SQL 多少个字符"更接近用户想看的「结果」
            let hit: Vec<&str> = focus
                .iter()
                .filter(|t| plan.sql.contains(t.as_str()))
                .map(String::as_str)
                .collect();
            let kinds: Vec<&str> = plan.charts.iter().map(|c| chart_kind_label(&c.kind)).collect();
            format!(
                "命中表 {} · SQL {} 个字符 · 图表 {} 张（{}）",
                if hit.is_empty() { "（未识别）".to_string() } else { hit.join("、") },
                plan.sql.trim().chars().count(),
                plan.charts.len(),
                kinds.join("、")
            )
        },
        // 这一步最该让人看见「模型怎么分析的」：它到底写出了什么 SQL
        "input": format!("候选表（已展开字段结构）：{}\n任务：写一条只读 SQL 与图表方案", focus.join("、")),
        "output": if clarify {
            "模型判定缺少关键信息，转而向用户反问".to_string()
        } else {
            format!("只读 SQL 已生成（{} 个字符）\n图表方案：{}", plan.sql.trim().chars().count(), charts_text)
        },
        "code": if clarify { Value::Null } else { json!(plan.sql) },
        "thinking": plan_thinking,
        "ms": t1.elapsed().as_millis() as u64,
    }));

    // 澄清类：不执行，直接把反问交给界面
    if plan.intent.eq_ignore_ascii_case("clarify") || plan.sql.trim().is_empty() {
        let clarify = if plan.clarify.trim().is_empty() {
            "这个问题还缺少必要信息：请说明时间范围、统计口径（哪个指标）或关注的维度".to_string()
        } else {
            plan.clarify.trim().to_string()
        };
        return Ok(json!({
            "success": true,
            "intent": "clarify",
            "question": question,
            "clarify": clarify,
            "summary": plan.summary,
            "followups": plan.followups,
            "sql": "",
            "columns": [],
            "columnTypes": [],
            "rows": [],
            "rowCount": 0,
            "charts": [],
            "candidateTables": focus,
            "steps": steps,
            "elapsedMs": started.elapsed().as_millis() as u64,
            "usage": usage,
        }));
    }

    // 护栏：只读。写语句/多语句一律拒绝，并明确告诉用户窗口的边界
    let sql = plan.sql.trim().trim_end_matches(';').to_string();
    if !read_only(&sql) {
        return Err(XError::bad_request(
            "问数窗口只执行只读查询（SELECT）。这条问题需要修改数据，请到 SQL 编辑器里处理",
        ));
    }

    // 执行：SQL 在这一刻就已经确定了，趁查询在跑先把代码亮出来 —— 用户不用等跑完才看到
    let t2 = std::time::Instant::now();
    tracer.begin_with(
        "exec",
        "执行查询",
        "SQL 先过只读校验（只允许 SELECT，拒绝写语句和多语句），然后发到数据库执行",
        Some(sql.trim()),
    );
    let max_rows = req.max_rows.unwrap_or(ASK_MAX_ROWS).clamp(50, 20_000);
    let result = crate::api::meta::run_sql_in(&state, &conn, &database, sql.clone(), max_rows)
        .await
        .map_err(|err| XError::bad_request(format!("SQL 执行失败：{}", err.message)))?;
    let payload = crate::api::shape::query_result_json(&result);
    let columns: Vec<String> = payload
        .get("columns")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let row_count = payload
        .get("rowCount")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    // 首行样例：让"结果"是看得见的，而不是一句抽象的"返回 10 行"
    let first_sample = payload
        .get("rows")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(Value::as_object)
        .map(|obj| {
            obj.iter()
                .filter(|(k, _)| *k != "__row")
                .take(4)
                .map(|(k, v)| format!("{} = {}", k, short_value(v)))
                .collect::<Vec<_>>()
                .join("，")
        })
        .unwrap_or_default();
    push_step(&mut steps, &tracer, json!({
        "key": "exec",
        "label": format!("执行查询（{row_count} 行）"),
        // 这一步的「结果」必须写清楚：返回了什么列、多少行，0 行时说清是"查成功但没数据"
        // 而不是"出错" —— 两者在界面上长得一样，用户会误以为查询挂了
        "detail": if row_count == 0 {
            format!(
                "查询执行成功，0 行匹配{}（共 {} 列：{}）",
                if result.truncated { "（已截断）" } else { "" },
                columns.len(),
                column_sample(&columns)
            )
        } else {
            format!(
                "返回 {row_count} 行 × {} 列：{}{}",
                columns.len(),
                column_sample(&columns),
                if result.truncated { "（已按行数上限截断）" } else { "" }
            )
        },
        // 输入就是发给数据库的 SQL；输出是实打实的结果（行数、列、首行）
        "input": sql.clone(),
        "output": if row_count == 0 {
            "查询成功，但没有匹配的数据".to_string()
        } else {
            format!(
                "返回 {} 行 × {} 列：{}\n首行：{}",
                row_count, columns.len(), column_sample(&columns), first_sample
            )
        },
        "thinking": format!(
            "SQL 先过只读校验（只允许 SELECT，拒绝写语句和多语句），再在库 {} 上执行、最多取 {} 行；{}",
            database, max_rows,
            if row_count == 0 {
                "结果为空，说明当前筛选条件下确实没有匹配数据，不是查询出错。".to_string()
            } else {
                format!("返回 {} 行，下一段据此核对数字再写结论。", row_count)
            }
        ),
        "ms": t2.elapsed().as_millis() as u64,
    }));

    // 结论：**拿到结果之后**再让模型看前 30 行写（见 NARRATE 的注释）。
    // 失败/超时都不影响主流程：退回一段由结果集算出来的本地结论。
    let t3 = std::time::Instant::now();
    tracer.begin_with(
        "narrate",
        "阅读结果并写结论",
        &if row_count == 0 {
            "结果集为空：把空结果如实交给模型，让它向用户说明当前条件没有匹配数据".to_string()
        } else {
            format!(
                "把结果集前 {} 行交给模型：先看总量与极值，核对数字后写结论与洞察",
                NARRATE_ROWS.min(row_count as usize)
            )
        },
        None,
    );
    let (narrated, narrate_usage) = narrate(
        narrate_model,
        req.lang.as_deref(),
        &question,
        &sql,
        &payload,
    )
    .await;
    let usage = merge_usage(usage, narrate_usage);
    // 「结论是谁写的」也是准确性的一部分：模型没答上就只能退回本地汇总，界面上要说清楚
    let narrate_detail = if row_count == 0 {
        "结果为空：向用户说明当前条件没有匹配数据".to_string()
    } else if narrated.summary.trim().is_empty() {
        format!("模型没有给出可用结论，改用结果集本地汇总（{row_count} 行）")
    } else {
        format!("模型基于结果集撰写结论与 {} 条洞察", narrated.insights.len())
    };
    // 这一步的「输出」就是结论本身（看得见的"结果"），太长就截断，细节看上方正文
    let narrate_output = if row_count == 0 {
        "向用户说明当前条件没有匹配到数据".to_string()
    } else {
        let s = narrated.summary.trim();
        if s.is_empty() {
            format!("改用结果集本地汇总（{} 行）", row_count)
        } else if s.chars().count() > 200 {
            format!("{}…", s.chars().take(200).collect::<String>())
        } else {
            s.to_string()
        }
    };
    // 模型读结果、下结论时的思路；没给就用一段如实的本地描述
    let narrate_thinking = {
        let from_model = clip(&narrated.thinking, 1000);
        if !from_model.is_empty() {
            from_model
        } else if row_count == 0 {
            "结果集为空，不能给出带数字的结论，如实说明当前条件没有匹配数据。".to_string()
        } else {
            format!(
                "我读结果集前 {} 行，先看总量与极值、再看分组间的差异，核对数字后写成一句可直接用的结论。",
                NARRATE_ROWS.min(row_count as usize)
            )
        }
    };
    push_step(&mut steps, &tracer, json!({
        "key": "narrate",
        "label": "阅读结果并写结论",
        "detail": narrate_detail,
        "input": if row_count == 0 {
            "结果集为空".to_string()
        } else {
            format!("结果集前 30 行（{} 行 × {} 列）", row_count, columns.len())
        },
        "output": narrate_output,
        "thinking": narrate_thinking,
        "ms": t3.elapsed().as_millis() as u64,
    }));
    let summary = if narrated.summary.trim().is_empty() {
        let plan_text = plan.summary.trim();
        if plan_text.is_empty() {
            local_summary(&payload, &columns)
        } else {
            plan_text.to_string()
        }
    } else {
        narrated.summary.trim().to_string()
    };
    let insights = if narrated.insights.is_empty() {
        plan.insights.clone()
    } else {
        narrated.insights.clone()
    };
    let followups = if narrated.followups.is_empty() {
        plan.followups.clone()
    } else {
        narrated.followups.clone()
    };

    // 图表：只保留字段对得上的；一张都不剩就退成表格（宁可给表，不要给空白）
    let mut charts: Vec<Value> = Vec::new();
    let mut fallback = false;
    if !columns.is_empty() {
        for spec in plan.charts.iter() {
            if chart_is_valid(spec, &columns) {
                charts.push(normalize_chart(
                    ChartSpec {
                        kind: spec.kind.clone(),
                        title: spec.title.clone(),
                        x: spec.x.clone(),
                        y: spec
                            .y
                            .iter()
                            .map(|y| ChartY {
                                field: y.field.clone(),
                                label: y.label.clone(),
                                unit: y.unit.clone(),
                                axis: y.axis,
                                kind: y.kind.clone(),
                            })
                            .collect(),
                        series: spec.series.clone(),
                        stack: spec.stack,
                        sort: spec.sort.clone(),
                        limit: spec.limit,
                    },
                    &columns,
                ));
            }
        }
        if charts.is_empty() {
            fallback = true;
            if let Some(first) = columns.iter().find(|c| *c != &"__row") {
                let numeric = columns
                    .iter()
                    .find(|c| is_numeric_column(&payload, c))
                    .cloned();
                charts.push(json!({
                    "type": if numeric.is_some() { "bar" } else { "table" },
                    "title": "查询结果",
                    "x": first,
                    "y": numeric.map(|field| vec![json!({"field": field, "label": field, "axis": 0, "type": "bar", "unit": ""})])
                        .unwrap_or_default(),
                    "series": Value::Null,
                    "stack": false,
                    "sort": "none",
                    "limit": 50,
                }));
            } else {
                charts.push(json!({
                    "type": "table", "title": "查询结果", "x": "", "y": [],
                    "series": Value::Null, "stack": false, "sort": "none", "limit": 50,
                }));
            }
        }
    }

    let mut body = payload;
    let object = body
        .as_object_mut()
        .ok_or_else(|| XError::internal("查询结果结构异常"))?;
    object.insert("success".to_string(), json!(true));
    object.insert("intent".to_string(), json!("query"));
    object.insert("question".to_string(), json!(question));
    object.insert("sql".to_string(), json!(sql));
    object.insert("charts".to_string(), json!(charts));
    object.insert("chartFallback".to_string(), json!(fallback));
    object.insert("summary".to_string(), json!(summary));
    object.insert("insights".to_string(), json!(insights));
    object.insert("followups".to_string(), json!(followups));
    // 本次真正喂给模型的候选表：执行过程里能直接看到"读了哪些表"，也便于核对模型没瞎编表名
    object.insert("candidateTables".to_string(), json!(focus));
    object.insert("steps".to_string(), json!(steps));
    object.insert(
        "elapsedMs".to_string(),
        json!(started.elapsed().as_millis() as u64),
    );
    object.insert(
        "usage".to_string(),
        serde_json::to_value(usage).unwrap_or(Value::Null),
    );
    Ok(body)
}

/// 粗判某列是不是数值（图表兜底时用来找「能画的那一列」）。
fn is_numeric_column(payload: &Value, column: &str) -> bool {
    let types = payload.get("columnTypes").and_then(Value::as_array);
    let columns = payload.get("columns").and_then(Value::as_array);
    let index = columns
        .and_then(|list| {
            list.iter().position(|c| {
                c.as_str()
                    .map(|name| name.eq_ignore_ascii_case(column))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(usize::MAX);
    if index == usize::MAX {
        return false;
    }
    let type_name = types
        .and_then(|list| list.get(index))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    ["int", "dec", "num", "float", "double", "real", "money", "bigint"]
        .iter()
        .any(|token| type_name.contains(token))
}

// ------------------------------------------------------------------ 建议问题

/// `GET /api/ai/ask/examples?connectionId&database` —— 问题示例（**不调模型**）。
///
/// 为什么要它：空白输入框是问数产品的第一道门槛 —— 用户不知道该问什么，也就永远不会知道
/// 这个功能能干什么。示例从真实表结构里按列类型推（时间列 → 趋势；类别列 → 分布；
/// 数值列 → 汇总），点一下就能问，比任何 onboarding 文案都有效。
pub async fn examples(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = crate::api::Params::parse(raw.as_deref());
    let conn = params.get("connectionId").unwrap_or_default().trim().to_string();
    let database = params.get("database").unwrap_or_default().trim().to_string();
    if conn.is_empty() {
        return Err(XError::bad_request("缺少 connectionId 参数"));
    }
    require_record(&state, &conn).await?;

    let tables = tables_of(&state, &conn, &database).await?;
    if tables.is_empty() {
        return Ok(Json(json!({ "success": true, "examples": [] })));
    }
    // 行数多的表更有"可问性"（空表和字典表的示例只会让人失望）
    let mut ranked = tables.clone();
    ranked.sort_by(|a, b| b.row_estimate.unwrap_or(0).cmp(&a.row_estimate.unwrap_or(0)));

    let mut items: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for table in ranked.iter().take(4) {
        let columns = match crate::api::ai::tools::columns_of(&state, &conn, &database, &table.name)
            .await
        {
            Ok(list) if !list.is_empty() => list,
            _ => continue,
        };
        let date_col = columns
            .iter()
            .find(|c| is_time_column(c))
            .map(|c| c.name.clone());
        // 指标与维度都按"像不像业务字段"打分后再挑：不筛的话会给出
        // 「按月统计 dingtalk_config 的 enabled 趋势」这种没人想问的示例（真机踩过）
        let metric = columns
            .iter()
            .filter(|c| is_number_column(c) && metric_score(c) > 0)
            .max_by_key(|c| metric_score(c))
            .map(|c| c.name.clone());
        let dim = columns
            .iter()
            .filter(|c| is_category_column(c) && dim_score(c) > 0)
            .max_by_key(|c| dim_score(c))
            .map(|c| c.name.clone());

        let mut push = |question: String, tag: String| {
            if items.len() >= 8 || seen.iter().any(|s| s == &question) {
                return;
            }
            seen.push(question.clone());
            items.push(json!({ "question": question, "table": tag }));
        };
        match (date_col.clone(), metric.clone()) {
            (Some(date), Some(metric)) => push(
                format!("按月统计 {} 的 {} 趋势（时间列 {date}）", table.name, metric),
                table.name.clone(),
            ),
            // 没有像样的数值列就退回「行数趋势」——依然是有意义的问题
            (Some(date), None) => push(
                format!("{} 每月新增多少条数据？（按 {date}）", table.name),
                table.name.clone(),
            ),
            _ => {}
        }
        match (dim.clone(), metric.clone()) {
            (Some(dim), Some(metric)) => push(
                format!("{} 各 {} 的 {} 排名 Top 10", table.name, dim, metric),
                table.name.clone(),
            ),
            (Some(dim), None) => push(
                format!("{} 按 {} 分组各有多少条记录？", table.name, dim),
                table.name.clone(),
            ),
            _ => {}
        }
        if let Some(metric) = metric {
            push(
                format!("{} 的 {} 合计和平均值是多少？", table.name, metric),
                table.name.clone(),
            );
        }
        let recent = match date_col {
            Some(date) => format!("{} 最近 30 天（按 {date}）新增了多少条数据？", table.name),
            None => format!("{} 一共有多少行数据？", table.name),
        };
        push(recent, table.name.clone());
    }
    Ok(Json(json!({ "success": true, "examples": items, "tables": tables.len() })))
}

fn is_time_column(column: &dbmind_core::ColumnDetail) -> bool {
    let name = column.name.to_ascii_lowercase();
    let tpe = column.type_name.clone().unwrap_or_default().to_ascii_lowercase();
    tpe.contains("date") || tpe.contains("time")
        || name.contains("date") || name.contains("time") || name.ends_with("_at")
        || name.contains("日期") || name.contains("时间")
}

/// 「像度量吗」：业务指标（金额/数量/金额类）加分，标志位/排序号/端口这类数字降权。
///
/// 不做这一步，示例会给出「按月统计 dingtalk_config 的 enabled 趋势」—— 用户不会想问这种问题，
/// 而一个不能点的问题示例比没有示例更糟。
fn metric_score(column: &dbmind_core::ColumnDetail) -> i32 {
    let name = column.name.to_ascii_lowercase();
    // 这些名字本身就是"枚举/标志/编号"，哪怕列类型是 int 也不能当度量
    let reserved = [
        "id", "status", "type", "level", "kind", "state", "sort", "order", "priority", "port",
        "seq", "version", "enabled", "disabled", "deleted", "flag", "code", "sort_order",
    ];
    let flag_like = reserved.contains(&name.as_str())
        || name.ends_with("_id")
        || name.ends_with("_code")
        || name.starts_with("is_")
        || name.ends_with("_flag")
        || name.ends_with("_enabled")
        || name.ends_with("_sort")
        || name.ends_with("_port")
        || name.ends_with("_seq")
        || name.ends_with("_version")
        || name.ends_with("_level")
        || name.ends_with("_priority")
        || name.ends_with("_type")
        || name.ends_with("_status");
    if flag_like {
        return 0;
    }
    let metric_like = [
        "amount", "amt", "price", "total", "sum", "count", "cnt", "qty", "quantity", "num",
        "score", "rate", "ratio", "fee", "tax", "cost", "revenue", "salary", "duration", "size",
        "sales", "profit", "balance", "weight",
    ];
    let hits = metric_like.iter().filter(|token| name.contains(*token)).count();
    if hits > 0 {
        return 10 * hits as i32;
    }
    // 普通数值列（既不是标志位也不像指标）：仍可当度量，但排在指标后面
    1
}

/// 「像维度吗」：类型/状态/地区/渠道这类加分；用户名、邮箱、URL 这类标识符降权。
fn dim_score(column: &dbmind_core::ColumnDetail) -> i32 {
    let name = column.name.to_ascii_lowercase();
    let identifier_like = [
        "username", "password", "email", "token", "url", "host", "phone", "mobile", "path",
        "file", "remark", "comment", "note", "description", "content", "json", "uuid",
    ];
    if identifier_like.iter().any(|token| name.contains(token)) {
        return 0;
    }
    let dim_like = [
        "type", "status", "category", "channel", "region", "province", "city", "area", "store",
        "shop", "level", "group", "kind", "brand", "product", "dept", "department", "team",
        "owner", "source", "platform", "tag", "gender", "state",
    ];
    let hits = dim_like.iter().filter(|token| name.contains(*token)).count();
    if hits > 0 {
        return 10 * hits as i32;
    }
    1
}

fn is_number_column(column: &dbmind_core::ColumnDetail) -> bool {
    let tpe = column.type_name.clone().unwrap_or_default().to_ascii_lowercase();
    if column.primary_key {
        return false;
    }
    ["int", "dec", "num", "float", "double", "real", "money"]
        .iter()
        .any(|token| tpe.contains(token))
        && !tpe.contains("point")
}

fn is_category_column(column: &dbmind_core::ColumnDetail) -> bool {
    let name = column.name.to_ascii_lowercase();
    let tpe = column.type_name.clone().unwrap_or_default().to_ascii_lowercase();
    let textual = tpe.contains("char") || tpe.contains("text") || tpe.contains("string");
    textual
        && ["type", "status", "category", "channel", "region", "level", "group", "kind", "name"]
            .iter()
            .any(|token| name.contains(token))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ti(name: &str, comment: Option<&str>) -> TableInfo {
        TableInfo {
            name: name.to_string(),
            kind: TableKind::Table,
            row_estimate: None,
            comment: comment.map(str::to_string),
        }
    }

    #[test]
    fn score_prefers_whole_table_name() {
        let question = "order_detail 里最近的销售额是多少";
        assert!(
            score_table(&ti("order_detail", None), question, question)
                > score_table(&ti("order_head", None), question, question)
        );
    }

    #[test]
    fn pick_focus_keeps_named_tables_first() {
        let tables = vec![ti("a", None), ti("orders", None), ti("users", None)];
        let (picked, how) = pick_focus(&tables, "有多少订单", &["orders".to_string()]);
        assert_eq!(picked.first().map(String::as_str), Some("orders"));
        assert_eq!(how, "named");
    }

    /// 表注释是中文时，注释 2-gram 直接命中中文问题 —— 不依赖词典，各行业通用。
    #[test]
    fn chinese_comment_directly_matches_chinese_question() {
        let tables = vec![
            ti("t_daily_agg", Some("商场销售日汇总")),
            ti("sys_log_2024", Some("系统运行日志")),
            ti("aibee_data_day", None),
        ];
        let (picked, how) = pick_focus(&tables, "近三个月商场每天的销售额怎么样？", &[]);
        assert_eq!(how, "hit");
        assert!(
            picked.contains(&"t_daily_agg".to_string()),
            "注释命中的表应进候选，实际：{picked:?}"
        );
        assert!(
            !picked.contains(&"sys_log_2024".to_string()),
            "只共享万金油词的日志表不该被拉进来，实际：{picked:?}"
        );
    }

    /// 问题里直接写了表名（整名命中 100 分）：候选就它一张 —— 问题越聚焦候选越少。
    #[test]
    fn whole_name_hit_gives_a_tight_candidate_set() {
        let tables = vec![
            ti("orders", None),
            ti("orders_2024", None),
            ti("users", None),
            ti("goods", None),
        ];
        let (picked, how) = pick_focus(&tables, "orders 表里最近一天有多少单", &[]);
        assert_eq!(how, "hit");
        assert!(picked.iter().all(|t| t.starts_with("orders")), "实际：{picked:?}");
    }

    /// 真机回归：426 张表、全英文表名、中文问题 —— 同义映射必须把销售类表拉进候选，
    /// 而不是只给字母序第一的 aibee_data_day。
    #[test]
    fn chinese_question_hits_sales_tables_via_synonyms() {
        let tables = vec![
            ti("aibee_data_day", None),
            ti("bi_shop_sale", None),
            ti("xingwei_kechuan_sales", None),
            ti("xingwei_kechuan_passenger", None),
            ti("etl_wide_contractd", None),
        ];
        let (picked, how) = pick_focus(&tables, "近三个月商场每天的销售额怎么样？", &[]);
        assert_eq!(how, "hit");
        assert!(
            picked.iter().any(|t| t.contains("sale") || t.contains("sales")),
            "销售表应进候选，实际：{picked:?}"
        );
        // 「商场」→ shop/mall：bi_shop_sale 的分应高于纯 passenger 表
        assert!(picked.first().map(|t| t.contains("shop") || t.contains("sale")).unwrap_or(false));
    }

    /// 全部 0 分（问题与表名完全沾不上边）时按前缀轮转兜底：同一前缀的家族给满上限，
    /// 而不是只给字母序第一张 —— 那正是「只看到 aibee_data_day」的根因。
    #[test]
    fn zero_score_fallback_fills_focus_max() {
        let tables: Vec<TableInfo> = (0..20).map(|i| ti(&format!("tzz_{i:02}"), None)).collect();
        let (picked, how) = pick_focus(&tables, "最近情况怎么样", &[]);
        assert_eq!(how, "fallback");
        assert_eq!(picked.len(), FOCUS_MAX.min(tables.len()));
    }

    /// 前缀横切面：不同家族的表都要露脸，而不是被某一家族占满。
    #[test]
    fn zero_score_fallback_spans_prefixes() {
        let mut tables: Vec<TableInfo> = Vec::new();
        for p in ["aaa", "bbb", "ccc", "ddd"] {
            for i in 0..10 {
                tables.push(ti(&format!("{p}_table_{i}"), None));
            }
        }
        let (picked, how) = pick_focus(&tables, "最近情况怎么样", &[]);
        assert_eq!(how, "fallback");
        let prefixes: Vec<&str> = picked.iter().map(|t| t.split('_').next().unwrap()).collect();
        for p in ["aaa", "bbb", "ccc", "ddd"] {
            assert!(prefixes.contains(&p), "前缀 {p} 应出现在横切面里，实际：{picked:?}");
        }
    }

    #[test]
    fn parse_plan_tolerates_code_fences() {
        let text = "好的\n```json\n{\"intent\":\"query\",\"sql\":\"select 1\",\"summary\":\"ok\"}\n```";
        let plan = parse_plan(text).expect("should parse");
        assert_eq!(plan.sql, "select 1");
        assert_eq!(plan.intent, "query");
    }

    #[test]
    fn chart_requires_existing_columns() {
        let spec = ChartSpec {
            kind: "line".to_string(),
            x: "日期".to_string(),
            y: vec![ChartY {
                field: "订单额".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let columns = vec!["日期".to_string(), "订单额".to_string()];
        assert!(chart_is_valid(&spec, &columns));
        assert!(!chart_is_valid(&spec, &["日期".to_string()]));
    }

    #[test]
    fn normalize_chart_fixes_case_and_rejects_unknown_type() {
        let spec = ChartSpec {
            kind: "waterfall".to_string(),
            title: String::new(),
            x: "month".to_string(),
            y: vec![ChartY {
                field: "total".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let value = normalize_chart(spec, &["Month".to_string(), "Total".to_string()]);
        assert_eq!(value["type"], "bar");
        assert_eq!(value["x"], "Month");
        assert_eq!(value["y"][0]["field"], "Total");
        assert_eq!(value["title"], "查询结果");
    }

    #[test]
    fn parse_narrated_tolerates_fences_and_prose() {
        let text = "好的\n```json\n{\"summary\":\"华东 1,284 万\",\"insights\":[\"环比 +12%\"],\"followups\":[]}\n```";
        let narrated = parse_narrated(text).expect("should parse");
        assert_eq!(narrated.summary, "华东 1,284 万");
        assert_eq!(narrated.insights, vec!["环比 +12%".to_string()]);
        assert!(narrated.followups.is_empty());
    }

    #[test]
    fn narrate_prompt_keeps_only_head_rows() {
        let rows: Vec<Value> = (0..100).map(|i| json!({"n": i, "note": "x"})).collect();
        let payload = json!({
            "columns": ["n", "note"],
            "columnTypes": ["int", "varchar"],
            "rows": rows,
            "rowCount": 100,
        });
        let prompt = narrate_prompt("有多少行", "select n, note from t", &payload);
        assert!(prompt.contains("【结果列】n(int)、note(varchar)"));
        assert!(prompt.contains("共 100 行"));
        assert!(prompt.contains("节选的前 50 行"));
        // 只带前 NARRATE_ROWS 行：第 50 行在、第 100 行不在
        assert!(prompt.contains("{\"n\":49"));
        assert!(!prompt.contains("{\"n\":50"));
    }

    #[test]
    fn narrate_prompt_stops_on_char_budget() {
        // 宽表长文本：行数远没到上限，靠字符预算收手，且至少保留 1 行
        let rows: Vec<Value> = (0..40).map(|i| json!({"n": i, "txt": "x".repeat(500)})).collect();
        let payload = json!({
            "columns": ["n", "txt"],
            "rows": rows,
            "rowCount": 40,
        });
        let prompt = narrate_prompt("q", "select 1", &payload);
        assert!(prompt.contains("节选的前"), "应标注为节选");
        assert!(prompt.contains("{\"n\":0"), "第一行必须在");
        assert!(!prompt.contains("{\"n\":39"), "超出预算的行不该被带上");
    }

    #[test]
    fn narrate_prompt_marks_small_result_as_complete() {
        // 32 行的小结果要全给，否则模型只能说「已展示的 N 种」（真机踩过）
        let rows: Vec<Value> = (0..32).map(|i| json!({"类型": format!("t{i}"), "记录数": 1})).collect();
        let payload = json!({
            "columns": ["类型", "记录数"],
            "columnTypes": ["varchar", "bigint"],
            "rows": rows,
            "rowCount": 32,
        });
        let prompt = narrate_prompt("q", "select 1", &payload);
        assert!(prompt.contains("共 32 行，全部如下"), "{prompt}");
        assert!(prompt.contains("{\"类型\":\"t31\""));
    }

    #[test]
    fn narrate_prompt_empty_without_columns() {
        assert!(narrate_prompt("q", "select 1", &json!({"rows": []})).is_empty());
    }

    #[test]
    fn local_summary_speaks_numbers() {
        let one = json!({
            "columns": ["订单额"], "columnTypes": ["decimal"],
            "rows": [{"订单额": 1284000}], "rowCount": 1,
        });
        assert_eq!(
            local_summary(&one, &["订单额".to_string()]),
            "当前口径下订单额 为 1284000。"
        );

        let many = json!({
            "columns": ["区域", "订单额"], "columnTypes": ["varchar", "decimal"],
            "rows": [
                {"区域": "华东", "订单额": 800},
                {"区域": "华北", "订单额": 200},
            ],
            "rowCount": 2,
        });
        let text = local_summary(&many, &["区域".to_string(), "订单额".to_string()]);
        assert!(text.contains("华东"), "{text}");
        assert!(text.contains("1000"), "{text}");

        let empty = json!({"columns": ["a"], "rows": [], "rowCount": 0});
        assert_eq!(local_summary(&empty, &["a".to_string()]), "没有符合条件的数据。");
    }
}
