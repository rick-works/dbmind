//! **知识库**（`/api/ai/kb/*`，16 个端点）：文档入库 → 分块 → 检索 → 注入对话。
//!
//! ## 存储形态（与上游同构，便于互相理解）
//!
//! ```text
//! <home>/ai-knowledge-bases/
//!   index.json                  库列表（每个库的 id/名称/文档数/分块数/字数）
//!   config.json                 全局默认召回与分段配置
//!   <kbId>/config.json          该库的配置（覆盖全局）
//!   <kbId>/docs/<docId>.json    一份文档：原文 + 父块 + 子块
//!   <kbId>/vectors/<docId>.json 该文档的向量（可选；关键词模式不需要）
//! ```
//!
//! 纯文件、纯本地：不引数据库、不引向量库。知识库的规模通常是「几十份制度/手册」，
//! 这个量级下暴力余弦比任何向量库都快（也没有「索引和文件对不上」的经典故障）。
//!
//! ## 检索三种模式
//!
//! - `keyword`：BM25（**默认**）。中文按 2-gram、英文按整词小写 —— 不需要模型、离线可用，
//!   这也是「没配 AI 也能用知识库」的原因；
//! - `vector`：对全部块做余弦（需要 embedding 模型）；
//! - `hybrid`：两路各自 min-max 归一化后按 `hybridWeight` 加权（权重给关键词）。
//!
//! ## 两个刻意的取舍
//!
//! 1. **不持久化倒排索引**：每次检索现算。库小（几千块）时这点开销远小于
//!    「索引文件与文档不同步」带来的排查成本；等库真的很大再谈倒排缓存。
//! 2. **入库是同步的**：写入 + 分块 + （可选）算向量在同一次请求里做完。
//!    知识库入库是「点一下等几秒」，做成分步的异步任务只会把界面搞复杂。

use std::collections::HashSet;

use axum::routing::post;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::ai::{prompts, run_blocking};
use crate::api::error::{XError, XResult};
use crate::api::tasks;
use crate::AppState;

/// 单文档字符上限（200 万字）与单库分块上限。
const MAX_DOC_CHARS: usize = 2_000_000;
const MAX_CHUNKS: usize = 20_000;
/// 详情页最多回多少原文（再长界面也显示不了）。
const MAX_RAW_ECHO: usize = 100_000;

// ------------------------------------------------------------------ 配置

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KbConfig {
    /// `keyword` | `vector` | `hybrid`
    #[serde(default = "d_index_mode")]
    pub index_mode: String,
    #[serde(default = "d_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub min_score: f64,
    /// 混合检索里关键词那一路的权重（0~1）
    #[serde(default = "d_hybrid")]
    pub hybrid_weight: f64,
    /// `general` | `parentChild`
    #[serde(default = "d_chunk_mode")]
    pub chunk_mode: String,
    #[serde(default = "d_target")]
    pub target_chars: usize,
    #[serde(default = "d_overlap")]
    pub overlap_chars: usize,
    #[serde(default = "d_separators")]
    pub separators: Vec<String>,
    #[serde(default = "d_min_chunk")]
    pub min_chunk_chars: usize,
    #[serde(default)]
    pub drop_urls: bool,
    #[serde(default)]
    pub drop_emails: bool,
    #[serde(default)]
    pub embed_model_id: String,
}

fn d_index_mode() -> String {
    "keyword".to_string()
}
fn d_top_k() -> usize {
    4
}
fn d_hybrid() -> f64 {
    0.5
}
fn d_chunk_mode() -> String {
    "general".to_string()
}
fn d_target() -> usize {
    2600
}
fn d_overlap() -> usize {
    130
}
fn d_separators() -> Vec<String> {
    vec!["\n####".to_string()]
}
fn d_min_chunk() -> usize {
    50
}

impl Default for KbConfig {
    fn default() -> Self {
        Self {
            index_mode: d_index_mode(),
            top_k: d_top_k(),
            min_score: 0.0,
            hybrid_weight: d_hybrid(),
            chunk_mode: d_chunk_mode(),
            target_chars: d_target(),
            overlap_chars: d_overlap(),
            separators: d_separators(),
            min_chunk_chars: d_min_chunk(),
            drop_urls: false,
            drop_emails: false,
            embed_model_id: String::new(),
        }
    }
}

impl KbConfig {
    fn target(&self) -> usize {
        self.target_chars.clamp(200, 20_000)
    }

