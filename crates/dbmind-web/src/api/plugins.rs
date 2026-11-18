//! 插件中心：官网托管、产品内一键安装的「纯数据扩展包」。
//!
//! ## 为什么是纯数据，不是代码
//!
//! 插件要从网络下载、装进**用户的数据库客户端**里。一旦允许携带可执行文件
//! （`dll`/`so`/`js`/`ps1`…），插件市场就等于一个任意代码执行通道：
//! 谁都能上传一个包，用户点一下「安装」就在自己的生产环境旁跑起来了。
//! 所以第一版只收**声明式**内容：`manifest.json` + 提示词（`.md`）+ 图标（`.svg`），
//! 安装时白名单校验，出现可执行后缀直接拒绝（见 [`BLOCKED_EXT`]）。
//!
//! 需要跑逻辑的能力（比如未来的 MCP Server），走「插件只声明配置、本体仍是产品内置」
//! 的路子 —— 端口、命令、参数由 manifest 描述，执行者永远是 DBMind 自己。
//!
//! ## 目录约定
//!
//! ```text
//! ~/.dbmind/plugins/<id>/
//!     manifest.json     必需：id / type / name / version / icon / prompt
//!     chat.md           对话形态的提示词（推荐；缺省退回 prompt.md）
//!     prompt.md         任务模板（兼容老包，占位符在对话形态下会被清掉）
//! ```
//!
//! 技能**没有独立界面**：装好后在 AI 助手的 Skills 里选中，提示词拼进 system，
//! 之后的每一轮提问都由它生效（见 [`augment_system`]）。所以 manifest 里不再有
//! `inputs`（那是「表单 → 一次生成」时代的字段，已废弃：带 inputs 的旧包照常能装，
//! 只是这些字段不再被读取）。
//!
//! 磁盘是**唯一真相**（装没装、什么版本），只有「启用/禁用」存在 `app_settings`
//! 的 `plugins.disabled` 里 —— 那点状态不值得单独建表，也不会因为目录被手工删掉而错乱。
//!
//! ## 安装链路
//!
//! 官网 `plugins/index.json`（双源：本站 + jsDelivr 镜像）→ 选一个包 → 下载 zip →
//! 可选 sha256 校验 → 安全解压到临时目录 → 校验 manifest → 原子改名到 `plugins/<id>`。
//! 任何一步失败都清掉临时目录，绝不留半个包在用户机器上。

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Multipart, Path as AxPath};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::ai::run_blocking;
use crate::api::error::{XError, XResult};

/// 当前能认的 manifest 版本。老版本产品读到更高版本应当**拒绝**而不是硬着头皮装
/// （字段语义可能已经变了，硬装出来的是个行为不可预期的插件）。
const SCHEMA_VERSION: u32 = 1;
/// 「哪些插件被停用」存这里（JSON 数组）。
const DISABLED_KEY: &str = "plugins.disabled";
/// 单包上限：提示词+图标，8 MB 足够；超过说明不是这类插件。
const MAX_ZIP_BYTES: u64 = 8 * 1024 * 1024;
/// 单个文件解压上限，防 zip 炸弹（一个 1 KB 的包解出 4 GB）。
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// 官网索引：主源 + 镜像（GitHub 直连不稳时 jsDelivr 通常可用；两者都挂就显示「离线」）。
const CATALOG_URLS: &[&str] = &[
    "https://www.dbmind.top/plugins/index.json",
    "https://cdn.jsdelivr.net/gh/rick-works/dbmind@master/website/plugins/index.json",
];

/// 插件包里**不允许**出现的后缀。命中即拒绝整包 —— 不做「只跳过这个文件」的妥协：
/// 一个包里出现可执行文件，说明作者的心智模型不是「声明式插件」。
const BLOCKED_EXT: &[&str] = &[
    "exe", "dll", "so", "dylib", "com", "bat", "cmd", "ps1", "sh", "py", "rb", "pl", "js", "mjs",
    "cjs", "jar", "node", "wasm", "msi", "dmg", "app",
];

// ------------------------------------------------------------------ manifest

/// 双语字段：字符串（单语）或 `{ "zh": …, "en": … }`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Localized {
    Text(String),
    Map(BTreeMap<String, String>),
}

