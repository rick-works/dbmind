//! 提示词模板：**内置 21 个，逐字取自上游**，用户可在 `<home>/prompts/` 里覆盖。
//!
//! ## 为什么模板要跟原版一模一样
//!
//! 提示词就是这套 AI 功能的「业务逻辑」：`sql.fix` 约定输出「错误原因：/修复SQL：」两行、
//! `command.parse` 要求严格 JSON、`kb.*` 系列要求特定 JSON 字段 —— 前端是**按这些约定解析**的。
//! 自己重写一版措辞「差不多」的提示词，模型输出就会时不时不符合约定，
//! 表现是「AI 偶尔不灵」，而根因在提示词 —— 那种问题最难查。所以这里直接沿用原文。
//!
//! ## 三件事
//!
//! 1. **首次启动把内置模板落到用户目录**：用户能看到、能改（上游也是这么做的）。
//!    改了之后以用户目录那份为准。
//! 2. **`{{name}}` 变量严格校验**：缺变量或模板里残留占位符都直接报错。
//!    悄悄渲染出一个带 `{{sql}}` 字样的提示词发给模型，等于让它对着占位符写答案。
//! 3. **不做内存缓存**：模板文件很小，每次读盘换来的是「改完立刻生效」——
//!    免得出现「改了提示词但没重启所以没生效」这种浪费半小时的排查。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::api::error::{XError, XResult};

/// 内置模板（名称 → 内容）。名称与上游的 `PromptIds` 一一对应。
const BUILTIN: &[(&str, &str)] = &[
    (
        "common.default-system",
        include_str!("../../../prompts/common.default-system.md"),
    ),
    ("common.brevity", include_str!("../../../prompts/common.brevity.md")),
    (
        "sql.explain.system",
        include_str!("../../../prompts/sql.explain.system.md"),
    ),
    (
        "sql.explain.user",
        include_str!("../../../prompts/sql.explain.user.md"),
    ),
    (
        "sql.optimize.system",
        include_str!("../../../prompts/sql.optimize.system.md"),
    ),
    (
        "sql.optimize.user",
        include_str!("../../../prompts/sql.optimize.user.md"),
    ),
    ("sql.fix.system", include_str!("../../../prompts/sql.fix.system.md")),
    ("sql.fix.user", include_str!("../../../prompts/sql.fix.user.md")),
    (
        "sql.rewrite.system",
        include_str!("../../../prompts/sql.rewrite.system.md"),
    ),
    (
        "sql.rewrite.user",
        include_str!("../../../prompts/sql.rewrite.user.md"),
    ),
    (
        "command.parse.system",
        include_str!("../../../prompts/command.parse.system.md"),
    ),
    (
        "command.parse.user",
        include_str!("../../../prompts/command.parse.user.md"),
    ),
    ("kb.inspect.system", include_str!("../../../prompts/kb.inspect.system.md")),
    ("kb.inspect.user", include_str!("../../../prompts/kb.inspect.user.md")),
    ("kb.polish.system", include_str!("../../../prompts/kb.polish.system.md")),
    ("kb.polish.user", include_str!("../../../prompts/kb.polish.user.md")),
    ("kb.plan.system", include_str!("../../../prompts/kb.plan.system.md")),
    ("kb.plan.user", include_str!("../../../prompts/kb.plan.user.md")),
    ("kb.outline.system", include_str!("../../../prompts/kb.outline.system.md")),
    ("kb.outline.user", include_str!("../../../prompts/kb.outline.user.md")),
];

fn user_dir() -> PathBuf {
    dbmind_core::paths::home_dir().join("prompts")
}

/// 首次使用把内置模板写到用户目录（已存在的**不覆盖** —— 那可能是用户改过的）。
pub(crate) fn release_defaults() -> XResult<PathBuf> {
    let dir = user_dir();
    dbmind_core::paths::ensure_dir(&dir)?;
    for (id, text) in BUILTIN {
        let file = dir.join(format!("{id}.md"));
        if !file.exists() {
            std::fs::write(&file, text).map_err(|e| {
                XError::internal(format!("写入默认提示词 {} 失败：{e}", file.display()))
            })?;
        }
    }
    Ok(dir)
}

/// 取模板原文：用户目录优先，其次内置。
pub fn text_of(id: &str) -> Option<String> {
    let file = user_dir().join(format!("{id}.md"));
    if let Ok(text) = std::fs::read_to_string(&file) {
        return Some(text);
    }
    BUILTIN
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, text)| (*text).to_string())
}

/// 渲染模板：把 `{{name}}` 换成给定值。
///
/// 缺变量、模板里残留占位符都是**错误**（见文件头第 2 条）。
pub fn render(id: &str, vars: &[(&str, &str)]) -> XResult<String> {
    let template = text_of(id)
        .ok_or_else(|| XError::internal(format!("提示词模板 {id} 不存在")))?;
    let mut out = template;
    for (name, value) in vars {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    // 残留判定要在替换之后：`{{sql}}` 没被替换掉说明调用方忘了传
    if let Some(start) = out.find("{{") {
        let rest = &out[start..];
        let name = rest
            .find("}}")
            .map(|end| rest[2..end].to_string())
            .unwrap_or_else(|| "?".to_string());
        return Err(XError::internal(format!(
            "提示词模板 {id} 里的变量 {{{{{name}}}}} 没有取值"
        )));
    }
    Ok(out)
}

/// `GET /api/ai/prompts` —— 模板清单（界面上的「提示词」页就是列它）。
pub async fn list() -> XResult<serde_json::Value> {
    let dir = release_defaults()?;
    let mut items: Vec<Value> = Vec::new();
    let mut names: BTreeMap<String, PathBuf> = BTreeMap::new();
    for (id, _) in BUILTIN {
        names.insert((*id).to_string(), PathBuf::from(format!("{id}.md")));
    }
    for (id, file) in names {
        let user_file = dir.join(&file);
        let customized = user_file.exists()
            && std::fs::read_to_string(&user_file)
                .map(|text| {
                    BUILTIN
                        .iter()
                        .find(|(name, _)| *name == id)
                        .map(|(_, builtin)| text.trim() != builtin.trim())
                        .unwrap_or(true)
                })
                .unwrap_or(false);
        let text = text_of(&id).unwrap_or_default();
        items.push(json!({
            "id": id,
            "file": user_file.display().to_string(),
            "customized": customized,
            "chars": text.chars().count(),
            "preview": text.lines().next().unwrap_or("").chars().take(80).collect::<String>(),
        }));
    }
    Ok(json!({
        "dir": dir.display().to_string(),
        "items": items,
        "count": items.len(),
    }))
}

/// `POST /api/ai/prompts/reload` —— 重新读盘。
///
/// 本来就没有缓存，所以这个端点的真实作用是「把内置模板重新释放一遍」
/// （用户删掉某个文件后能一键恢复），并把最新清单回给界面。
pub async fn reload() -> XResult<Value> {
    list().await
}