    fn overlap(&self) -> usize {
        // 重叠必须小于目标长度，否则分块永远推进不下去
        self.overlap_chars.min(self.target() / 2)
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct KbInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
    pub doc_count: usize,
    pub chunk_count: usize,
    pub char_count: usize,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub index: usize,
    pub text: String,
    #[serde(default)]
    pub parent_index: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Parent {
    pub index: usize,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KbDoc {
    pub id: String,
    pub title: String,
    pub source: String,
    pub created_at: String,
    pub char_count: usize,
    pub raw: String,
    #[serde(default)]
    pub parents: Vec<Parent>,
    #[serde(default)]
    pub chunks: Vec<Chunk>,
}

#[derive(Serialize, Deserialize, Default)]
struct VectorFile {
    #[serde(default)]
    model: String,
    #[serde(default)]
    dim: usize,
    #[serde(default)]
    vectors: Vec<Vec<f32>>,
}

// ------------------------------------------------------------------ 存储

fn safe_id(id: &str) -> XResult<String> {
    let trimmed = id.trim();
    // id 直接进文件路径：只放行「字母数字下划线短横」，路径穿越无从下手
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(XError::bad_request(format!("非法的知识库/文档 id：{id}")));
    }
    Ok(trimmed.to_string())
}

fn now() -> String {
    chrono::Local::now().to_rfc3339()
}

// 存储层：全部落在主库 `~/.dbmind/dbmind.db` 的 kb_list / kb_config / kb_docs / kb_vectors 四张表里。
// 老版本的 ai-knowledge-bases/ 目录（一库一目录、一文档一文件）由 migrate 模块整棵导入后挪走。
//
// 对外行为一字未变：`list()` 仍带统计、`refresh()` 仍重算统计、配置仍是
// 「单库 → 全局默认 → 内置默认」三级回退，文档仍按 created_at 排序。

fn load_index() -> Vec<KbInfo> {
    let Some(store) = dbmind_core::global_store() else {
        return Vec::new();
    };
    store
        .kb_infos()
        .unwrap_or_default()
        .into_iter()
        .map(|row| KbInfo {
            id: row.id,
            name: row.name,
            description: row.description,
            created_at: row.created_at,
            updated_at: row.updated_at,
            doc_count: row.doc_count as usize,
            chunk_count: row.chunk_count as usize,
            char_count: row.char_count as usize,
        })
        .collect()
}

fn save_index(items: &[KbInfo]) -> XResult<()> {
    let Some(store) = dbmind_core::global_store() else {
        return Err(XError::internal("主库未就绪，知识库索引无法保存"));
    };
    let rows: Vec<dbmind_core::KbInfoRow> = items
        .iter()
        .map(|item| dbmind_core::KbInfoRow {
            id: item.id.clone(),
            name: item.name.clone(),
            description: item.description.clone(),
            created_at: item.created_at.clone(),
            updated_at: item.updated_at.clone(),
            doc_count: item.doc_count as u64,
            chunk_count: item.chunk_count as u64,
            char_count: item.char_count as u64,
        })
        .collect();
    store
        .kb_put_infos(&rows)
        .map_err(|err| XError::internal(format!("写入主库失败：{err}")))
}

/// 配置三级回退：单库 → 全局默认（`kb_id` 为空串那条）→ 内置默认。
fn load_config(kb_id: &str) -> KbConfig {
    let Some(store) = dbmind_core::global_store() else {
        return KbConfig::default();
    };
    for key in [kb_id, ""] {
        if let Ok(Some(text)) = store.kb_get_config(key) {
            if let Ok(config) = serde_json::from_str::<KbConfig>(&text) {
                return config;
            }
        }
    }
    KbConfig::default()
}

fn save_config(kb_id: &str, config: &KbConfig) -> XResult<()> {
    let Some(store) = dbmind_core::global_store() else {
        return Err(XError::internal("主库未就绪，知识库配置无法保存"));
    };
    let text = serde_json::to_string(config).map_err(|e| XError::internal(e.to_string()))?;
    store
        .kb_put_config(kb_id, &text)
        .map_err(|err| XError::internal(format!("写入主库失败：{err}")))
}

/// 主库行 → 文档。`docs_of` 与 `find_doc` 共用这一份映射，
/// 加字段时不会出现"一处带了、另一处漏了"。
fn doc_from_row(row: dbmind_core::KbDocRow) -> KbDoc {
    KbDoc {
        id: row.id,
        title: row.title,
        source: row.source,
        created_at: row.created_at,
        char_count: row.char_count as usize,
        raw: row.raw,
        parents: serde_json::from_str(&row.parents).unwrap_or_default(),
        chunks: serde_json::from_str(&row.chunks).unwrap_or_default(),
    }
}

/// 文档 → 主库行。
fn doc_row(doc: &KbDoc) -> dbmind_core::KbDocRow {
    dbmind_core::KbDocRow {
        id: doc.id.clone(),
        title: doc.title.clone(),
        source: doc.source.clone(),
        created_at: doc.created_at.clone(),
        char_count: doc.char_count as u64,
        raw: doc.raw.clone(),
        parents: serde_json::to_string(&doc.parents).unwrap_or_else(|_| "[]".to_string()),
        chunks: serde_json::to_string(&doc.chunks).unwrap_or_else(|_| "[]".to_string()),
    }
}

fn docs_of(kb_id: &str) -> Vec<KbDoc> {
    let Some(store) = dbmind_core::global_store() else {
        return Vec::new();
    };
    store
        .kb_docs(kb_id)
        .unwrap_or_default()
        .into_iter()
        .map(doc_from_row)
        .collect()
}

fn find_doc(kb_id: &str, doc_id: &str) -> XResult<KbDoc> {
    let Some(store) = dbmind_core::global_store() else {
        return Err(XError::bad_request(format!("文档 {doc_id} 不存在")));
    };
    match store.kb_doc(kb_id, doc_id) {
        Ok(Some(row)) => Ok(doc_from_row(row)),
        Ok(None) => Err(XError::bad_request(format!("文档 {doc_id} 不存在"))),
        Err(err) => Err(XError::internal(format!("读取文档失败：{err}"))),
    }
}

fn save_doc(kb_id: &str, doc: &KbDoc) -> XResult<()> {
    let Some(store) = dbmind_core::global_store() else {
        return Err(XError::internal("主库未就绪，文档无法保存"));
    };
    store
        .kb_put_doc(kb_id, &doc_row(doc))
        .map_err(|err| XError::internal(format!("写入文档失败：{err}")))
}

fn load_vectors(kb_id: &str, doc_id: &str) -> Option<VectorFile> {
    let store = dbmind_core::global_store()?;
    let row = store.kb_vectors(kb_id, doc_id).ok()??;
    Some(VectorFile {
        model: row.model,
        dim: row.dim,
        vectors: row.vectors,
    })
}

fn save_vectors(kb_id: &str, doc_id: &str, vectors: &VectorFile) -> XResult<()> {
    let Some(store) = dbmind_core::global_store() else {
        return Err(XError::internal("主库未就绪，向量无法保存"));
    };
    store
        .kb_put_vectors(kb_id, doc_id, &vectors.model, &vectors.vectors)
        .map_err(|err| XError::internal(format!("写入向量失败：{err}")))
}

/// 重算某个库的统计（文档数 / 分块数 / 字数）并回写索引。
fn refresh(kb_id: &str) -> XResult<KbInfo> {
    let docs = docs_of(kb_id);
    let mut items = load_index();
    let info = items
        .iter_mut()
        .find(|item| item.id == kb_id)
        .ok_or_else(|| XError::bad_request(format!("知识库 {kb_id} 不存在")))?;
    info.doc_count = docs.len();
    info.chunk_count = docs.iter().map(|doc| doc.chunks.len()).sum();
    info.char_count = docs.iter().map(|doc| doc.char_count).sum();
    info.updated_at = now();
    let snapshot = info.clone();
    save_index(&items)?;
    Ok(snapshot)
}

/// 把老版本的 `ai-knowledge-bases/` 目录整棵读进库（一次性迁移用）。
///
/// 目录形态是「一库一目录、一文档一文件」，所以要遍历：
/// `index.json` + `config.json`（全局那份）+ `<kb_id>/{config.json, docs/*.json, vectors/*.json}`。
///
/// 单份文件坏了只跳过它，不让整次迁移失败 —— 最坏是少了几篇文档，而不是一篇都进不来。
pub(crate) fn import_directory(root: &std::path::Path) -> XResult<usize> {
    let mut imported = 0usize;

    if let Ok(text) = std::fs::read_to_string(root.join("index.json")) {
        if let Ok(items) = serde_json::from_str::<Vec<KbInfo>>(&text) {
            imported += items.len();
            save_index(&items)?;
        }
    }
    if let Ok(text) = std::fs::read_to_string(root.join("config.json")) {
        if let Ok(config) = serde_json::from_str::<KbConfig>(&text) {
            // 根目录那份是「全局默认」，库里用空串当键
            save_config("", &config)?;
            imported += 1;
        }
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(imported);
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(kb_id) = dir.file_name().and_then(|name| name.to_str()).map(str::to_string) else {
            continue;
        };
        // 目录名不是合法 id（例如手工放进去的东西）就跳过，不当成知识库
        if safe_id(&kb_id).is_err() {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("config.json")) {
            if let Ok(config) = serde_json::from_str::<KbConfig>(&text) {
                save_config(&kb_id, &config)?;
                imported += 1;
            }
        }
        if let Ok(docs) = std::fs::read_dir(dir.join("docs")) {
            for doc in docs.flatten() {
                if let Ok(text) = std::fs::read_to_string(doc.path()) {
                    if let Ok(parsed) = serde_json::from_str::<KbDoc>(&text) {
                        save_doc(&kb_id, &parsed)?;
                        imported += 1;
                    }
                }
            }
        }
        if let Ok(files) = std::fs::read_dir(dir.join("vectors")) {
            for file in files.flatten() {
                let path = file.path();
                let Some(doc_id) = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::to_string)
                else {
                    continue;
                };
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Ok(vectors) = serde_json::from_str::<VectorFile>(&text) {
                    save_vectors(&kb_id, &doc_id, &vectors)?;
                    imported += 1;
                }
            }
        }
    }
    Ok(imported)
}

// ------------------------------------------------------------------ 分块

pub struct ChunkOutcome {
    pub chunks: Vec<Chunk>,
    pub parents: Vec<Parent>,
    pub cleaned_chars: usize,
    pub dropped: usize,
    pub total_chars: usize,
}

/// 预处理：压缩连续空白、可选取掉 URL / 邮箱。
fn preprocess(text: &str, config: &KbConfig) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.replace('\r', "").lines() {
        let mut line = line.trim_end().to_string();
        if config.drop_urls {
            line = strip_patterns(&line, "http://", "https://");
        }
        if config.drop_emails {
            line = strip_emails(&line);
        }
        if line.trim().is_empty() {
            blank_run += 1;
            // 最多保留一个空行：段落边界要留住，但不要留出十来个空行
            if blank_run <= 1 {
                out.push('\n');
            }
            continue;
        }
        blank_run = 0;
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn strip_patterns(line: &str, a: &str, b: &str) -> String {
    let mut out = line.to_string();
    for prefix in [a, b] {
        while let Some(start) = out.find(prefix) {
            let end = out[start..]
                .find(char::is_whitespace)
                .map(|offset| start + offset)
                .unwrap_or(out.len());
            out.replace_range(start..end, "");
        }
    }
    out
}

fn strip_emails(line: &str) -> String {
    let mut out = String::new();
    for token in line.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim();
        let looks_like_email = trimmed.contains('@')
            && trimmed.contains('.')
            && !trimmed.contains('/')
            && trimmed.split('@').count() == 2;
        if looks_like_email {
            out.push_str(if token.ends_with(char::is_whitespace) { " " } else { "" });
            out.push(' ');
        } else {
            out.push_str(token);
        }
    }
    out
}

/// 分块：先按**硬分隔符**（默认 `\n####`）切大段，再按空行累加到目标长度。
///
/// 父子模式：父块 = 硬分隔出来的大段（喂模型），子块 = 累加出来的小块（拿来检索）。
pub fn chunk(text: &str, config: &KbConfig) -> ChunkOutcome {
    let cleaned = preprocess(text, config);
    let target = config.target();
    let overlap = config.overlap();
    let min_chunk = config.min_chunk_chars;

    let sections = split_sections(&cleaned, &config.separators);
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut parents: Vec<Parent> = Vec::new();
    let mut dropped = 0usize;
    let parent_mode = config.chunk_mode == "parentChild";

    for (section_index, section) in sections.iter().enumerate() {
        if section.trim().is_empty() {
            continue;
        }
        if parent_mode {
            parents.push(Parent {
                index: parents.len(),
                text: section.clone(),
            });
        }
        let parent_index = if parent_mode {
            Some(parents.len() - 1)
        } else {
            None
        };
        let mut buffer = String::new();
        for piece in split_pieces(section) {
            // 单块就超长：把它硬切成若干段（宁可切在句子中间，也不要一个巨块）
            if piece.chars().count() > target {
                if !buffer.trim().is_empty() {
                    push_chunk(&mut chunks, &mut buffer, min_chunk, &mut dropped, parent_index);
                }
                for hard in hard_split(&piece, target) {
                    let mut temp = hard;
                    push_chunk(&mut chunks, &mut temp, min_chunk, &mut dropped, parent_index);
                }
                continue;
            }
            if buffer.chars().count() + piece.chars().count() > target && !buffer.trim().is_empty() {
                // 换块时带上尾部重叠：断点正好落在答案中间时，重叠是唯一的补救
                let tail = tail_chars(&buffer, overlap);
                push_chunk(&mut chunks, &mut buffer, min_chunk, &mut dropped, parent_index);
                buffer = tail;
            }
            buffer.push_str(&piece);
            buffer.push('\n');
        }
        if !buffer.trim().is_empty() {
            push_chunk(&mut chunks, &mut buffer, min_chunk, &mut dropped, parent_index);
        }
        if chunks.len() >= MAX_CHUNKS {
            break;
        }
        let _ = section_index;
    }

    ChunkOutcome {
        chunks,
        parents,
        cleaned_chars: cleaned.chars().count(),
        dropped,
        total_chars: text.chars().count(),
    }
}

fn push_chunk(
    chunks: &mut Vec<Chunk>,
    buffer: &mut String,
    min_chunk: usize,
    dropped: &mut usize,
    parent_index: Option<usize>,
) {
    let text = buffer.trim().to_string();
    buffer.clear();
    if text.is_empty() {
        return;
    }
    if text.chars().count() < min_chunk {
        // 碎块（页眉页脚那种）丢掉：它们只会在检索时占位置
        *dropped += 1;
        return;
    }
    chunks.push(Chunk {
        index: chunks.len(),
        text,
        parent_index,
    });
}

fn split_sections(text: &str, separators: &[String]) -> Vec<String> {
    let mut parts = vec![text.to_string()];
    for separator in separators {
        if separator.trim().is_empty() {
            continue;
        }
        let mut next: Vec<String> = Vec::new();
        for part in parts {
            for piece in part.split(separator.as_str()) {
                next.push(piece.to_string());
            }
        }
        parts = next;
    }
    parts
}

/// 把一段切成「段落 → 句子」两级候选片段。
fn split_pieces(section: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    for paragraph in section.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        if paragraph.chars().count() <= 400 {
            pieces.push(paragraph.to_string());
            continue;
        }
        // 长段落按句子再切，避免「一块就是半篇文档」
        let mut sentence = String::new();
        for ch in paragraph.chars() {
            sentence.push(ch);
            if matches!(ch, '。' | '！' | '？' | '!' | '?' | ';' | '；' | '\n') {
                let trimmed = sentence.trim();
                if !trimmed.is_empty() {
                    pieces.push(trimmed.to_string());
                }
                sentence.clear();
            }
        }
        if !sentence.trim().is_empty() {
            pieces.push(sentence.trim().to_string());
        }
    }
    pieces
}