impl Localized {
    fn pick(&self, lang: &str) -> String {
        match self {
            Localized::Text(text) => text.clone(),
            Localized::Map(map) => {
                // 中文界面上英文也认，反之亦然（作者只写一种语言时不要出现空标题）
                let order: &[&str] = if lang.starts_with("en") {
                    &["en", "zh", "zh-CN"]
                } else {
                    &["zh", "zh-CN", "en"]
                };
                for key in order {
                    if let Some(v) = map.get(*key) {
                        if !v.trim().is_empty() {
                            return v.clone();
                        }
                    }
                }
                map.values().next().cloned().unwrap_or_default()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default)]
    pub schema_version: u32,
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: Option<Localized>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub description: Option<Localized>,
    #[serde(default)]
    pub icon: Option<String>,
    /// 是否把当前库的表结构塞进 system（写 SQL / 看表结构的技能需要）。
    #[serde(default)]
    pub needs_context: bool,
    #[serde(default)]
    pub prompt: Option<String>,
    /// 对话形态用的提示词（相对路径，可选）。
    ///
    /// `prompt` 是「表单 → 一次生成」的任务模板（带 `{{sql}}` 这类变量）；对话形态下没有表单，
    /// 变量由用户消息给出，所以作者可以另写一份「人格与方法」放进这个文件。
    /// 没给就退回 `prompt` 并清洗掉变量占位符（见 [`strip_placeholders`]）。
    #[serde(default)]
    pub chat_prompt: Option<String>,
}

// ------------------------------------------------------------------ 目录与状态

fn root() -> PathBuf {
    dbmind_core::paths::home_dir().join("plugins")
}

fn ensure_root() -> XResult<PathBuf> {
    let dir = root();
    dbmind_core::paths::ensure_dir(&dir)?;
    Ok(dir)
}

/// 插件 id 同时是目录名：只允许小写字母数字与 `._-`，且不能以点开头
/// （`.` 开头会和临时目录 `.tmp-…` 撞，`..` 更是目录穿越）。
fn id_ok(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 || id.starts_with('.') {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

fn disabled_ids() -> BTreeSet<String> {
    dbmind_core::global_store()
        .and_then(|store| store.get_setting(DISABLED_KEY).ok().flatten())
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .map(|list| list.into_iter().collect())
        .unwrap_or_default()
}

fn write_disabled(ids: &BTreeSet<String>) -> XResult<()> {
    let store = dbmind_core::global_store()
        .ok_or_else(|| XError::internal("本地存储尚未就绪，无法保存插件状态"))?;
    let text = serde_json::to_string(&ids.iter().cloned().collect::<Vec<_>>())
        .map_err(|e| XError::internal(format!("序列化插件状态失败：{e}")))?;
    store
        .set_setting(DISABLED_KEY, &text)
        .map_err(|e| XError::internal(format!("保存插件状态失败：{e}")))
}

/// 读一个插件目录的 manifest，并做静态校验。
fn read_manifest(dir: &Path) -> XResult<Manifest> {
    let file = dir.join("manifest.json");
    let text = std::fs::read_to_string(&file)
        .map_err(|e| XError::bad_request(format!("读取 {} 失败：{e}", file.display())))?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|e| XError::bad_request(format!("manifest.json 不是合法 JSON：{e}")))?;
    validate(&manifest, dir)?;
    Ok(manifest)
}

fn validate(manifest: &Manifest, dir: &Path) -> XResult<()> {
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(XError::bad_request(format!(
            "插件需要更新的 DBMind（manifest schemaVersion={}，本机只支持到 {SCHEMA_VERSION}）",
            manifest.schema_version
        )));
    }
    if !id_ok(&manifest.id) {
        return Err(XError::bad_request(format!(
            "插件 id「{}」不合法：只允许小写字母/数字与 . _ -",
            manifest.id
        )));
    }
    if manifest.kind != "skill" {
        return Err(XError::bad_request(format!(
            "暂不支持的插件类型「{}」（当前只支持 skill）",
            manifest.kind
        )));
    }
    let prompt = manifest.prompt.as_deref().unwrap_or("").trim();
    if prompt.is_empty() {
        return Err(XError::bad_request("skill 插件必须声明 prompt 文件"));
    }
    if !dir.join(prompt).is_file() {
        return Err(XError::bad_request(format!("插件包里缺少提示词文件 {prompt}")));
    }
    if let Some(chat) = manifest.chat_prompt.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !dir.join(chat).is_file() {
            return Err(XError::bad_request(format!("插件包里缺少对话提示词文件 {chat}")));
        }
    }
    Ok(())
}

