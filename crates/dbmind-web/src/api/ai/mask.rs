//! **AI 出站脱敏**：把指定的表 / 字段在发给模型之前换成不可逆掩码。
//!
//! ## 这里解决的是哪一件事
//!
//! 不是「把数据库里的列加密」——那要改写入链路、管密钥，还会让结果表格、导出、
//! 同步全部看不懂数据。用户真正要的是：**同样的库、同样的界面，只是别把敏感字段
//! 的明文送进模型**。所以脱敏只作用于「出站流量」的两处：
//!
//! 1. `context::build` 里每张展开表的 **1 行样例值**（那是真实数据）；
//! 2. `agent::run` 的只读工具结果（智能体自己查回来的行）。
//!
//! 两处必须共用这套判定 —— 只堵第一处，模型换个姿势自己 `select` 一次就绕过去了。
//!
//! ## 三个刻意的取舍
//!
//! 1. **不可逆**。掩码后模型仍能看出「这是手机号 / 这是邮箱 / 这是枚举」，
//!    但永远拿不回原值。可逆加密对 AI 没有增益（它看密文一样能写 SQL），却引入
//!    「密钥丢了数据就没了」的风险，所以不做。
//! 2. **表名 / 列名 / 表注释不脱敏**。脱了模型就无从判断该查哪张表、哪个字段，
//!    功能直接失效。代价要讲清楚：表注释里如果写了敏感信息，仍会发出去。
//! 3. **用户在 SQL 里手写的字面量不管**。`where phone = '138…'` 是用户自己的原文，
//!    工具没有可靠办法猜哪一段是敏感值，硬猜只会改坏 SQL。设置页里明写这一条。
//!
//! ## 规则怎么匹配
//!
//! 一条规则 = 连接（空=全部）/ 库（空=全部）/ 表（空=任意表）/ 列（必填，支持 `*` 通配）。
//! 「表为空 = 任意表」是给智能体工具结果用的：一轮 JOIN 下来拿不到可靠表名，
//! 靠列名兜住才是实际可行的做法。表钉死的规则在不知道表名时**不放行**（宁可漏脱敏一次
//! 也不误伤别的表？反过来 —— 这里选的是「不知道就不脱敏」，因为钉表规则表达的是
//! 「只有这张表的这个列敏感」，套到别的表上会把正常字段也打码，反而让模型失明）。

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::blocking;
use crate::api::error::XResult;
use crate::AppState;

/// 设置键。存 `{ enabled, rules[] }` 的 JSON，跟着设置一起迁移/备份。
pub const KEY: &str = "ai.maskRules";

/// 一条脱敏规则。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskRule {
    /// 空 = 所有连接
    #[serde(default)]
    pub connection_id: Option<String>,
    /// 空 = 所有库
    #[serde(default)]
    pub database: Option<String>,
    /// 空 = 任意表（智能体工具结果靠这一档兜底）
    #[serde(default)]
    pub table: Option<String>,
    /// 列名，支持 `*`（全部）、`*_phone`（后缀）、`phone*`（前缀）
    pub column: String,
    /// 备注（为什么脱敏这一列），只给人看
    #[serde(default)]
    pub note: Option<String>,
}

/// 整个脱敏配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<MaskRule>,
}