fn hard_split(text: &str, size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(size)
        .map(|slice| slice.iter().collect::<String>())
        .collect()
}

fn tail_chars(text: &str, count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let start = chars.len().saturating_sub(count);
    chars[start..].iter().collect()
}

// ------------------------------------------------------------------ 检索

/// 分词：中文按 2-gram，英文/数字按整词小写。
///
/// 中文不用分词器：2-gram 在「字段口径」「审批流程」这类短术语上的召回已经够用，
/// 而且**零依赖**（引一个中文分词器要多几 MB，还得跟词典版本打交道）。
fn tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut latin = String::new();
    let mut han: Vec<char> = Vec::new();
    let flush_latin = |latin: &mut String, out: &mut Vec<String>| {
        if !latin.is_empty() {
            out.push(latin.to_lowercase());
            latin.clear();
        }
    };
    let flush_han = |han: &mut Vec<char>, out: &mut Vec<String>| {
        if han.len() == 1 {
            out.push(han[0].to_string());
        }
        for pair in han.windows(2) {
            out.push(pair.iter().collect());
        }
        han.clear();
    };
    for ch in text.chars() {
        if ch.is_alphanumeric() && !is_han(ch) {
            latin.push(ch);
        } else if is_han(ch) {
            flush_latin(&mut latin, &mut out);
            han.push(ch);
        } else {
            flush_latin(&mut latin, &mut out);
            flush_han(&mut han, &mut out);
        }
    }
    flush_latin(&mut latin, &mut out);
    flush_han(&mut han, &mut out);
    out
}