/// 已装插件清单（按 id 排序，磁盘为准）。
fn scan() -> XResult<Vec<(Manifest, PathBuf, String)>> {
    let dir = ensure_root()?;
    let disabled = disabled_ids();
    let mut out: Vec<(Manifest, PathBuf, String)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // 临时目录（安装中）不算插件；非目录也不算
        if !path.is_dir() || name.starts_with('.') {
            continue;
        }
        match read_manifest(&path) {
            Ok(manifest) => {
                let state = if disabled.contains(&manifest.id) { "disabled" } else { "enabled" };
                out.push((manifest, path, state.to_string()));
            }
            Err(err) => {
                // 半坏目录不让它把整个列表打挂：跳过 + 记日志，用户可以在界面里删掉它
                tracing::warn!(dir = %path.display(), error = %err.message, "插件目录无法识别，已跳过");
            }
        }
    }
    out.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    Ok(out)
}

fn item_json(manifest: &Manifest, dir: &Path, state: &str, lang: &str) -> Value {
    let name = manifest.name.as_ref().map(|n| n.pick(lang)).unwrap_or_default();
    json!({
        "id": manifest.id,
        "type": manifest.kind,
        "name": if name.is_empty() { manifest.id.clone() } else { name },
        "description": manifest.description.as_ref().map(|d| d.pick(lang)).unwrap_or_default(),
        "version": manifest.version.clone().unwrap_or_default(),
        "author": manifest.author.clone().unwrap_or_default(),
        "homepage": manifest.homepage.clone().unwrap_or_default(),
        "needsContext": manifest.needs_context,
        "state": state,
        "dir": dir.display().to_string(),
        "icon": manifest.icon.clone().unwrap_or_default(),
    })
}

// ------------------------------------------------------------------ 接口：清单

/// `GET /api/plugins[?lang=zh]` —— 已装插件。
pub async fn list() -> XResult<Json<Value>> {
    let items: Vec<Value> = scan()?
        .iter()
        .map(|(m, dir, state)| item_json(m, dir, state, "zh"))
        .collect();
    Ok(Json(json!({
        "dir": root().display().to_string(),
        "items": items,
        "count": items.len(),
    })))
}

/// 官网索引缓存：10 分钟。插件中心是低频页面，但用户可能反复点开对比。
fn catalog_cache() -> &'static Mutex<Option<(Instant, Value)>> {
    static CACHE: OnceLock<Mutex<Option<(Instant, Value)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// `GET /api/plugins/catalog` —— 官网插件索引。
///
/// 拉不到**不算错误**（`ok: false` + 错误原因）：插件中心是可选功能，
/// 断网时不该弹一个红色报错框，界面显示「离线，可稍后重试 / 从文件安装」即可。
pub async fn catalog() -> XResult<Json<Value>> {
    if let Some((at, value)) = catalog_cache().lock().ok().and_then(|c| c.clone()) {
        if at.elapsed() < Duration::from_secs(600) {
            return Ok(Json(value));
        }
    }
    let fetched = run_blocking(|| {
        let mut errs: Vec<String> = Vec::new();
        for url in CATALOG_URLS {
            match fetch_text(url) {
                Ok(text) => match serde_json::from_str::<Value>(&text) {
                    Ok(mut value) => {
                        if let Some(obj) = value.as_object_mut() {
                            obj.insert("source".into(), json!(url));
                        }
                        return Ok(value);
                    }
                    Err(err) => errs.push(format!("{url}: 索引不是合法 JSON（{err}）")),
                },
                Err(err) => errs.push(format!("{url}: {}", err.message)),
            }
        }
        Err(XError::internal(errs.join("；")))
    })
    .await;

    let value = match fetched {
        Ok(value) => json!({ "ok": true, "data": value }),
        Err(err) => json!({ "ok": false, "error": err.message }),
    };
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        if let Ok(mut cache) = catalog_cache().lock() {
            *cache = Some((Instant::now(), value.clone()));
        }
    }
    Ok(Json(value))
}

