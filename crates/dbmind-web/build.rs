//! 从 `src/api/transfer/plugins/` 的**文件名**生成读写插件注册表。
//!
//! 约定：一种数据源就是两个文件
//!
//! ```text
//! plugins/<kind>_reader.rs   →  pub(crate) fn reader(plan) -> Box<dyn Reader>
//! plugins/<kind>_writer.rs   →  pub(crate) fn writer(plan) -> Arc<dyn Writer>  +  static SPEC: Spec
//! ```
//!
//! 生成到 `OUT_DIR/transfer_plugins.rs`，由 `plugins/mod.rs` `include!`：
//! `reader_plugin` / `writer_plugin` / `spec_plugin` 三张表 + `KINDS` 清单。
//!
//! 这样「新增一种数据库 = 加两个文件」才是真的零改动 —— 手写注册表迟早会漏一行，
//! 而漏了的表现是「同步能跑、但用的是通用参数」，界面上完全看不出来。

use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR 一定存在（cargo 提供）"),
    );
    let dir = manifest.join("src/api/transfer/plugins");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut readers: Vec<String> = Vec::new();
    let mut writers: Vec<String> = Vec::new();
    let entries = fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("读不到插件目录 {}：{err}", dir.display()));
    for entry in entries {
        let path = entry.expect("读取目录项失败").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".rs") || name == "mod.rs" {
            continue;
        }
        // 文件名 → (种类, 角色)。种类必须是单段标识符（不含下划线），否则拆不出来：
        // 这条限制写在 `plugins/mod.rs` 的模块文档里，这里只做硬校验，别让它悄悄错。
        if let Some(kind) = name.strip_suffix("_reader.rs") {
            assert_single_token(kind, name);
            readers.push(kind.to_string());
        } else if let Some(kind) = name.strip_suffix("_writer.rs") {
            assert_single_token(kind, name);
            writers.push(kind.to_string());
        } else {
            panic!("插件文件名必须形如 <kind>_reader.rs / <kind>_writer.rs：{name}");
        }
    }
    readers.sort();
    writers.sort();

    // 只有一侧存在的类型也生成模块声明与注册（另一侧注册表里缺失）——
    // 这样「只加了一个文件」会被 `每种数据源都有读写两个插件` 那条测试明确点名，
    // 而不是编译期报一句「模块不存在」让人猜。
    let mut kinds: Vec<&String> = readers.iter().chain(writers.iter()).collect();
    kinds.sort();
    kinds.dedup();

    let mut code = String::new();
    code.push_str("// 由 crates/dbmind-web/build.rs 生成，请勿手改。\n\n");

    // 模块声明必须带**绝对路径**：这份文件生成在 OUT_DIR，而 `mod foo;` 的相对路径是
    // 相对「声明所在文件」的目录解析的 —— 不加 `#[path]` 就会去 OUT_DIR 里找 .rs 文件。
    // 生成物不提交、每次构建重写，所以这里出现本机绝对路径是安全的（源码树保持干净）。
    let dir_text = dir.to_string_lossy().replace('\\', "/");
    for kind in &readers {
        code.push_str(&format!(
            "#[path = \"{dir_text}/{kind}_reader.rs\"]\npub(crate) mod {kind}_reader;\n"
        ));
    }
    for kind in &writers {
        code.push_str(&format!(
            "#[path = \"{dir_text}/{kind}_writer.rs\"]\npub(crate) mod {kind}_writer;\n"
        ));
    }
    code.push('\n');

    code.push_str("/// 已登记读插件的类型（源）。\npub(crate) fn reader_plugin(kind: &str) -> Option<ReaderFactory> {\n    Some(match kind {\n");
    for kind in &readers {
        code.push_str(&format!("        \"{kind}\" => {kind}_reader::reader,\n"));
    }
    code.push_str("        _ => return None,\n    })\n}\n\n");

    code.push_str("/// 已登记写插件的类型（目标）。\npub(crate) fn writer_plugin(kind: &str) -> Option<WriterFactory> {\n    Some(match kind {\n");
    for kind in &writers {
        code.push_str(&format!("        \"{kind}\" => {kind}_writer::writer,\n"));
    }
    code.push_str("        _ => return None,\n    })\n}\n\n");

    code.push_str("/// 各类型的搬运规格（写在写插件文件里）。\npub(crate) fn spec_plugin(kind: &str) -> Option<&'static Spec> {\n    Some(match kind {\n");
    for kind in &writers {
        code.push_str(&format!("        \"{kind}\" => &{kind}_writer::SPEC,\n"));
    }
    code.push_str("        _ => return None,\n    })\n}\n\n");

    code.push_str("/// 已登记的类型清单（测试与排障用）。\n#[allow(dead_code)]\npub(crate) const KINDS: &[&str] = &[\n");
    for kind in &kinds {
        code.push_str(&format!("    \"{kind}\",\n"));
    }
    code.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR 一定存在")).join("transfer_plugins.rs");
    fs::write(&out, code).unwrap_or_else(|err| panic!("写 {} 失败：{err}", out.display()));
}

fn assert_single_token(kind: &str, file: &str) {
    let ok = !kind.is_empty()
        && kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    assert!(
        ok,
        "插件文件名里的类型必须是小写单段标识符（不含下划线）：{file}"
    );
}
