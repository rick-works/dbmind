//! **字段脱敏**：把指定的表 / 字段换成摘要或密文（MD5 / SHA-256 / AES）。
//!
//! 放在 core 而不是 web：同一套规则要喂给四个出口 ——
//! 发给大模型的内容（web 的上下文与智能体）、MCP 服务返回的数据（dbmind-mcp）、
//! 界面里的结果表格（前端调同一套算法）、导出的文件（web 的 export）。
//! 四处各写一份判定，迟早会漂移成「配置了却没生效」，而那种 bug 用户只会说「脱敏是假的」。
//!
//! ## 处理方式（都是常见做法）
//!
//! | 方式 | 结果 | 什么时候用 |
//! |---|---|---|
//! | `md5`（默认） | `d41d8cd98f00b204e9800998ecf8427e` | 最常用：同值同摘要、不可反推 |
//! | `sha256` | `ba7816bf…15ad`（64 位） | 同 MD5 但更放心 |
//! | `aes` | `q8Zk…`（base64，AES-256-GCM） | 需要**能解回原文**：密钥在本机，同值同密文 |
//! | `star` | `***` | 只关心「这列被遮了」，连长度都不透露 |
//! | `last4` | `****1234` | 卡号 / 账号，只留尾号 |
//!
//! ## 四个出口（`MaskChannel`）
//!
//! - `Llm`：AI 上下文里的样例行 + 智能体的只读查询结果；
//! - `Mcp`：MCP 工具（query / sample）返回给外部客户端的数据；
//! - `Grid`：DBMind 自己的结果表格（只为「别被人一眼看到」，不影响复制）；
//! - `Export`：导出的文件（CSV / Excel / JSON / SQL 脚本）里落盘的值。
//!
//! ## 刻意的取舍
//!
//! 1. **摘要不可逆，AES 可逆但密钥只在本地**。
//! 2. **同值同结果**：AES 用「nonce = SHA-256(key‖0x00‖明文) 前 12 字节」做确定性加密，
//!    否则同一列里每个值都变样，连「两行是同一个手机号」都看不出来。
//! 3. **表名 / 列名 / 注释不脱敏**：脱了模型就不知道该查哪张表。
//! 4. **SQL 里手写的字面量不管**：那是用户自己的原文，硬猜只会改坏 SQL。
//!
//! ## 规则怎么匹配
//!
//! 一条规则 = 连接（空=全部）/ 库（空=全部）/ 模式（空=全部）/ 表（空=任意表）/
//! 列（必填，支持 `*` 通配）。表钉死的规则在不知道表名时**不命中** —— 套到别的表上
//! 会把正常字段也打码，让模型失明。

use serde::{Deserialize, Serialize};

// 两个 digest 版本（md-5 用 0.11、sha2 用 0.10）都叫 Digest，匿名导入互不遮蔽
use md5::Digest as _;
use sha2::Digest as _;

/// 设置键：整份配置存 `ai.maskRules`。
pub const KEY: &str = "ai.maskRules";

/// 设置键：AES 密钥文本（默认是 base64 的 32 字节，用户也可以改成一个口令）。
///
/// 跟规则分开存：换规则不该换密钥 —— 密钥一变，之前发出去的密文就再也对不上了。
pub const KEY_AES: &str = "ai.maskKey";

/// 四个出口，各自一个开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskChannel {
    /// 发给大模型的内容
    Llm,
    /// MCP 服务返回给外部客户端的内容
    Mcp,
    /// DBMind 界面里的结果表格
    Grid,
    /// 导出的文件（CSV / Excel / JSON / SQL 脚本）
    Export,
}

/// 处理方式。缺省 `md5`：老规则（没有这个字段、或写了已下线的 `mask`/`hash`）
/// 读回来一律按 MD5 处理，正好是「把值换成一个不像原文的摘要」这个本意。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MaskMethod {
    /// MD5 摘要（32 位小写十六进制）
    #[default]
    #[serde(alias = "mask", alias = "hash")]
    Md5,
    /// SHA-256 摘要（64 位小写十六进制）
    Sha256,
    /// AES-256-GCM 加密（base64，含 12 字节 nonce，可用本机密钥解回原文）
    Aes,
    /// 全遮 `***`
    #[serde(alias = "fixed")]
    Star,
    /// 只留后 4 位：`****1234`
    Last4,
}

impl MaskMethod {
    /// 这种处理方式要不要用到 AES 密钥（用来避免「没人选 AES 也生成一把密钥」）
    pub fn needs_key(self) -> bool {
        matches!(self, MaskMethod::Aes)
    }
}

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
    /// 模式（PostgreSQL / SQL Server / Oracle 等有 Schema 层的引擎）；空 = 所有模式
    #[serde(default)]
    pub schema: Option<String>,
    /// 空 = 任意表（拿不到表名的工具结果靠这一档兜底）
    #[serde(default)]
    pub table: Option<String>,
    /// 列名，支持 `*`（全部）、`*_phone`（后缀）、`phone*`（前缀）
    pub column: String,
    /// 处理方式，缺省 MD5
    #[serde(default)]
    pub method: MaskMethod,
    /// 备注（为什么脱敏这一列），只给人看
    #[serde(default)]
    pub note: Option<String>,
}