// ------------------------------------------------------------------ 接口：安装 / 启停 / 卸载

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReq {
    pub id: String,
    #[serde(default)]
    pub url: Option<String>,
    /// 官网索引里给的摘要；给了就校验，不给只在日志里提示（老索引兼容）。
    #[serde(default)]
    pub sha256: Option<String>,
}

/// `POST /api/plugins/install` —— 按 id（或显式 url）从官网安装。
pub async fn install(Json(req): Json<InstallReq>) -> XResult<Json<Value>> {
    if !id_ok(&req.id) {
        return Err(XError::bad_request("插件 id 不合法"));
    }
    let url = match req.url.clone() {
        Some(url) => url,
        None => {
            // 只给 id 的时候（脚本/命令行调用）先把索引拉一次，别让调用方还要先"刷新列表"
            if catalog_cache().lock().ok().and_then(|c| c.clone()).is_none() {
                let _ = catalog().await;
            }
            catalog_url_of(&req.id)?
        }
    };
    if !url.starts_with("https://") {
        return Err(XError::bad_request("插件下载地址必须是 https"));
    }
    let id = req.id.clone();
    let sha = req.sha256.clone().unwrap_or_default();
    run_blocking(move || install_from_url(&id, &url, &sha))
        .await
        .map(Json)
}

/// 从官网索引里找这个 id 的下载地址。
fn catalog_url_of(id: &str) -> XResult<String> {
    let cached = catalog_cache().lock().ok().and_then(|c| c.clone());
    let Some((_, value)) = cached else {
        return Err(XError::bad_request("还没有插件索引，请先刷新插件中心"));
    };
    let items = value
        .pointer("/data/plugins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in items {
        if item.get("id").and_then(Value::as_str) == Some(id) {
            if let Some(url) = item.get("download").and_then(Value::as_str) {
                return Ok(url.to_string());
            }
        }
    }
    Err(XError::bad_request(format!("插件索引里没有「{id}」")))
}

fn fetch_text(url: &str) -> XResult<String> {
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(20))
        .call()
        .map_err(|e| XError::internal(format!("请求失败：{e}")))?;
    let mut text = String::new();
    resp.into_reader()
        .take(4 * 1024 * 1024)
        .read_to_string(&mut text)
        .map_err(|e| XError::internal(format!("读取响应失败：{e}")))?;
    Ok(text)
}

fn fetch_bytes(url: &str, limit: u64) -> XResult<Vec<u8>> {
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(60))
        .call()
        .map_err(|e| XError::internal(format!("下载失败：{e}")))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| XError::internal(format!("下载中断：{e}")))?;
    if bytes.len() as u64 > limit {
        return Err(XError::bad_request(format!(
            "插件包超过 {} MB 上限，已拒绝",
            limit / 1024 / 1024
        )));
    }
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn install_from_url(id: &str, url: &str, sha256: &str) -> XResult<Value> {
    let bytes = fetch_bytes(url, MAX_ZIP_BYTES)?;
    if !sha256.trim().is_empty() {
        let actual = sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(sha256.trim()) {
            return Err(XError::bad_request(format!(
                "插件包校验失败（期望 {sha256}，实际 {actual}），已丢弃"
            )));
        }
    } else {
        tracing::info!(id, url, "插件索引未提供 sha256，跳过完整性校验");
    }
    install_from_bytes(id, &bytes)
}