fn clean(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// 名字比较：忽略大小写与首尾空白。
///
/// 不同引擎对标识符大小写态度不同（MySQL 列名原样返回、Oracle 全大写回来），
/// 用户填 `phone` 而列名是 `PHONE` 时必须能命中 —— 否则「设置了规则却没生效」
/// 是最难排查的一类问题。
fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// 列 / 表的通配匹配：`*`、`*后缀`、`前缀*`、等值。
fn pattern_match(pattern: &str, name: &str) -> bool {
    let want = pattern.trim().to_ascii_lowercase();
    if want.is_empty() || want == "*" {
        return true;
    }
    let got = name.trim().to_ascii_lowercase();
    if let Some(suffix) = want.strip_prefix('*') {
        return !suffix.is_empty() && got.ends_with(suffix);
    }
    if let Some(prefix) = want.strip_suffix('*') {
        return !prefix.is_empty() && got.starts_with(prefix);
    }
    want == got
}

impl MaskRule {
    /// 这条规则是否命中「某连接 / 某库 / 某表（可未知）的某一列」。
    pub fn hits(&self, conn: &str, database: &str, table: Option<&str>, column: &str) -> bool {
        if let Some(want) = clean(self.connection_id.as_deref()) {
            if !same_name(want, conn) {
                return false;
            }
        }
        if let Some(want) = clean(self.database.as_deref()) {
            if !same_name(want, database) {
                return false;
            }
        }
        match (clean(self.table.as_deref()), table) {
            // 规则钉了表：这次也知道表名 → 比表名；不知道表名 → 不命中
            (Some(want), Some(got)) => {
                if !pattern_match(want, got) {
                    return false;
                }
            }
            (Some(_), None) => return false,
            // 表为空的规则：任意表都算，智能体工具结果走这一档
            (None, _) => {}
        }
        pattern_match(&self.column, column)
    }
}

impl MaskConfig {
    /// 这一列要不要脱敏。
    pub fn hits(&self, conn: &str, database: &str, table: Option<&str>, column: &str) -> bool {
        self.enabled && self.rules.iter().any(|rule| rule.hits(conn, database, table, column))
    }
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

/// 把单个值换成不可逆掩码。
///
/// 保留「形状」是有意的：模型靠形状判断这列是手机号、邮箱还是枚举，
/// 抹成一律的 `***` 会让它把 `PAID` 当成数字，写出来的 SQL 反而更容易错。
/// 但形状只保留**类型特征**（前 3 位、域名、长度），不保留可还原的信息。
pub fn mask_value(raw: &str) -> String {
    let text = raw.trim();
    if text.is_empty() {
        return String::new();
    }
    // 邮箱：a***@example.com（域名保留，否则模型判断不出这是邮箱）
    if let Some((user, domain)) = text.split_once('@') {
        if !user.is_empty() && domain.contains('.') {
            let head: String = user.chars().take(1).collect();
            return format!("{head}***@{domain}");
        }
    }
    let len = text.chars().count();
    // 手机号（11 位、1 开头）：138****8000 —— 前 3 后 4 是运营商号段与尾号，
    // 不足以定位到人，但足以让模型知道「这是手机号」
    if len == 11 && text.starts_with('1') && all_digits(text) {
        let head: String = text.chars().take(3).collect();
        let tail: String = text.chars().skip(7).collect();
        return format!("{head}****{tail}");
    }
    // 身份证（18 位，末位可能是 X）：110***********12X
    if len == 18 && text.chars().take(17).all(|c| c.is_ascii_digit()) {
        let head: String = text.chars().take(3).collect();
        let tail: String = text.chars().skip(14).collect();
        return format!("{head}***********{tail}");
    }
    // 卡片 / 账号这类长数字：只留后 2 位
    if len >= 6 && all_digits(text) {
        let tail: String = text.chars().skip(len - 2).collect();
        return format!("****{tail}");
    }
    if len <= 2 {
        return "***".to_string();
    }
    let head: String = text.chars().take(1).collect();
    format!("{head}***")
}

/// 读配置。解析失败按「未启用」处理 —— 一条坏规则不该让整个 AI 面板报错。
pub async fn load(state: &AppState) -> MaskConfig {
    let engine = state.engine();
    let raw = match blocking(move || engine.get_setting(KEY)).await {
        Ok(value) => value,
        Err(_) => None,
    };
    raw.as_deref()
        .and_then(|text| serde_json::from_str::<MaskConfig>(text).ok())
        .unwrap_or_default()
}

/// 写配置。
pub async fn save(state: &AppState, config: &MaskConfig) -> XResult<()> {
    let text = serde_json::to_string(config)?;
    let engine = state.engine();
    blocking(move || engine.set_setting(KEY, &text)).await?;
    Ok(())
}

/// `GET /api/ai/mask-rules`
pub async fn get_rules(State(state): State<AppState>) -> XResult<Json<Value>> {
    let config = load(&state).await;
    Ok(Json(json!({
        "enabled": config.enabled,
        "rules": config.rules,
        "count": config.rules.len(),
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesBody {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<MaskRule>,
}

/// `PUT /api/ai/mask-rules` —— 整份覆盖（规则条数不多，前端一次全量提交最不容易出错）。
pub async fn put_rules(
    State(state): State<AppState>,
    Json(body): Json<RulesBody>,
) -> XResult<Json<Value>> {
    let rules: Vec<MaskRule> = body
        .rules
        .into_iter()
        // 列名是唯一必填项：空列名的规则永远匹配不到东西，存下来只会让人以为生效了
        .filter(|rule| !rule.column.trim().is_empty())
        .map(|rule| MaskRule {
            connection_id: clean(rule.connection_id.as_deref()).map(str::to_string),
            database: clean(rule.database.as_deref()).map(str::to_string),
            table: clean(rule.table.as_deref()).map(str::to_string),
            column: rule.column.trim().to_string(),
            note: clean(rule.note.as_deref()).map(str::to_string),
        })
        .collect();
    let config = MaskConfig {
        enabled: body.enabled,
        rules,
    };
    save(&state, &config).await?;
    tracing::info!(
        enabled = config.enabled,
        count = config.rules.len(),
        "AI 字段脱敏规则已更新"
    );
    Ok(Json(json!({
        "success": true,
        "enabled": config.enabled,
        "count": config.rules.len(),
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewBody {
    #[serde(default)]
    pub value: String,
}

/// `POST /api/ai/mask/preview` —— 设置页的实时预览。
///
/// 走接口而不是在前端再写一份掩码逻辑：两套实现迟早会漂移，那时「预览里好好的、
/// 实际发出去是明文」就成了最难查的 bug。
pub async fn preview(Json(body): Json<PreviewBody>) -> Json<Value> {
    Json(json!({
        "input": body.value,
        "result": mask_value(&body.value),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(conn: Option<&str>, db: Option<&str>, table: Option<&str>, column: &str) -> MaskRule {
        MaskRule {
            connection_id: conn.map(str::to_string),
            database: db.map(str::to_string),
            table: table.map(str::to_string),
            column: column.to_string(),
            note: None,
        }
    }

    #[test]
    fn mask_keeps_shape_without_leaking() {
        assert_eq!(mask_value("13812348000"), "138****8000");
        assert_eq!(mask_value("alice@example.com"), "a***@example.com");
        assert_eq!(mask_value("11010119900307123X"), "110***********123X");
        assert_eq!(mask_value("6222021234567890123"), "****23");
        assert_eq!(mask_value("张"), "***");
        assert_eq!(mask_value("张三丰"), "张***");
        assert_eq!(mask_value(""), "");
        assert_eq!(mask_value("NULL"), "N***");
    }

    #[test]
    fn table_rules_do_not_leak_to_unknown_tables() {
        let config = MaskConfig {
            enabled: true,
            rules: vec![rule(None, None, Some("users"), "phone")],
        };
        // 表名已知且命中
        assert!(config.hits("conn_1", "demo", Some("users"), "phone"));
        // 表名大小写不同也算命中（Oracle 回来是全大写）
        assert!(config.hits("conn_1", "demo", Some("USERS"), "PHONE"));
        // 表名未知（智能体工具结果）→ 钉表规则不放行
        assert!(!config.hits("conn_1", "demo", None, "phone"));
        // 别的表不命中
        assert!(!config.hits("conn_1", "demo", Some("orders"), "phone"));
    }

    #[test]
    fn any_table_rules_cover_tool_results() {
        let config = MaskConfig {
            enabled: true,
            rules: vec![
                rule(None, None, None, "*_phone"),
                rule(None, None, None, "id_card"),
                rule(None, Some("demo"), None, "email"),
            ],
        };
        assert!(config.hits("conn_1", "demo", None, "user_phone"));
        assert!(config.hits("conn_1", "demo", Some("orders"), "ID_CARD"));
        assert!(config.hits("conn_1", "demo", None, "email"));
        // 库限定生效
        assert!(!config.hits("conn_1", "prod", None, "email"));
        // 不相关列不命中
        assert!(!config.hits("conn_1", "demo", None, "status"));
    }

    #[test]
    fn connection_scope_and_switch() {
        let mut config = MaskConfig {
            enabled: true,
            rules: vec![rule(Some("conn_a"), None, None, "phone")],
        };
        assert!(config.hits("conn_a", "demo", None, "phone"));
        assert!(!config.hits("conn_b", "demo", None, "phone"));
        // 总开关关掉就一律不脱敏
        config.enabled = false;
        assert!(!config.hits("conn_a", "demo", None, "phone"));
    }
}