fn is_han(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF)
}

/// BM25（K1=1.2、B=0.75）。每次检索现算，见文件头取舍 1。
fn bm25(query: &str, texts: &[String]) -> Vec<f64> {
    const K1: f64 = 1.2;
    const B: f64 = 0.75;
    let terms: Vec<String> = {
        let mut set: HashSet<String> = tokens(query).into_iter().collect();
        set.remove("");
        set.into_iter().collect()
    };
    let doc_tokens: Vec<Vec<String>> = texts.iter().map(|text| tokens(text)).collect();
    let lengths: Vec<f64> = doc_tokens.iter().map(|tokens| tokens.len() as f64).collect();
    let avg = if lengths.is_empty() {
        1.0
    } else {
        lengths.iter().sum::<f64>() / lengths.len() as f64
    };
    let total = doc_tokens.len().max(1) as f64;
    let mut scores = vec![0.0f64; doc_tokens.len()];
    for term in &terms {
        let mut df = 0usize;
        for tokens in &doc_tokens {
            if tokens.iter().any(|token| token == term) {
                df += 1;
            }
        }
        if df == 0 {
            continue;
        }
        let idf = ((total - df as f64 + 0.5) / (df as f64 + 0.5) + 1.0).ln();
        for (index, tokens) in doc_tokens.iter().enumerate() {
            let tf = tokens.iter().filter(|token| *token == term).count() as f64;
            if tf == 0.0 {
                continue;
            }
            let norm = 1.0 - B + B * (lengths[index] / avg.max(1e-6));
            scores[index] += idf * (tf * (K1 + 1.0)) / (tf + K1 * norm);
        }
    }
    scores
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let len = a.len().min(b.len());
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for index in 0..len {
        dot += a[index] as f64 * b[index] as f64;
        na += (a[index] as f64).powi(2);
        nb += (b[index] as f64).powi(2);
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

fn min_max(values: &[f64]) -> Vec<f64> {
    let max = values.iter().cloned().fold(f64::MIN, f64::max);
    let min = values.iter().cloned().fold(f64::MAX, f64::min);
    if (max - min).abs() < 1e-9 {
        return values.iter().map(|_| 0.0).collect();
    }
    values.iter().map(|value| (value - min) / (max - min)).collect()
}

pub struct Hit {
    pub kb_id: String,
    pub kb_name: String,
    pub doc_id: String,
    pub doc_title: String,
    pub index: usize,
    pub score: f64,
    pub text: String,
}

/// 在指定的库里检索。
fn search(kb_ids: &[String], query: &str, config: &KbConfig, model_id: Option<&str>) -> XResult<Vec<Hit>> {
    let mode = config.index_mode.as_str();
    let mut hits: Vec<Hit> = Vec::new();
    let embed_model = if mode == "keyword" {
        None
    } else {
        crate::api::ai::config::embedding_model(
            Some(if config.embed_model_id.is_empty() {
                model_id.unwrap_or("")
            } else {
                &config.embed_model_id
            })
            .filter(|id| !id.is_empty()),
        )
    };
    if mode != "keyword" && embed_model.is_none() {
        return Err(XError::bad_request(
            "向量检索需要配置一个支持 embeddings 的模型（或把检索模式切回「关键词」）",
        ));
    }
    let query_vector = match (&embed_model, mode) {
        (Some(model), "vector") | (Some(model), "hybrid") => {
            let vectors = crate::api::ai::config::embed(model, &[query.to_string()])?;
            vectors.into_iter().next()
        }
        _ => None,
    };

    for kb_id in kb_ids {
        let info = load_index()
            .into_iter()
            .find(|item| &item.id == kb_id)
            .ok_or_else(|| XError::bad_request(format!("知识库 {kb_id} 不存在")))?;
        let docs = docs_of(kb_id);
        let mut texts: Vec<String> = Vec::new();
        let mut meta: Vec<(usize, usize)> = Vec::new();
        let mut vectors: Vec<Vec<f32>> = Vec::new();
        for (doc_index, doc) in docs.iter().enumerate() {
            let stored = load_vectors(kb_id, &doc.id);
            for (chunk_index, chunk) in doc.chunks.iter().enumerate() {
                texts.push(chunk.text.clone());
                meta.push((doc_index, chunk_index));
                if let Some(stored) = &stored {
                    vectors.push(
                        stored
                            .vectors
                            .get(chunk_index)
                            .cloned()
                            .unwrap_or_default(),
                    );
                } else {
                    vectors.push(Vec::new());
                }
            }
        }
        if texts.is_empty() {
            continue;
        }
        let keyword_scores = if mode == "keyword" || mode == "hybrid" {
            bm25(query, &texts)
        } else {
            vec![0.0; texts.len()]
        };
        let vector_scores: Vec<f64> = match &query_vector {
            Some(query_vector) if !query_vector.is_empty() => vectors
                .iter()
                .map(|vector| {
                    if vector.is_empty() {
                        0.0
                    } else {
                        cosine(query_vector, vector)
                    }
                })
                .collect(),
            _ => vec![0.0; texts.len()],
        };
        let final_scores = match mode {
            "vector" => vector_scores.clone(),
            "hybrid" => {
                let k = min_max(&keyword_scores);
                let v = min_max(&vector_scores);
                k.iter()
                    .zip(v.iter())
                    .map(|(k, v)| k * config.hybrid_weight + v * (1.0 - config.hybrid_weight))
                    .collect()
            }
            _ => keyword_scores.clone(),
        };
        for (position, score) in final_scores.iter().enumerate() {
            if *score <= 0.0 || *score < config.min_score {
                continue;
            }
            let (doc_index, chunk_index) = meta[position];
            let doc = &docs[doc_index];
            hits.push(Hit {
                kb_id: kb_id.clone(),
                kb_name: info.name.clone(),
                doc_id: doc.id.clone(),
                doc_title: doc.title.clone(),
                index: chunk_index,
                score: (*score * 1000.0).round() / 1000.0,
                text: texts[position].clone(),
            });
        }
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(config.top_k.max(1));
    Ok(hits)
}

/// 给对话用的召回片段（`contextFor`）。
///
/// 注入时用围栏包住：文档是**用户上传的资料**，里面写什么都有可能，
/// 与表结构同理 —— 不能让它被当成指令。
pub fn context_for(state_hits: &[Hit], parents: bool) -> String {
    if state_hits.is_empty() {
        return String::new();
    }
    let mut text = String::from("<<<TEAM_DOCS_BEGIN>>>\n");
    for (number, hit) in state_hits.iter().enumerate() {
        text.push_str(&format!(
            "【参考资料 {}｜来源：{}】\n{}（片段 {}）\n{}\n\n",
            number + 1,
            hit.doc_title,
            if parents { "（父块）" } else { "" },
            hit.index + 1,
            hit.text
        ));
    }
    text.push_str("<<<TEAM_DOCS_END>>>");
    text
}

// ------------------------------------------------------------------ 请求

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KbReq {
    #[serde(default)]
    kb_id: Option<String>,
    #[serde(default)]
    doc_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    top_k: Option<usize>,
    #[serde(default)]
    index_mode: Option<String>,
    #[serde(default)]
    chunk_mode: Option<String>,
    #[serde(default)]
    target_chars: Option<usize>,
    #[serde(default)]
    overlap_chars: Option<usize>,
    #[serde(default)]
    separators: Option<Vec<String>>,
    #[serde(default)]
    min_chunk_chars: Option<usize>,
    #[serde(default)]
    min_score: Option<f64>,
    #[serde(default)]
    hybrid_weight: Option<f64>,
    #[serde(default)]
    drop_urls: Option<bool>,
    #[serde(default)]
    drop_emails: Option<bool>,
    #[serde(default)]
    embed_model_id: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
}

impl KbReq {
    fn kb(&self) -> XResult<String> {
        self.kb_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| XError::bad_request("缺少 kbId 参数"))
    }

    fn text(&self) -> XResult<String> {
        self.text
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| XError::bad_request("缺少 text 参数"))
    }

    /// 用请求里给的分段参数覆盖库配置（预览接口就是这样试参数的）。
    fn config_with(&self, base: &KbConfig) -> KbConfig {
        let mut config = base.clone();
        if let Some(mode) = &self.chunk_mode {
            config.chunk_mode = mode.clone();
        }
        if let Some(value) = self.target_chars {
            config.target_chars = value;
        }
        if let Some(value) = self.overlap_chars {
            config.overlap_chars = value;
        }
        if let Some(value) = &self.separators {
            if !value.is_empty() {
                config.separators = value.clone();
            }
        }
        if let Some(value) = self.min_chunk_chars {
            config.min_chunk_chars = value;
        }
        if let Some(value) = self.top_k {
            config.top_k = value;
        }
        if let Some(value) = &self.index_mode {
            config.index_mode = value.clone();
        }
        if let Some(value) = self.min_score {
            config.min_score = value;
        }
        if let Some(value) = self.hybrid_weight {
            config.hybrid_weight = value.clamp(0.0, 1.0);
        }
        if let Some(value) = self.drop_urls {
            config.drop_urls = value;
        }
        if let Some(value) = self.drop_emails {
            config.drop_emails = value;
        }
        if let Some(value) = &self.embed_model_id {
            config.embed_model_id = value.clone();
        }
        config
    }
}

// ------------------------------------------------------------------ 处理器

/// `POST /api/ai/kb/list`。
pub async fn list(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let items = load_index();
    // 单库配置 → 全局默认 → 内置默认（`load_config` 自己就带三级回退）
    let config = load_config(req.kb_id.as_deref().unwrap_or(""));
    Ok(Json(json!({ "items": items, "config": config })))
}

/// `POST /api/ai/kb/create`。
pub async fn create(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let name = req
        .name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("请填写知识库名称"))?;
    let id = format!("kb_{}", tasks::stamp().replace('_', ""));
    let info = KbInfo {
        id: id.clone(),
        name,
        description: req.description.clone().unwrap_or_default(),
        created_at: now(),
        updated_at: now(),
        doc_count: 0,
        chunk_count: 0,
        char_count: 0,
    };
    let mut items = load_index();
    items.push(info.clone());
    save_index(&items)?;
    Ok(Json(json!({ "success": true, "kbId": id, "info": info })))
}