/// 把一个 zip 装成 `plugins/<id>`。安装过程全部在临时目录里完成，
/// 校验通过才改名 —— 中途失败不会留下半个插件。
fn install_from_bytes(want_id: &str, bytes: &[u8]) -> XResult<Value> {
    let dest_root = ensure_root()?;
    let tmp = dest_root.join(format!(
        ".tmp-{want_id}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let result = (|| -> XResult<Value> {
        dbmind_core::paths::ensure_dir(&tmp)?;
        extract_zip(bytes, &tmp)?;
        // zip 里很常见「外面套一层同名目录」，顺着走一层（只走一层，避免误判）
        let dir = if tmp.join("manifest.json").is_file() {
            tmp.clone()
        } else {
            let mut subs: Vec<PathBuf> = std::fs::read_dir(&tmp)
                .map_err(|e| XError::internal(format!("读取临时目录失败：{e}")))?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            if subs.len() == 1 && subs[0].join("manifest.json").is_file() {
                subs.remove(0)
            } else {
                return Err(XError::bad_request("插件包里找不到 manifest.json"));
            }
        };
        let manifest = read_manifest(&dir)?;
        if manifest.id != want_id {
            return Err(XError::bad_request(format!(
                "插件包声明的 id 是「{}」，与要安装的「{want_id}」不一致",
                manifest.id
            )));
        }
        let target = dest_root.join(&manifest.id);
        if target.exists() {
            std::fs::remove_dir_all(&target)
                .map_err(|e| XError::internal(format!("覆盖旧版本失败：{e}")))?;
        }
        std::fs::rename(&dir, &target)
            .map_err(|e| XError::internal(format!("安装插件失败（改名）：{e}")))?;
        // 装好即启用：用户的意图就是「我要用这个技能」
        let mut disabled = disabled_ids();
        if disabled.remove(&manifest.id) {
            write_disabled(&disabled)?;
        }
        tracing::info!(id = manifest.id, version = ?manifest.version, "插件已安装");
        Ok(item_json(&manifest, &target, "enabled", "zh"))
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// 安全解压：拒绝 `..`/绝对路径、拒绝可执行后缀、限制大小。
fn extract_zip(bytes: &[u8], dest: &Path) -> XResult<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| XError::bad_request(format!("不是合法的 zip 包：{e}")))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| XError::bad_request(format!("读取 zip 条目失败：{e}")))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(XError::bad_request(format!(
                "插件包里有非法路径「{}」，已拒绝安装",
                entry.name()
            )));
        };
        let name = rel.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if let Some(ext) = name.rsplit('.').next() {
            if BLOCKED_EXT.contains(&ext) {
                return Err(XError::bad_request(format!(
                    "插件包里有可执行文件「{}」，已拒绝安装（插件只允许声明式内容）",
                    entry.name()
                )));
            }
        }
        let out = dest.join(&rel);
        if entry.is_dir() {
            dbmind_core::paths::ensure_dir(&out)?;
            continue;
        }
        if entry.size() > MAX_FILE_BYTES {
            return Err(XError::bad_request(format!(
                "插件包里的「{}」超过 {} KB 上限",
                entry.name(),
                MAX_FILE_BYTES / 1024
            )));
        }
        if let Some(parent) = out.parent() {
            dbmind_core::paths::ensure_dir(parent)?;
        }
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| XError::bad_request(format!("解压「{}」失败：{e}", entry.name())))?;
        std::fs::write(&out, &buf)
            .map_err(|e| XError::internal(format!("写入「{}」失败：{e}", out.display())))?;
    }
    Ok(())
}

/// `POST /api/plugins/install-file` —— 从本地 zip 安装（multipart，字段名 file）。
pub async fn install_file(mut multipart: Multipart) -> XResult<Json<Value>> {
    let mut bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| XError::bad_request(format!("读取上传内容失败：{e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let data = field
            .bytes()
            .await
            .map_err(|e| XError::bad_request(format!("读取文件失败：{e}")))?;
        if data.len() as u64 > MAX_ZIP_BYTES {
            return Err(XError::bad_request("插件包超过 8 MB 上限"));
        }
        bytes = Some(data.to_vec());
        break;
    }
    let bytes = bytes.ok_or_else(|| XError::bad_request("没有收到插件包（字段名应为 file）"))?;
    // 先看一眼 manifest 里的 id：安装路径由它决定（本地安装时不另外指定 id）
    let peek = peek_manifest_id(&bytes)?;
    run_blocking(move || install_from_bytes(&peek, &bytes))
        .await
        .map(Json)
}

/// 从 zip 里读出 manifest.id（用于本地安装时确定目标目录）。
fn peek_manifest_id(bytes: &[u8]) -> XResult<String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| XError::bad_request(format!("不是合法的 zip 包：{e}")))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| XError::bad_request(format!("读取 zip 条目失败：{e}")))?;
        if entry.enclosed_name().map(|p| p.ends_with("manifest.json")).unwrap_or(false) {
            let mut text = String::new();
            entry
                .read_to_string(&mut text)
                .map_err(|e| XError::bad_request(format!("读取 manifest.json 失败：{e}")))?;
            let manifest: Manifest = serde_json::from_str(&text)
                .map_err(|e| XError::bad_request(format!("manifest.json 不是合法 JSON：{e}")))?;
            if !id_ok(&manifest.id) {
                return Err(XError::bad_request("manifest 里的 id 不合法"));
            }
            return Ok(manifest.id);
        }
    }
    Err(XError::bad_request("插件包里找不到 manifest.json"))
}

