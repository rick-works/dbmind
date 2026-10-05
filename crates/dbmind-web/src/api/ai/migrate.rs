//! 一次性把老版本散在 `~/.dbmind` 下的 json 状态导进主库。
//!
//! ## 为什么要有它
//!
//! 老版本把这些状态写成独立文件（`ai-config.json` / `ai-usage.json` / `ai-audit.log` / …）。
//! 现在全部收进 `dbmind.db`：可事务、可查询、备份时不会漏。升级上来的机器上，
//! 那些老文件还在 —— **不能当它们不存在**，那等于把人家配好的模型、账本、审计记录丢了。
//!
//! ## 三条规矩
//!
//! 1. **幂等**：只有当「库里没有这份数据」且「老文件还在」时才导，多跑几次不会翻倍。
//! 2. **导完把老文件挪走**（移到 `backups/legacy-json/`）。留着它一定会出这种 bug：
//!    用户在新界面上删掉一条，重启又被老文件"复活" —— 而且看起来像灵异事件。
//! 3. **一样失败不牵连别样**：各自独立 try。最坏情况是某一样没迁过来，程序照常跑，
//!    用户看到的是"那部分回到了默认值"，不是整个服务起不来。
//!
//! 老的 json 全部挪完之后，`~/.dbmind` 顶层就只剩 `dbmind.db` 与各目录了。

use dbmind_core::{AiUsageRow, Store};
use serde_json::Value;
use std::path::Path;

/// 跑一遍迁移，返回**实际迁移的动作数**（0 = 没有老文件要迁，或已经迁过）。
pub fn import_legacy() -> usize {
    let Some(store) = dbmind_core::global_store() else {
        return 0;
    };
    let home = dbmind_core::paths::home_dir();
    let mut moved = 0usize;
    moved += import_settings(&store, &home);
    moved += import_usage(&store, &home);
    moved += import_audit(&store, &home);
    moved += import_knowledge(&store, &home);
    moved += import_quality(&store, &home);
    moved += import_knowledge_bases(&store, &home);
    moved
}

/// `ai-knowledge-bases/` 目录整棵 → `kb_list` / `kb_config` / `kb_docs` / `kb_vectors`。
///
/// 这一处与别处不同：老形态是**一棵目录树**（一库一目录、一文档一文件），
/// 所以交给 kb 模块自己的 import_directory 去遍历（它才认识那些结构体）。
fn import_knowledge_bases(store: &Store, home: &Path) -> usize {
    let root = home.join("ai-knowledge-bases");
    if !root.is_dir() {
        return 0;
    }
    if !store.kb_infos().unwrap_or_default().is_empty() {
        return 0;
    }
    match super::kb::import_directory(&root) {
        Ok(count) if count > 0 => {
            if archive_dir(&root) {
                tracing::info!(items = count, "知识库目录已从 ai-knowledge-bases/ 迁入主库");
            }
            1
        }
        _ => 0,
    }
}

/// 目录版的归档：整棵挪走（`archive` 只管单个文件）。
fn archive_dir(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let dir = dbmind_core::paths::home_dir().join("backups").join("legacy-json");
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    std::fs::rename(path, dir.join(name)).is_ok()
}