/// 整个脱敏配置：四个开关 + 一份规则。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskConfig {
    /// 发给大模型的脱敏（旧的单开关 `enabled` 折算到这里，见 [`MaskConfig::normalize`]）
    #[serde(default)]
    pub llm: bool,
    /// MCP 服务的脱敏
    #[serde(default)]
    pub mcp: bool,
    /// 结果表格的脱敏（只改显示）
    #[serde(default)]
    pub grid: bool,
    /// 导出文件的脱敏
    #[serde(default)]
    pub export: bool,
    /// v1.8.x 的单开关：只读不写，读到 true 就并进 `llm`
    #[serde(default, skip_serializing)]
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

/// 把 `库.模式` 拆开。
///
/// 后端表级接口的约定是「有模式层的引擎用 `database=库.模式`」，所以调用方拿到的
/// `database` 可能已经带着模式；规则里用户也可能把库写成 `库.模式`。两边按同一规则拆，
/// 才不会出现「界面上配了却不生效」。
fn split_db(database: &str) -> (&str, Option<&str>) {
    match database.split_once('.') {
        Some((db, schema)) if !db.trim().is_empty() && !schema.trim().is_empty() => {
            (db, Some(schema))
        }
        _ => (database, None),
    }
}

impl MaskRule {
    /// 这条规则是否命中「某连接 / 某库（可带模式）/ 某表（可未知）的某一列」。
    pub fn hits(&self, conn: &str, database: &str, table: Option<&str>, column: &str) -> bool {
        if let Some(want) = clean(self.connection_id.as_deref()) {
            if !same_name(want, conn) {
                return false;
            }
        }
        let (got_db, got_schema) = split_db(database);
        if let Some(want) = clean(self.database.as_deref()) {
            let (want_db, want_schema) = split_db(want);
            if !same_name(want_db, got_db) {
                return false;
            }
            if let Some(want_schema) = want_schema {
                // 规则把模式写在库名里（`demo.dbo`）：调用方也必须带着模式
                if !got_schema.is_some_and(|got| same_name(want_schema, got)) {
                    return false;
                }
            }
        }
        if let Some(want) = clean(self.schema.as_deref()) {
            // 单独的模式字段：调用方没带模式就不命中（不能假设是哪个模式）
            if !got_schema.is_some_and(|got| same_name(want, got)) {
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
            // 表为空的规则：任意表都算，工具结果走这一档
            (None, _) => {}
        }
        pattern_match(&self.column, column)
    }
}

impl MaskConfig {
    /// 读老配置后调用：把已废弃的单开关并进 `llm`。
    pub fn normalize(&mut self) {
        if self.enabled {
            self.llm = true;
            self.enabled = false;
        }
    }

    /// 某个出口的开关是否打开。
    pub fn channel_on(&self, channel: MaskChannel) -> bool {
        match channel {
            MaskChannel::Llm => self.llm || self.enabled,
            MaskChannel::Mcp => self.mcp,
            MaskChannel::Grid => self.grid,
            MaskChannel::Export => self.export,
        }
    }

    /// 任何一个出口打开就值得取 AES 密钥。
    pub fn any_on(&self) -> bool {
        self.llm || self.mcp || self.grid || self.export || self.enabled
    }

    /// 这一列在这个出口要不要脱敏；要的话用哪种方式。
    ///
    /// 返回 `Option<MaskMethod>` 而不是 bool：一条列可能被多条规则命中（比如「任意表
    /// 的 `*phone`」和「users 表的 `phone`」），取**第一条**命中的方式，顺序即优先级
    /// —— 用户想覆盖时把更具体的规则挪到前面即可。
    pub fn method_for(
        &self,
        channel: MaskChannel,
        conn: &str,
        database: &str,
        table: Option<&str>,
        column: &str,
    ) -> Option<MaskMethod> {
        if !self.channel_on(channel) {
            return None;
        }
        self.rules
            .iter()
            .find(|rule| rule.hits(conn, database, table, column))
            .map(|rule| rule.method)
    }