/// `POST /api/plugins/{id}/enable` / `.../disable` —— 停用只是不再出现在 AI 技能里，
/// 文件与目录不动（用户随时能再开关回来，也不丢他改过的东西）。
pub async fn enable(AxPath(id): AxPath<String>) -> XResult<Json<Value>> {
    set_enabled(&id, true).map(Json)
}

pub async fn disable(AxPath(id): AxPath<String>) -> XResult<Json<Value>> {
    set_enabled(&id, false).map(Json)
}

fn set_enabled(id: &str, on: bool) -> XResult<Value> {
    let dir = root().join(id);
    if !dir.is_dir() {
        return Err(XError::bad_request(format!("插件「{id}」未安装")));
    }
    let mut disabled = disabled_ids();
    let changed = if on { disabled.remove(id) } else { disabled.insert(id.to_string()) };
    if changed {
        write_disabled(&disabled)?;
    }
    Ok(json!({ "id": id, "state": if on { "enabled" } else { "disabled" } }))
}

/// `DELETE /api/plugins/{id}` —— 卸载：删目录 + 清停用标记。
pub async fn uninstall(AxPath(id): AxPath<String>) -> XResult<Json<Value>> {
    if !id_ok(&id) {
        return Err(XError::bad_request("插件 id 不合法"));
    }
    let dir = root().join(&id);
    if !dir.is_dir() {
        return Err(XError::bad_request(format!("插件「{id}」未安装")));
    }
    std::fs::remove_dir_all(&dir)
        .map_err(|e| XError::internal(format!("删除插件目录失败：{e}")))?;
    let mut disabled = disabled_ids();
    if disabled.remove(&id) {
        write_disabled(&disabled)?;
    }
    tracing::info!(id, "插件已卸载");
    Ok(Json(json!({ "id": id, "removed": true })))
}

// ------------------------------------------------------------------ 对话形态：技能拼进 system

/// 单个技能最多贡献多少字符：提示词是插件作者写的，不能让它把上下文预算吃光。
const MAX_SKILL_PROMPT_CHARS: usize = 8000;
/// 一条对话最多叠加几个技能（前端也限，这里兜底）。
const MAX_SKILLS_PER_TURN: usize = 8;

/// 清掉 `{{变量}}` 占位符。
///
/// 对话形态没有表单，变量由用户消息给出。留着占位符，模型会对着 `{{sql}}` 反问
/// 「请提供 SQL」—— 等于把用户又踢回一次表单。
pub(crate) fn strip_placeholders(text: &str) -> String {
    let mut out = text.to_string();
    while let Some(start) = out.find("{{") {
        let rest = &out[start..];
        let Some(end) = rest.find("}}") else { break };
        out.replace_range(start..start + end + 2, "");
    }
    out.trim().to_string()
}

/// 插件名按界面语言挑一个（没起名就用 id）
fn display_name(manifest: &Manifest, lang: Option<&str>) -> String {
    manifest
        .name
        .as_ref()
        .map(|n| n.pick(lang.unwrap_or("zh")))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| manifest.id.clone())
}