/// `POST /api/ai/kb/rename`。
pub async fn rename(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let kb_id = safe_id(&req.kb()?)?;
    let mut items = load_index();
    let info = items
        .iter_mut()
        .find(|item| item.id == kb_id)
        .ok_or_else(|| XError::bad_request("知识库不存在"))?;
    if let Some(name) = req.name.clone().filter(|value| !value.trim().is_empty()) {
        info.name = name;
    }
    if let Some(description) = &req.description {
        info.description = description.clone();
    }
    info.updated_at = now();
    let snapshot = info.clone();
    save_index(&items)?;
    Ok(Json(json!({ "success": true, "info": snapshot })))
}

/// `POST /api/ai/kb/delete`。
pub async fn delete(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let kb_id = safe_id(&req.kb()?)?;
    let mut items = load_index();
    let before = items.len();
    items.retain(|item| item.id != kb_id);
    if items.len() == before {
        return Err(XError::bad_request("知识库不存在"));
    }
    save_index(&items)?;
    // 这个库的配置 / 文档 / 向量一起清掉（目录版是 remove_dir_all）
    if let Some(store) = dbmind_core::global_store() {
        let _ = store.kb_delete(&kb_id);
    }
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/ai/kb/docs`。
pub async fn docs(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let kb_id = safe_id(&req.kb()?)?;
    let config = load_config(&kb_id);
    let list = docs_of(&kb_id);
    let vectors_ready = list
        .iter()
        .all(|doc| load_vectors(&kb_id, &doc.id).map(|v| !v.vectors.is_empty()).unwrap_or(false));
    let items: Vec<Value> = list
        .iter()
        .map(|doc| {
            json!({
                "id": doc.id,
                "title": doc.title,
                "source": doc.source,
                "createdAt": doc.created_at,
                "charCount": doc.char_count,
                "charText": format_chars(doc.char_count),
                "chunkCount": doc.chunks.len(),
                "hasVector": load_vectors(&kb_id, &doc.id).map(|v| !v.vectors.is_empty()).unwrap_or(false),
            })
        })
        .collect();
    Ok(Json(json!({
        "docs": items,
        "chunkCount": list.iter().map(|doc| doc.chunks.len()).sum::<usize>(),
        "indexMode": config.index_mode,
        "vectorsReady": vectors_ready,
    })))
}

fn format_chars(count: usize) -> String {
    if count >= 10_000 {
        format!("{:.1} 万字", count as f64 / 10_000.0)
    } else {
        format!("{count} 字")
    }
}

/// `POST /api/ai/kb/doc/import` —— 入库（同步分块 + 可选算向量）。
pub async fn import_doc(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let kb_id = safe_id(&req.kb()?)?;
    let text = req.text()?;
    if text.chars().count() > MAX_DOC_CHARS {
        return Err(XError::bad_request(format!(
            "单份资料不能超过 {MAX_DOC_CHARS} 字（当前 {} 字），请拆分后再入库",
            text.chars().count()
        )));
    }
    let config = load_config(&kb_id);
    let outcome = chunk(&text, &config);
    if outcome.chunks.is_empty() {
        return Err(XError::bad_request(
            "分块后没有任何有效内容：检查「最小块长度」是否过大，或资料本身是否为空",
        ));
    }
    let doc_id = format!("doc_{}", tasks::stamp().replace('_', ""));
    let doc = KbDoc {
        id: doc_id.clone(),
        title: req
            .title
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "未命名资料".to_string()),
        source: req.source.clone().unwrap_or_default(),
        created_at: now(),
        char_count: text.chars().count(),
        raw: text,
        parents: outcome.parents,
        chunks: outcome.chunks,
    };
    save_doc(&kb_id, &doc)?;

    // 向量：**失败不阻断入库**（关键词检索照样能用），但会在返回值里说明
    let mut warning: Option<String> = None;
    if config.index_mode != "keyword" {
        match crate::api::ai::config::embedding_model(Some(&config.embed_model_id)) {
            Some(model) => {
                let texts: Vec<String> = doc.chunks.iter().map(|chunk| chunk.text.clone()).collect();
                match crate::api::ai::config::embed(&model, &texts) {
                    Ok(vectors) => {
                        let dim = vectors.first().map(Vec::len).unwrap_or(0);
                        save_vectors(
                            &kb_id,
                            &doc_id,
                            &VectorFile {
                                model: model.model.clone(),
                                dim,
                                vectors,
                            },
                        )?;
                    }
                    Err(err) => warning = Some(format!("向量生成失败（已按无向量入库）：{}", err.message)),
                }
            }
            None => {
                warning = Some(
                    "没有可用的向量模型：本次只建了关键词索引（检索模式仍是「关键词」）".to_string(),
                )
            }
        }
    }
    let info = refresh(&kb_id)?;
    Ok(Json(json!({
        "success": true,
        "docId": doc_id,
        "chunkCount": doc.chunks.len(),
        "info": info,
        "warning": warning,
    })))
}

/// `POST /api/ai/kb/doc/delete`。
pub async fn delete_doc(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let kb_id = safe_id(&req.kb()?)?;
    let doc_id = safe_id(
        req.doc_id
            .as_deref()
            .ok_or_else(|| XError::bad_request("缺少 docId 参数"))?,
    )?;
    let Some(store) = dbmind_core::global_store() else {
        return Err(XError::internal("主库未就绪，无法删除文档"));
    };
    match store.kb_doc(&kb_id, &doc_id) {
        Ok(Some(_)) => {}
        Ok(None) => return Err(XError::bad_request("文档不存在")),
        Err(err) => return Err(XError::internal(format!("读取文档失败：{err}"))),
    }
    // 文档与它的向量一起删（目录版是两个 remove_file）
    store
        .kb_delete_doc(&kb_id, &doc_id)
        .map_err(|err| XError::internal(format!("删除文档失败：{err}")))?;
    let info = refresh(&kb_id)?;
    Ok(Json(json!({ "success": true, "info": info })))
}

/// `POST /api/ai/kb/doc/chunks` —— 分块正文预览（详情页）。
pub async fn doc_chunks(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let kb_id = safe_id(&req.kb()?)?;
    let doc_id = safe_id(
        req.doc_id
            .as_deref()
            .ok_or_else(|| XError::bad_request("缺少 docId 参数"))?,
    )?;
    let doc = find_doc(&kb_id, &doc_id)?;
    let avg = if doc.chunks.is_empty() {
        0
    } else {
        doc.chars_total() / doc.chunks.len()
    };
    Ok(Json(json!({
        "docId": doc.id,
        "title": doc.title,
        "source": doc.source,
        "charCount": doc.char_count,
        "charText": format_chars(doc.char_count),
        "chunkCount": doc.chunks.len(),
        "parentCount": doc.parents.len(),
        "avgChars": avg,
        "raw": doc.raw.chars().take(MAX_RAW_ECHO).collect::<String>(),
        "chunks": doc.chunks.iter().map(|chunk| json!({
            "index": chunk.index,
            "text": chunk.text,
            "charCount": chunk.text.chars().count(),
            "parentIndex": chunk.parent_index,
        })).collect::<Vec<_>>(),
    })))
}

impl KbDoc {
    fn chars_total(&self) -> usize {
        self.chunks.iter().map(|chunk| chunk.text.chars().count()).sum()
    }
}

/// `POST /api/ai/kb/preview` —— 试切（不落盘）。
pub async fn preview(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let text = req.text()?;
    let base = req
        .kb_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .map(|kb_id| load_config(&kb_id))
        .unwrap_or_default();
    let config = req.config_with(&base);
    let outcome = chunk(&text, &config);
    let avg = if outcome.chunks.is_empty() {
        0
    } else {
        outcome
            .chunks
            .iter()
            .map(|chunk| chunk.text.chars().count())
            .sum::<usize>()
            / outcome.chunks.len()
    };
    Ok(Json(json!({
        "items": outcome.chunks.iter().map(|chunk| json!({
            "index": chunk.index,
            "text": chunk.text,
            "charCount": chunk.text.chars().count(),
            "parentIndex": chunk.parent_index,
        })).collect::<Vec<_>>(),
        "count": outcome.chunks.len(),
        "rawCount": outcome.total_chars,
        "parentCount": outcome.parents.len(),
        "chunkMode": config.chunk_mode,
        "cleanedChars": outcome.cleaned_chars,
        "dropped": outcome.dropped,
        "totalChars": text.chars().count(),
        "avgChars": avg,
        "truncated": text.chars().count() >= MAX_DOC_CHARS,
    })))
}

/// `POST /api/ai/kb/reindex` —— 重建向量。
pub async fn reindex(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let kb_id = safe_id(&req.kb()?)?;
    let config = load_config(&kb_id);
    if config.index_mode == "keyword" {
        // 关键词模式本来就不需要向量：**如实返回 0**，不要报错
        return Ok(Json(json!({
            "chunks": 0,
            "indexMode": config.index_mode,
            "message": "当前是关键词模式，不需要向量索引",
        })));
    }
    let model = crate::api::ai::config::embedding_model(Some(&config.embed_model_id))
        .ok_or_else(|| XError::bad_request("还没有可用的向量模型：向量索引需要能调用 embeddings 的模型"))?;
    let mut total = 0usize;
    for doc in docs_of(&kb_id) {
        let texts: Vec<String> = doc.chunks.iter().map(|chunk| chunk.text.clone()).collect();
        let vectors = crate::api::ai::config::embed(&model, &texts)?;
        let dim = vectors.first().map(Vec::len).unwrap_or(0);
        save_vectors(
            &kb_id,
            &doc.id,
            &VectorFile {
                model: model.model.clone(),
                dim,
                vectors,
            },
        )?;
        total += texts.len();
    }
    Ok(Json(json!({ "chunks": total, "model": model.model })))
}

/// `POST /api/ai/kb/search` —— 召回测试。
pub async fn search_handler(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let query = req
        .query
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("请输入检索内容"))?;
    let kb_ids: Vec<String> = match &req.kb_id {
        Some(id) if !id.trim().is_empty() => vec![safe_id(id)?],
        _ => load_index().into_iter().map(|item| item.id).collect(),
    };
    if kb_ids.is_empty() {
        return Ok(Json(json!({ "hits": [], "count": 0 })));
    }
    let base = req
        .kb_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .map(|kb_id| load_config(&kb_id))
        .unwrap_or_default();
    let config = req.config_with(&base);
    let hits = search(&kb_ids, &query, &config, req.model_id.as_deref())?;
    Ok(Json(json!({
        "hits": hits.iter().map(|hit| json!({
            "kbId": hit.kb_id,
            "kbName": hit.kb_name,
            "docId": hit.doc_id,
            "docTitle": hit.doc_title,
            "index": hit.index,
            "score": hit.score,
            "text": hit.text,
        })).collect::<Vec<_>>(),
        "count": hits.len(),
        "indexMode": config.index_mode,
    })))
}

/// `POST /api/ai/kb/config` —— 保存召回/分段配置。
pub async fn save_config_handler(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let kb_id = safe_id(&req.kb()?)?;
    let config = req.config_with(&load_config(&kb_id));
    save_config(&kb_id, &config)?;
    Ok(Json(json!({ "success": true, "config": config })))
}

/// `POST /api/ai/kb/inspect` —— 入库体检（规则 + 可选模型判定）。
pub async fn inspect(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let text = req.text()?;
    let source = req.source.clone().unwrap_or_default();
    let stats = text_stats(&text);
    let fatal = stats["fatal"].as_bool().unwrap_or(false);
    let mut reject_reason = stats["rejectReason"].as_str().unwrap_or("").to_string();
    let mut suitable = !fatal;
    let mut kind = "文本资料".to_string();
    let mut verdict = if fatal {
        "物理层判定不适合入库".to_string()
    } else {
        "未发现致命问题，可以入库".to_string()
    };
    let mut problems: Vec<Value> = stats["problems"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut ai_used = false;
    let mut ai_error: Option<String> = None;

    // 有模型时再做一次语义判定；**模型失败也放行**（规则层已经挡掉了明显不可用的）
    if !fatal {
        if let Ok(model) = crate::api::ai::config::resolve(req.model_id.as_deref()) {
            let sample: String = text.chars().take(4000).collect();
            let system = prompts::text_of("kb.inspect.system").unwrap_or_default();
            let user = prompts::render(
                "kb.inspect.user",
                &[("source", if source.is_empty() { "（未命名）" } else { &source }), ("text", &sample)],
            );
            match user {
                Ok(user) => {
                    let messages = vec![crate::api::ai::config::Msg {
                        role: "user".to_string(),
                        content: user,
                    }];
                    match run_blocking(move || crate::api::ai::config::chat(&model, &system, &messages)).await {
                        Ok(raw) => {
                            ai_used = true;
                            if let Ok(value) = serde_json::from_str::<Value>(&extract_json(&raw)) {
                                suitable = value.get("suitable").and_then(Value::as_bool).unwrap_or(suitable);
                                kind = value.get("kind").and_then(Value::as_str).unwrap_or(&kind).to_string();
                                verdict = value.get("verdict").and_then(Value::as_str).unwrap_or(&verdict).to_string();
                                if let Some(reason) = value.get("rejectReason").and_then(Value::as_str) {
                                    if !reason.trim().is_empty() {
                                        reject_reason = reason.to_string();
                                    }
                                }
                                if let Some(list) = value.get("problems").and_then(Value::as_array) {
                                    problems.extend(list.iter().cloned());
                                }
                            }
                        }
                        Err(err) => ai_error = Some(err.message),
                    }
                }
                Err(err) => ai_error = Some(err.message),
            }
        }
    }

    Ok(Json(json!({
        "stats": stats,
        "suitable": suitable,
        "fatal": fatal,
        "rejectReason": reject_reason,
        "kind": kind,
        "verdict": verdict,
        "problems": problems,
        "aiUsed": ai_used,
        "aiError": ai_error,
    })))
}

/// 物理层统计（不依赖模型）。
fn text_stats(text: &str) -> Value {
    let chars = text.chars().count();
    let lines = text.lines().count();
    let effective = text
        .lines()
        .filter(|line| line.trim().chars().count() >= 8)
        .count();
    let control = text
        .chars()
        .filter(|c| (*c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t'))
        .count();
    let replacement = text.chars().filter(|c| *c == '\u{FFFD}').count();
    let printable_ratio = if chars == 0 {
        0.0
    } else {
        1.0 - (control + replacement) as f64 / chars as f64
    };
    let garble_ratio = if chars == 0 {
        0.0
    } else {
        (control + replacement) as f64 / chars as f64
    };
    let mut seen: HashSet<&str> = HashSet::new();
    let mut duplicates = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.chars().count() < 8 {
            continue;
        }
        if !seen.insert(line) {
            duplicates += 1;
        }
    }
    let duplicate_ratio = if lines == 0 {
        0.0
    } else {
        duplicates as f64 / lines as f64
    };

    let mut problems: Vec<Value> = Vec::new();
    let mut fatal = false;
    let mut reject_reason = String::new();
    if chars < 50 {
        fatal = true;
        reject_reason = "内容太短（不足 50 字），没有可检索的知识量".to_string();
    }
    if garble_ratio > 0.05 {
        fatal = true;
        reject_reason = format!(
            "疑似乱码或二进制内容（不可打印字符占比 {:.1}%），不是可读文本",
            garble_ratio * 100.0
        );
    }
    if effective < 3 && !fatal {
        problems.push(json!({
            "type": "structure",
            "severity": "mid",
            "detail": "有效内容行很少（可能只有标题/目录）",
            "fix": "补充正文后再入库",
        }));
    }
    if duplicate_ratio > 0.4 {
        problems.push(json!({
            "type": "duplicate",
            "severity": "mid",
            "detail": format!("重复行比例较高（{:.0}%），可能是重复采集的日志", duplicate_ratio * 100.0),
            "fix": "先去重再入库",
        }));
    }
    json!({
        "chars": chars,
        "charText": format_chars(chars),
        "lines": lines,
        "effectiveLines": effective,
        "printableRatio": (printable_ratio * 1000.0).round() / 1000.0,
        "garbleRatio": (garble_ratio * 1000.0).round() / 1000.0,
        "duplicateRatio": (duplicate_ratio * 1000.0).round() / 1000.0,
        "fatal": fatal,
        "rejectReason": reject_reason,
        "warnings": problems.len(),
        "problems": problems,
    })
}

fn extract_json(text: &str) -> String {
    let text = text.trim();
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if end > start {
            return text[start..=end].to_string();
        }
    }
    text.to_string()
}

/// `POST /api/ai/kb/polish` —— 规范优化（模型 + 事实校验）。
pub async fn polish(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let text = req.text()?;
    let original_chars = text.chars().count();
    let model = crate::api::ai::config::resolve(req.model_id.as_deref())?;
    let system = prompts::text_of("kb.polish.system").unwrap_or_default();
    let user = prompts::render("kb.polish.user", &[("text", &text)])?;
    let messages = vec![crate::api::ai::config::Msg {
        role: "user".to_string(),
        content: user,
    }];
    let polished = run_blocking(move || crate::api::ai::config::chat(&model, &system, &messages)).await?;

    // 事实校验：把原文里的「事实性 token」（数字、日期、编号）抽出来，
    // 看润色后还剩多少 —— 润色最危险的失败是**把数字改掉**，那比文字不通顺严重得多
    let facts = fact_tokens(&text);
    let kept: Vec<String> = facts
        .iter()
        .filter(|fact| polished.contains(fact.as_str()))
        .cloned()
        .collect();
    let missing: Vec<String> = facts
        .iter()
        .filter(|fact| !polished.contains(fact.as_str()))
        .take(20)
        .cloned()
        .collect();
    let keep_rate = if facts.is_empty() {
        100.0
    } else {
        (kept.len() as f64 * 100.0 / facts.len() as f64 * 10.0).round() / 10.0
    };
    let changed = polished.trim() != text.trim();
    Ok(Json(json!({
        "text": polished,
        "originalChars": original_chars,
        "polishedChars": polished.chars().count(),
        "changed": changed,
        "factsTotal": facts.len(),
        "factsKept": kept.len(),
        "keepRate": keep_rate,
        "factsMissing": missing,
        "factsMissingCount": facts.len().saturating_sub(kept.len()),
        "risk": if keep_rate < 95.0 { Some("润色后丢失了部分数字/编号，请人工确认") } else { None },
    })))
}

/// 抽取「不该被改写」的事实片段：数字串与日期。
fn fact_tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, out: &mut Vec<String>| {
        if current.chars().filter(|c| c.is_ascii_digit()).count() >= 2 {
            out.push(current.clone());
        }
        current.clear();
    };
    for ch in text.chars() {
        if ch.is_ascii_digit() || matches!(ch, '-' | '/' | '.' | ':' | '%') {
            current.push(ch);
        } else {
            flush(&mut current, &mut out);
        }
    }
    flush(&mut current, &mut out);
    out.sort();
    out.dedup();
    out.truncate(200);
    out
}

/// `POST /api/ai/kb/auto-plan` —— 推荐配置 + 起名（模型，失败兜底）。
pub async fn auto_plan(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let text = req.text()?;
    let source = req.source.clone().unwrap_or_default();
    let stats = text_stats(&text);
    // 规则推荐：字符分布决定分段长度，内容规模决定索引模式
    let suggest_vector = text.chars().count() >= 20_000;
    let recommend = if suggest_vector { "hybrid" } else { "keyword" };
    let mut name = req
        .name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::path::Path::new(&source)
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "新建知识库".to_string());
    let mut description = format!(
        "{}，约 {}",
        stats["charText"].as_str().unwrap_or(""),
        stats["lines"].as_i64().unwrap_or(0)
    );
    let mut ai_used = false;
    let mut ai_error: Option<String> = None;
    if let Ok(model) = crate::api::ai::config::resolve(req.model_id.as_deref()) {
        let sample: String = text.chars().take(3000).collect();
        let system = prompts::text_of("kb.plan.system").unwrap_or_default();
        if let Ok(user) = prompts::render(
            "kb.plan.user",
            &[("source", if source.is_empty() { "（未命名）" } else { &source }), ("text", &sample)],
        ) {
            let messages = vec![crate::api::ai::config::Msg {
                role: "user".to_string(),
                content: user,
            }];
            match run_blocking(move || crate::api::ai::config::chat(&model, &system, &messages)).await {
                Ok(raw) => {
                    ai_used = true;
                    if let Ok(value) = serde_json::from_str::<Value>(&extract_json(&raw)) {
                        if let Some(found) = value.get("name").and_then(Value::as_str) {
                            if !found.trim().is_empty() {
                                name = found.to_string();
                            }
                        }
                        if let Some(found) = value.get("description").and_then(Value::as_str) {
                            if !found.trim().is_empty() {
                                description = found.to_string();
                            }
                        }
                    }
                }
                Err(err) => ai_error = Some(err.message),
            }
        }
    }
    Ok(Json(json!({
        "stats": stats,
        "recommend": recommend,
        "buildVector": suggest_vector,
        "indexMode": recommend,
        "name": name,
        "description": description,
        "aiUsed": ai_used,
        "aiError": ai_error,
        "source": source,
    })))
}

/// `POST /api/ai/kb/outline` —— 归纳整个库（模型，严格 JSON）。
pub async fn outline(Json(req): Json<KbReq>) -> XResult<Json<Value>> {
    let kb_id = safe_id(&req.kb()?)?;
    let info = load_index()
        .into_iter()
        .find(|item| item.id == kb_id)
        .ok_or_else(|| XError::bad_request("知识库不存在"))?;
    let docs = docs_of(&kb_id);
    if docs.is_empty() {
        return Err(XError::bad_request("这个库还没有资料，先入库再归纳"));
    }
    // 取样：每份资料取前若干块，总量控制在上下文安全范围内
    let mut sample = String::new();
    for doc in docs.iter().take(20) {
        sample.push_str(&format!("### {}\n", doc.title));
        for chunk in doc.chunks.iter().take(3) {
            sample.push_str(&chunk.text.chars().take(500).collect::<String>());
            sample.push('\n');
        }
    }
    let model = crate::api::ai::config::resolve(req.model_id.as_deref())?;
    let system = prompts::text_of("kb.outline.system").unwrap_or_default();
    let user = prompts::render(
        "kb.outline.user",
        &[
            ("kbName", &info.name),
            ("docCount", &docs.len().to_string()),
            ("text", &sample),
        ],
    )?;
    let messages = vec![crate::api::ai::config::Msg {
        role: "user".to_string(),
        content: user,
    }];
    let raw = run_blocking(move || crate::api::ai::config::chat(&model, &system, &messages)).await?;
    let value: Value = serde_json::from_str(&extract_json(&raw)).unwrap_or_else(|_| {
        json!({ "summary": raw.chars().take(200).collect::<String>(), "questions": [], "terms": [] })
    });
    Ok(Json(json!({
        "summary": value.get("summary").cloned().unwrap_or(Value::Null),
        "questions": value.get("questions").cloned().unwrap_or(json!([])),
        "terms": value.get("terms").cloned().unwrap_or(json!([])),
        "aiUsed": true,
    })))
}

/// 供对话调用：按提问召回知识库片段（`kbIds`：null=全部、[]=不注入、[...]=指定）。
///
/// **同步函数**（没有 `async`）：它读文件、可能还打一次 embeddings 接口，全是阻塞操作，
/// 调用方负责把它放进 blocking 线程池。写成 async 是个陷阱 ——
/// 那样它会在 async 上下文里「看起来能调」，实际把运行时卡住。
pub fn recall(question: &str, kb_ids: &Option<Value>, model_id: Option<&str>) -> Option<String> {
    let question = question.trim();
    if question.is_empty() {
        return None;
    }
    let targets: Vec<String> = match kb_ids {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::Null) | None => load_index().into_iter().map(|item| item.id).collect(),
        _ => load_index().into_iter().map(|item| item.id).collect(),
    };
    if targets.is_empty() {
        return None;
    }
    let config = KbConfig::default();
    match search(&targets, question, &config, model_id) {
        Ok(hits) => {
            let (top, _rest) = hits.split_at(hits.len().min(4));
            let text = context_for(top, config.chunk_mode == "parentChild");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        // 召回失败不该让对话失败（比如向量模式没配模型）
        Err(_) => None,
    }
}

/// 路由。
pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/api/ai/kb/list", post(list))
        .route("/api/ai/kb/create", post(create))
        .route("/api/ai/kb/rename", post(rename))
        .route("/api/ai/kb/delete", post(delete))
        .route("/api/ai/kb/docs", post(docs))
        .route("/api/ai/kb/doc/import", post(import_doc))
        .route("/api/ai/kb/doc/delete", post(delete_doc))
        .route("/api/ai/kb/doc/chunks", post(doc_chunks))
        .route("/api/ai/kb/preview", post(preview))
        .route("/api/ai/kb/reindex", post(reindex))
        .route("/api/ai/kb/search", post(search_handler))
        .route("/api/ai/kb/config", post(save_config_handler))
        .route("/api/ai/kb/inspect", post(inspect))
        .route("/api/ai/kb/polish", post(polish))
        .route("/api/ai/kb/auto-plan", post(auto_plan))
        .route("/api/ai/kb/outline", post(outline))
}