/// `ai-knowledge.json` → `ai_glossary` + `ai_examples`。
fn import_knowledge(store: &Store, home: &Path) -> usize {
    let path = home.join("ai-knowledge.json");
    if !path.is_file() {
        return 0;
    }
    let already = !store.ai_glossary().unwrap_or_default().is_empty()
        || !store.ai_examples().unwrap_or_default().is_empty();
    if already {
        return 0;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let Ok(doc) = serde_json::from_str::<Value>(&text) else {
        return 0;
    };
    // 复用模块自己的写入路径：字段映射只有一份，导入与日常保存不会走两套逻辑
    if super::knowledge::save(&doc).is_err() {
        return 0;
    }
    if archive(&path) {
        tracing::info!("团队知识已从 ai-knowledge.json 迁入主库");
    }
    1
}

/// `ai-quality-rules.json` → `ai_quality_rules`。
fn import_quality(store: &Store, home: &Path) -> usize {
    let path = home.join("ai-quality-rules.json");
    if !path.is_file() {
        return 0;
    }
    if !store.ai_quality_rules().unwrap_or_default().is_empty() {
        return 0;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let Ok(doc) = serde_json::from_str::<Value>(&text) else {
        return 0;
    };
    let tables = doc.as_object().map(|map| map.len()).unwrap_or(0);
    if tables == 0 {
        return 0;
    }
    if super::quality::save_store(&doc).is_err() {
        return 0;
    }
    if archive(&path) {
        tracing::info!(tables = tables, "质量规则已从 ai-quality-rules.json 迁入主库");
    }
    1
}

/// `ai-config.json` → `ai_settings` + `ai_models`。
fn import_settings(store: &Store, home: &Path) -> usize {
    let path = home.join("ai-config.json");
    if !path.is_file() {
        return 0;
    }
    // 库里已经有设置 → 说明迁过了（或用户在新版本里重新配过），老文件不该再覆盖它
    if !matches!(store.ai_settings(), Ok(None)) {
        return 0;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let Ok(settings) = serde_json::from_str::<super::config::AiSettings>(&text) else {
        return 0;
    };
    let row = super::config::to_row(&settings);
    let count = row.models.len();
    if store.save_ai_settings(&row).is_err() {
        return 0;
    }
    if archive(&path) {
        tracing::info!(models = count, "AI 设置已从 ai-config.json 迁入主库");
    }
    1
}

/// `ai-usage.json` → `ai_usage_days` + `ai_usage_models`。
fn import_usage(store: &Store, home: &Path) -> usize {
    let path = home.join("ai-usage.json");
    if !path.is_file() {
        return 0;
    }
    // 空串当"从最早的一天开始"用：日期是 YYYY-MM-DD，任何真实日期都大于空串
    if !store.ai_usage_days("").unwrap_or_default().is_empty() {
        return 0;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let Ok(doc) = serde_json::from_str::<Value>(&text) else {
        return 0;
    };

    let mut days: Vec<(String, u64)> = Vec::new();
    if let Some(map) = doc.get("days").and_then(Value::as_object) {
        for (day, entry) in map {
            days.push((day.clone(), entry.get("calls").and_then(Value::as_u64).unwrap_or(0)));
        }
    }

    let mut models: Vec<AiUsageRow> = Vec::new();
    if let Some(map) = doc.get("models").and_then(Value::as_object) {
        for (model_key, bucket) in map {
            let Some(by_day) = bucket.as_object() else {
                continue;
            };
            for (day, entry) in by_day {
                let number = |key: &str| entry.get(key).and_then(Value::as_u64).unwrap_or(0);
                models.push(AiUsageRow {
                    model_key: model_key.clone(),
                    day: day.clone(),
                    calls: number("calls"),
                    prompt_tokens: number("promptTokens"),
                    completion_tokens: number("completionTokens"),
                    total_tokens: number("totalTokens"),
                });
            }
        }
    }

    if days.is_empty() && models.is_empty() {
        return 0;
    }
    if store.import_ai_usage(&days, &models).is_err() {
        return 0;
    }
    if archive(&path) {
        tracing::info!(days = days.len(), models = models.len(), "用量账本已从 ai-usage.json 迁入主库");
    }
    1
}

/// `ai-audit.log`（JSONL）→ `ai_audit`。逐行解析：坏行跳过，不让一行脏数据废掉整个文件。
fn import_audit(store: &Store, home: &Path) -> usize {
    let path = home.join("ai-audit.log");
    if !path.is_file() {
        return 0;
    }
    if store.ai_audit_count().unwrap_or(0) > 0 {
        return 0;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let mut count = 0usize;
    for line in text.lines() {
        let Ok(Value::Object(entry)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("legacy");
        let prompt = entry.get("prompt").and_then(Value::as_str).unwrap_or("");
        if store.insert_ai_audit(kind, prompt).is_ok() {
            count += 1;
        }
    }
    if count == 0 {
        return 0;
    }
    if archive(&path) {
        tracing::info!(rows = count, "审计日志已从 ai-audit.log 迁入主库");
    }
    1
}

/// 把老文件挪到 `backups/legacy-json/`：留个底（万一迁移逻辑有 bug 还能人工看），
/// 但它不再是"当前存储"，不会被再次读到。
fn archive(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let dir = dbmind_core::paths::home_dir().join("backups").join("legacy-json");
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let target = dir.join(name);
    if std::fs::rename(path, &target).is_ok() {
        return true;
    }
    // 跨卷时 rename 会失败（比如数据目录被指到另一个盘）：退化成复制 + 删源
    if std::fs::copy(path, &target).is_ok() {
        return std::fs::remove_file(path).is_ok();
    }
    false
}