/// 把一个已装技能读成一段 system 文本：优先 `chatPrompt`，否则退回 `prompt` 并清洗占位符。
fn skill_prompt_text(dir: &Path, manifest: &Manifest) -> Option<String> {
    let chat_file = manifest
        .chat_prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let file = chat_file.clone().or_else(|| manifest.prompt.clone())?;
    let text = std::fs::read_to_string(dir.join(&file)).ok()?;
    let text = if chat_file.is_some() {
        // 作者专门写的对话提示词：原样使用（里面不该再有表单变量）
        text.trim().to_string()
    } else {
        // 复用任务模板：中间的 {{sql}} 之类换成一句说明，首尾空行压掉
        strip_placeholders(&text)
            .split('\n')
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(MAX_SKILL_PROMPT_CHARS).collect())
}

/// 把对话里选中的插件技能拼进 system。
///
/// 这是技能**唯一**的执行方式（没有独立运行界面）：提示词是持续生效的人格与方法，
/// 之后每一轮提问都带着它，直到用户把技能从输入框上摘掉。
///
/// 找不到 / 已停用 / 不是 skill 的 id 一律静默跳过 —— 插件被卸载了不该把整条对话打成 500。
pub fn augment_system(base: String, ids: &[String], lang: Option<&str>) -> String {
    if ids.is_empty() {
        return base;
    }
    let disabled = disabled_ids();
    let mut parts: Vec<String> = Vec::new();
    let mut budget = MAX_SKILL_PROMPT_CHARS * MAX_SKILLS_PER_TURN / 2;
    for id in ids.iter().take(MAX_SKILLS_PER_TURN) {
        if !id_ok(id) || disabled.contains(id) {
            continue;
        }
        let dir = root().join(id);
        if !dir.is_dir() {
            continue;
        }
        let Ok(manifest) = read_manifest(&dir) else { continue };
        if manifest.kind != "skill" {
            continue;
        }
        let Some(text) = skill_prompt_text(&dir, &manifest) else { continue };
        if text.len() > budget {
            tracing::warn!(id, len = text.len(), "技能提示词超预算，本次跳过");
            break;
        }
        budget -= text.len();
        parts.push(format!("### 技能：{}\n{}", display_name(&manifest, lang), text));
    }
    if parts.is_empty() {
        return base;
    }
    format!(
        "{base}\n\n用户已启用以下技能，请按它的要求处理用户消息；\
变量类输入（如「待审查的 SQL」）由用户消息给出，不要反问占位符。\n\n{}",
        parts.join("\n\n")
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_rules() {
        assert!(id_ok("sql-review"));
        assert!(id_ok("acme.sql_review-2"));
        assert!(!id_ok("SQL-Review"), "大写会和目录/大小写不敏感文件系统打架");
        assert!(!id_ok("../escape"));
        assert!(!id_ok(".hidden"));
        assert!(!id_ok(""));
    }

    #[test]
    fn chat_mode_strips_form_placeholders() {
        // 对话形态没有表单：{{sql}} 留着会被模型当成「让用户填」的占位符反问回去
        let text = "审查下面这条：\n```sql\n{{sql}}\n```\n关注点：{{focus}}";
        let out = strip_placeholders(text);
        assert!(!out.contains("{{"), "{out}");
        assert!(out.contains("审查下面这条"), "{out}");
    }

    #[test]
    fn localized_pick_falls_back() {
        let one = Localized::Text("纯中文".into());
        assert_eq!(one.pick("en"), "纯中文");

        let mut map = BTreeMap::new();
        map.insert("zh".to_string(), "中文名".to_string());
        let two = Localized::Map(map);
        assert_eq!(two.pick("zh-CN"), "中文名");
        assert_eq!(two.pick("en"), "中文名", "只有中文时英文界面也用它，而不是空字符串");
    }

    #[test]
    fn zip_with_executable_is_rejected() {
        let dir = std::env::temp_dir().join(format!("dbmind-plugin-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
            writer.start_file("manifest.json", opts).unwrap();
            std::io::Write::write_all(&mut writer, br#"{"schemaVersion":1,"id":"x","type":"skill"}"#).unwrap();
            writer.start_file("evil.ps1", opts).unwrap();
            std::io::Write::write_all(&mut writer, b"rm -rf /").unwrap();
            writer.finish().unwrap();
        }
        let err = extract_zip(&buf, &dir).unwrap_err();
        assert!(err.message.contains("可执行文件"), "{}", err.message);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