    /// 是否有规则用到 AES（决定要不要生成/读取密钥）
    pub fn uses_aes(&self) -> bool {
        self.rules.iter().any(|rule| rule.method.needs_key())
    }
}

/// 32 字节密钥。只在进程内传递，不写日志、不进 JSON。
pub type MaskKey = [u8; 32];

/// 小写十六进制。自己拼而不是 `format!("{:x}", digest)`：两个 digest 版本的输出类型
/// （hybrid-array / generic-array）都没有实现 LowerHex，一个能用一个不能用，干脆统一。
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// MD5 摘要（32 位小写十六进制）。与 MySQL `md5()`、`DigestUtils.md5Hex()` 输出一致。
fn md5_hex(text: &str) -> String {
    to_hex(&md5::Md5::digest(text.as_bytes()))
}

/// SHA-256 摘要（64 位小写十六进制）。
fn sha256_hex(text: &str) -> String {
    to_hex(&sha2::Sha256::digest(text.as_bytes()))
}

/// 派生 nonce：`SHA-256(key || 0x00 || 明文)` 的前 12 字节。
///
/// 中间那个 0x00 是域分隔：没有它，`key="ab", text="c"` 与 `key="a", text="bc"`
/// 会算出同一个 nonce（这里 key 定长，不会真撞上，但写完不用再想这件事）。
fn derived_nonce(key: &MaskKey, text: &str) -> [u8; 12] {
    let mut hasher = sha2::Sha256::new();
    hasher.update(key);
    hasher.update([0u8]);
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&digest[..12]);
    nonce
}

/// AES-256-GCM 加密，输出 `base64(nonce || 密文+tag)`。
fn aes_encrypt(key: &MaskKey, text: &str) -> String {
    use base64::Engine as _;
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
    let nonce = derived_nonce(key, text);
    let sealed = (|| {
        let less = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
        let mut body = text.as_bytes().to_vec();
        less.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut body)
            .ok()?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&body);
        Some(out)
    })();
    match sealed {
        Some(out) => base64::engine::general_purpose::STANDARD.encode(out),
        // 密钥定长 32 字节，正常走不到这里。真走到了也宁可退成摘要，绝不把明文送出去。
        None => {
            tracing::warn!("AES-256-GCM 不可用，该值退回 SHA-256 摘要");
            sha256_hex(text)
        }
    }
}

/// 解密（自检与「我需要拿回原文」时用）：与 [`aes_encrypt`] 对称。
pub fn aes_decrypt(key: &MaskKey, encoded: &str) -> Option<String> {
    use base64::Engine as _;
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
    let buf = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    if buf.len() <= 12 + 16 {
        return None;
    }
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&buf[..12]);
    let mut body = buf[12..].to_vec();
    let less = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
    let plain = less
        .open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut body)
        .ok()?;
    String::from_utf8(plain.to_vec()).ok()
}

