//! 口令的落盘形态：`enc:v2:` 前缀 + AES-256-GCM（随机 nonce，密钥在 `app_settings` 里）。
//!
//! 为什么要加密：`~/.dbmind/dbmind.db` 就是个普通 SQLite 文件，用户（或任何拿到这台机器
//! 的人）用 DBMind 自己的 SQLite 连接、或用别的工具打开，`select password from connections`
//! 直接就是明文 —— 界面上写着「已保存」，实际上等于没存。
//!
//! 四条纪律：
//!   1. **只加密落盘形态**。内存里的 `ConnectionConfig.password` 永远是明文，业务侧
//!      （连接、备份、同步、AI 问数）一行都不用改；加解密只发生在 [`crate::storage::Store`]
//!      的读写边界上。
//!   2. **可逆**（不能是哈希）：口令要拿去认证。
//!   3. **带版本前缀**：没有前缀 = 历史明文，读到按明文用并在启动时迁移；`enc:v1:` = 早期
//!      的「确定性 nonce」密文（`mask::aes_encrypt`，同明文同密文），启动时顺手换成 v2。
//!   4. **v2 用随机 nonce**：同样的口令两次保存不该得到同样的密文，否则扫一眼文件就能看出
//!      「这两个连接用的是同一个口令」。
//!
//! 代价要讲清楚：密钥跟这台机器/这个用户绑定，**导出的密文换台机器解不开**（导入后等于
//! 没填口令）。这正是「导出不再带明文」的应有之义。

use base64::Engine as _;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};

use crate::mask::{self, MaskKey};

/// 当前密文前缀（随机 nonce）
pub const PREFIX: &str = "enc:v2:";
/// 早期密文前缀（确定性 nonce，仅用于兼容读取与迁移）
pub const PREFIX_V1: &str = "enc:v1:";

/// 密钥在 `app_settings` 里的键名。**别跟脱敏的 `ai.maskKey` 共用**：那个键是脱敏规则的
/// 配置项，用户在界面上可以改；改一次就会把所有连接口令解不开。
pub const KEY_SETTING: &str = "app.secretKey";

/// 已经是密文？（`enc:v1:` 也算 —— 只是还没升级到 v2）
pub fn is_encrypted(stored: &str) -> bool {
    stored.starts_with(PREFIX) || stored.starts_with(PREFIX_V1)
}

/// 是否已经是**当前版本**的密文（迁移时用来判断要不要重写这一行）
pub fn is_current(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

/// 明文 → 密文（`enc:v2:<base64(nonce||密文+tag)>`，nonce 每次随机）
pub fn encode(plain: &str, key: &MaskKey) -> String {
    let body = seal(key, plain)
        // ring 不可用时退回确定性版本：仍然是密文，只是同样的值会长得一样
        .unwrap_or_else(|| mask::aes_encrypt(key, plain));
    format!("{PREFIX}{body}")
}

/// 密文 → 明文；不是密文 / 解不开（换机器、换密钥）时返回 `None`
pub fn decode(stored: &str, key: &MaskKey) -> Option<String> {
    if let Some(body) = stored.strip_prefix(PREFIX) {
        return open(key, body);
    }
    if let Some(body) = stored.strip_prefix(PREFIX_V1) {
        return mask::aes_decrypt(key, body);
    }
    None
}

/// 幂等加密：已经是密文（v1/v2）的原样返回。空串也原样返回（「空口令」是有意义的状态）。
pub fn encode_if_plain(stored: &str, key: &MaskKey) -> String {
    if stored.is_empty() || is_encrypted(stored) {
        stored.to_string()
    } else {
        encode(stored, key)
    }
}

/// 落盘值 → 明文。历史明文（无前缀）原样返回 —— 向后兼容，也保证迁移前后的读路径一致。
pub fn decode_or_plain(stored: &str, key: &MaskKey) -> String {
    if !is_encrypted(stored) {
        return stored.to_string();
    }
    // 解不开时返回空串而不是密文：把密文当口令送进驱动只会得到一个莫名奇妙的认证失败
    decode(stored, key).unwrap_or_default()
}

/// `nonce(12) || ciphertext+tag` → base64，nonce 每次随机
fn seal(key: &MaskKey, text: &str) -> Option<String> {
    let rng = ring::rand::SystemRandom::new();
    let mut nonce = [0u8; 12];
    ring::rand::SecureRandom::fill(&rng, &mut nonce).ok()?;
    let less = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
    let mut body = text.as_bytes().to_vec();
    less.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut body)
        .ok()?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&body);
    Some(base64::engine::general_purpose::STANDARD.encode(out))
}

fn open(key: &MaskKey, body: &str) -> Option<String> {
    // 布局与 v1 相同（nonce 在前），可直接复用 mask 的解密
    mask::aes_decrypt(key, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_idempotent() {
        let key = mask::key_from_text("unit-test-key");
        let enc = encode("Dbmind@123", &key);
        assert!(is_encrypted(&enc) && is_current(&enc));
        assert_eq!(decode(&enc, &key).as_deref(), Some("Dbmind@123"));
        // 已加密的不再二次加密（导入密文、重复保存都走这条）
        assert_eq!(encode_if_plain(&enc, &key), enc);
        // 空串保持空串
        assert_eq!(encode_if_plain("", &key), "");
        // 历史明文原样读
        assert_eq!(decode_or_plain("plain-pwd", &key), "plain-pwd");
    }

    #[test]
    fn same_plaintext_gets_different_ciphertext() {
        let key = mask::key_from_text("unit-test-key");
        // 随机 nonce：同一口令两次加密结果必须不同（否则一眼能看出两个连接共用口令）
        assert_ne!(encode("same", &key), encode("same", &key));
    }

    #[test]
    fn legacy_v1_still_decrypts_and_upgrades() {
        let key = mask::key_from_text("unit-test-key");
        let v1 = format!("{PREFIX_V1}{}", mask::aes_encrypt(&key, "old-pwd"));
        assert!(is_encrypted(&v1) && !is_current(&v1));
        assert_eq!(decode(&v1, &key).as_deref(), Some("old-pwd"));
        // 迁移后应是 v2 且仍能解回原文
        let upgraded = encode(&decode_or_plain(&v1, &key), &key);
        assert!(is_current(&upgraded));
        assert_eq!(decode(&upgraded, &key).as_deref(), Some("old-pwd"));
    }

    #[test]
    fn wrong_key_degrades_to_empty() {
        let a = mask::key_from_text("key-a");
        let b = mask::key_from_text("key-b");
        let enc = encode("secret", &a);
        assert_eq!(decode(&enc, &b), None);
        assert_eq!(decode_or_plain(&enc, &b), "");
    }
}
