//! 提示词模板：内置 21 个，用户可在 `<home>/prompts/` 里覆盖。
//!
//! ## 文案：能省的字符都省，**输出契约一个字不动**
//!
//! 提示词就是这套 AI 功能的「业务逻辑」：`sql.fix` 约定输出「错误原因：/修复SQL：」两行、
//! `command.parse` 要求严格 JSON、`kb.*` 系列要求特定 JSON 字段 —— 前端是**按这些约定解析**的。
//! 所以压缩的边界很清楚：
//!
//! - **能压**：客套话、复述、同一条规则在两处各写一遍、以及「第 1 条」这类排版开销；
//! - **不能压**：输出格式、动作名清单、JSON 字段、方言差异（`IFNULL/LIMIT` vs `NVL/ROWNUM`）、
//!   「不确定就说不知道、不编表名字段」这类防幻觉约束 —— 它们是效果的来源，删了就是另一种毛病。
//!
//! 每一条 token 都要付钱、而且 prompt 每次请求都重发：默认人格从 1549 B 压到 1069 B、
//! `sql.fix` 的系统提示从 823 B 压到 579 B（规则一条没删）。
//!
//! ## 四件事
//!
//! 1. **首次启动把内置模板落到用户目录**：用户能看到、能改。
//! 2. **升级只接管「用户没改过」的那份**（见 [`release_defaults`]）：靠内容指纹判断。
//!    没有这一步，改好的内置文案永远到不了老用户机器上 —— 他们目录里躺着的还是旧版。
//! 3. **`{{name}}` 变量严格校验**：缺变量或模板里残留占位符都直接报错。
//!    悄悄渲染出一个带 `{{sql}}` 字样的提示词发给模型，等于让它对着占位符写答案。
//! 4. **不做内存缓存**：模板文件很小，每次读盘换来的是「改完立刻生效」。

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

/// 首次使用把内置模板写到用户目录；升级时只覆盖「用户没改过」的那些。
pub(crate) fn release_defaults() -> XResult<PathBuf> {
    let dir = user_dir();
    dbmind_core::paths::ensure_dir(&dir)?;
    let upgraded = release_into(&dir)?;
    if upgraded > 0 {
        tracing::info!(upgraded, "提示词模板已升级到内置新版（用户自定义的模板不动）");
    }
    Ok(dir)
}

/// 往 `dir` 释放内置模板，返回**这次升级掉的份数**（0 = 只补了缺失的 / 全都已是最新）。
fn release_into(dir: &std::path::Path) -> XResult<usize> {
    let mut manifest = read_manifest(dir);
    let mut upgraded = 0usize;
    for (id, text) in BUILTIN {
        let file = dir.join(format!("{id}.md"));
        let shipped = fingerprint(text);
        let existing = std::fs::read_to_string(&file).ok();
        let write = match &existing {
            None => true,
            Some(old) => {
                let old_fp = fingerprint(old);
                if old_fp == shipped {
                    // 已经是最新内置版：只补一条记录，不重写文件
                    manifest.insert((*id).to_string(), shipped);
                    continue;
                }
                // 用户没动过 ⇒ 可以升级；动过 ⇒ 从此不再接管
                let ours = should_upgrade(old, shipped, manifest.get(*id).copied());
                if !ours {
                    manifest.remove(*id);
                }
                ours
            }
        };
        if write {
            std::fs::write(&file, text).map_err(|e| {
                XError::internal(format!("写入默认提示词 {} 失败：{e}", file.display()))
            })?;
            manifest.insert((*id).to_string(), shipped);
            if existing.is_some() {
                upgraded += 1;
            }
        }
    }
    write_manifest(dir, &manifest);
    Ok(upgraded)
}

/// 这份用户目录里的模板能不能被内置新版覆盖？
///
/// 判据只有一条：**它是否还是我们上次写进去的那份**（内容指纹对得上 manifest 记的那版，
/// 或者是发行过的历史版本）。对不上就是用户自己改的 —— 一个字都不碰。
fn should_upgrade(existing: &str, shipped: u64, known: Option<u64>) -> bool {
    let current = fingerprint(existing);
    if current == shipped {
        return false;
    }
    known == Some(current) || LEGACY.contains(&current)
}