/// 把设置里存的那串文本变成 32 字节密钥。
///
/// 默认生成的形态是 base64 的 32 字节随机数，遇到这种就直接解出来；
/// 用户自己改成了别的文本（口令）时按 SHA-256 派生 —— 这样「可修改」才真的可用，
/// 而不必强求用户去填 44 个字符的 base64。
pub fn key_from_text(text: &str) -> MaskKey {
    use base64::Engine as _;
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(text.trim()) {
        if let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return key;
        }
    }
    let digest = sha2::Sha256::digest(text.trim().as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

/// 生成一把默认密钥文本（base64 的 32 字节随机数）。
pub fn random_key_text() -> String {
    use base64::Engine as _;
    use ring::rand::{SecureRandom, SystemRandom};
    let mut key = [0u8; 32];
    if SystemRandom::new().fill(&mut key).is_err() {
        // 系统熵源不可用的极端情况：退回时间戳派生，仍然只在本机使用，且不至于开不了功能
        tracing::warn!("系统随机数不可用，AES 密钥退化生成");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let digest = sha2::Sha256::digest(nanos.to_string().as_bytes());
        key.copy_from_slice(&digest);
    }
    base64::engine::general_purpose::STANDARD.encode(key)
}

/// 把单个值换成代换值。
///
/// 空值一律保持空：`NULL`/空串本身没有信息可泄露，改写成 `***` 反而会让模型
/// 以为「这列有值但被遮了」，对判断可空性产生误导。
pub fn mask_value(raw: &str, method: MaskMethod, key: &MaskKey) -> String {
    let text = raw.trim();
    if text.is_empty() {
        return String::new();
    }
    match method {
        MaskMethod::Md5 => md5_hex(text),
        MaskMethod::Sha256 => sha256_hex(text),
        MaskMethod::Aes => aes_encrypt(key, text),
        MaskMethod::Star => "***".to_string(),
        MaskMethod::Last4 => {
            let len = text.chars().count();
            if len <= 4 {
                return "***".to_string();
            }
            let tail: String = text.chars().skip(len - 4).collect();
            format!("****{tail}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: MaskKey = [7u8; 32];

    fn tmask(raw: &str, method: MaskMethod) -> String {
        mask_value(raw, method, &TEST_KEY)
    }

    fn rule(conn: Option<&str>, db: Option<&str>, table: Option<&str>, column: &str) -> MaskRule {
        MaskRule {
            connection_id: conn.map(str::to_string),
            database: db.map(str::to_string),
            schema: None,
            table: table.map(str::to_string),
            column: column.to_string(),
            method: MaskMethod::Md5,
            note: None,
        }
    }

    #[test]
    fn md5_matches_well_known_vectors() {
        assert_eq!(
            tmask("abc", MaskMethod::Md5),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            tmask("message digest", MaskMethod::Md5),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(tmask("13812348000", MaskMethod::Md5).len(), 32);
        assert_eq!(tmask("13812348000", MaskMethod::Md5), tmask("13812348000", MaskMethod::Md5));
        assert_ne!(tmask("13812348000", MaskMethod::Md5), tmask("13812348001", MaskMethod::Md5));
        assert!(!tmask("13812348000", MaskMethod::Md5).contains("138"));
    }

    #[test]
    fn sha256_matches_well_known_vectors() {
        assert_eq!(
            tmask("abc", MaskMethod::Sha256),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(tmask("13812348000", MaskMethod::Sha256).len(), 64);
    }

    #[test]
    fn aes_is_deterministic_and_reversible() {
        let a = tmask("13812348000", MaskMethod::Aes);
        let b = tmask("13812348000", MaskMethod::Aes);
        let c = tmask("13812348001", MaskMethod::Aes);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(!a.contains("138"));
        assert_eq!(aes_decrypt(&TEST_KEY, &a).as_deref(), Some("13812348000"));
        assert_eq!(aes_decrypt(&[8u8; 32], &a), None);
        assert_eq!(tmask("", MaskMethod::Aes), "");
    }

    #[test]
    fn star_and_last4_behave() {
        assert_eq!(tmask("13812348000", MaskMethod::Star), "***");
        assert_eq!(tmask("6222021234567890123", MaskMethod::Last4), "****0123");
        assert_eq!(tmask("12", MaskMethod::Last4), "***");
    }

    #[test]
    fn aes_key_accepts_base64_or_passphrase() {
        use base64::Engine as _;
        let key = [9u8; 32];
        let text = base64::engine::general_purpose::STANDARD.encode(key);
        assert_eq!(key_from_text(&text), key);
        let a = key_from_text("my-passphrase");
        assert_eq!(a, key_from_text("my-passphrase"));
        assert_ne!(a, key);
        let cipher = mask_value("13812348000", MaskMethod::Aes, &a);
        assert_eq!(aes_decrypt(&a, &cipher).as_deref(), Some("13812348000"));
    }

    #[test]
    fn rule_matching_prefers_first_hit_and_respects_table() {
        let mut config = MaskConfig {
            llm: true,
            rules: vec![
                rule(None, None, None, "*phone"),
                rule(Some("c1"), Some("demo"), Some("users"), "phone"),
                rule(None, None, Some("orders"), "remark"),
            ],
            ..Default::default()
        };
        // 任意表的 *phone 先命中
        assert_eq!(
            config.method_for(MaskChannel::Llm, "c1", "demo", Some("users"), "phone"),
            Some(MaskMethod::Md5)
        );
        // 换出口就没开
        assert_eq!(
            config.method_for(MaskChannel::Mcp, "c1", "demo", Some("users"), "phone"),
            None
        );
        config.mcp = true;
        assert_eq!(
            config.method_for(MaskChannel::Mcp, "c1", "demo", Some("users"), "phone"),
            Some(MaskMethod::Md5)
        );
        // 钉表规则在不知道表名时不命中
        assert_eq!(
            config.method_for(MaskChannel::Mcp, "c1", "demo", None, "remark"),
            None
        );
        assert_eq!(
            config.method_for(MaskChannel::Mcp, "c1", "demo", Some("orders"), "remark"),
            Some(MaskMethod::Md5)
        );
    }

    #[test]
    fn legacy_single_switch_folds_into_llm() {
        let mut config: MaskConfig = serde_json::from_str(
            r#"{"enabled":true,"rules":[{"column":"a","method":"mask"},{"column":"b","method":"hash"},{"column":"c","method":"fixed"},{"column":"d"}]}"#,
        )
        .expect("老配置必须还能读");
        // 老配置里的 enabled 没折算前就已经算作 llm 打开
        assert!(config.channel_on(MaskChannel::Llm));
        assert_eq!(config.rules[0].method, MaskMethod::Md5);
        assert_eq!(config.rules[1].method, MaskMethod::Md5);
        assert_eq!(config.rules[2].method, MaskMethod::Star);
        assert_eq!(config.rules[3].method, MaskMethod::Md5);
        config.normalize();
        assert!(config.llm);
        assert!(!config.mcp && !config.grid);
        assert!(!config.uses_aes());
    }
}