/// 模板指纹：**只看内容，不看行尾**（同一份模板在 Windows/Linux 上的 CRLF/LF 不该被当成两版）。
///
/// 为什么需要指纹：用户目录里的 `.md` 是首次启动时我们从内置释放的。此后每次改内置文案，
/// 都必须区分「这份还是我们写的」和「用户自己改的」——前者该升级（否则压缩永远到不了老用户），
/// 后者一个字都不能碰。用内容指纹而不是时间戳/版本号，是因为用户可能只改了时间戳。
fn fingerprint(text: &str) -> u64 {
    // FNV-1a 64：十行实现、不引依赖，只用来判等
    const OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01B3;
    let mut hash = OFFSET;
    for byte in text.replace("\r\n", "\n").trim_end().as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(PRIME);
    }
    hash
}

/// 历史版本（压缩前的官方文案）指纹。
///
/// 老用户目录里躺着的是这些内容、且当时还没有 manifest，无从判断「改没改过」——
/// 于是把官方发行过的每一版都记在这里：命中即官方原文（可安全升级），不命中即用户自己写的。
const LEGACY: &[u64] = &[
    0xc06b_07c1_3076_9147, // command.parse.user
    0x73f2_ce16_dea8_1c99, // common.default-system
    0x4768_481d_1dd9_18b1, // sql.explain.user
    0xa66a_b4dc_e289_96dc, // sql.fix.system
    0xe18d_a3f5_2994_3801, // sql.fix.user
    0xfff8_3147_266d_863a, // sql.optimize.system
    0xda1d_9d05_1ff7_489c, // sql.optimize.user
    0x1ec2_8ac0_8e15_5c96, // sql.rewrite.user
];

/// 「内置模板最后一次写到用户目录时的指纹」。下次再改内置文案，靠它判断要不要升级。
fn manifest_path(dir: &std::path::Path) -> PathBuf {
    dir.join(".builtin.json")
}

fn read_manifest(dir: &std::path::Path) -> BTreeMap<String, u64> {
    std::fs::read_to_string(manifest_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_manifest(dir: &std::path::Path, manifest: &BTreeMap<String, u64>) {
    // 写不了就算了：下一次最多把「已是最新」那一支再跑一遍，不影响正确性
    if let Ok(text) = serde_json::to_string_pretty(manifest) {
        let _ = std::fs::write(manifest_path(dir), text);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dbmind-prompts-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fingerprint_ignores_line_endings() {
        assert_eq!(fingerprint("a\r\nb\r\n"), fingerprint("a\nb\n"));
        assert_eq!(fingerprint("a\nb\n\n"), fingerprint("a\nb"), "尾部空行不算改动");
        assert_ne!(fingerprint("a\nb"), fingerprint("a\nc"));
    }

    #[test]
    fn upgrade_only_touches_untouched_templates() {
        let shipped = fingerprint("新版文案");
        // 用户自己改的：不动
        assert!(!should_upgrade("我自己写的", shipped, None));
        // 上次就是我们写进去的那版（manifest 记着）：升级
        assert!(should_upgrade("旧版文案", shipped, Some(fingerprint("旧版文案"))));
        // 已经是新版：不用动
        assert!(!should_upgrade("新版文案", shipped, None));
    }

    #[test]
    fn release_fills_missing_but_never_overwrites_user_edits() {
        let dir = temp_dir("release");
        assert_eq!(release_into(&dir).unwrap(), 0, "首次释放不算升级");
        let file = dir.join("common.default-system.md");
        assert!(file.is_file());

        std::fs::write(&file, "我的自定义人格").unwrap();
        assert_eq!(release_into(&dir).unwrap(), 0);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "我的自定义人格",
            "用户改过的模板一个字都不能动"
        );

        // 把 manifest 删掉也不能变成「无脑覆盖」：没有指纹就无从判断，保守不动
        let _ = std::fs::remove_file(manifest_path(&dir));
        assert_eq!(release_into(&dir).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "我的自定义人格");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
