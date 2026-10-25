//! **`LOAD DATA LOCAL INFILE` 快通道**（MySQL / MariaDB 目标，纯插入场景）。
//!
//! ## 为什么值得单独一条路
//!
//! 目标端每行一条多值 `INSERT` 时，服务端要逐行解析 SQL、逐行走一遍优化/执行器；
//! 同样的数据走 `LOAD DATA LOCAL INFILE`，服务端只做「读一行、塞一行」，同机实测
//! 5~8 万行/秒，而多值 INSERT 只有 2~3 万 —— 这是整条同步链路里最大的一笔开销。
//!
//! ## 只对「纯插入」有意义
//!
//! `LOAD DATA` 不会更新已有行：upsert 必须继续走 `INSERT … on duplicate key update`。
//! 所以是否启用由调用方判断（见 `writer::SqlWriter::write` 的 `UpsertMode::Off` 条件），
//! 本模块只负责「怎么把行编码成 TSV / 怎么发」。
//!
//! ## 编码规则（与 MySQL 的字段解析严格对齐）
//!
//! | 值 | 文本形态 |
//! |---|---|
//! | NULL | `\N`（整格只有这两个字符） |
//! | `\` `\t` `\n` `\r` `\0` `\x1a` | `\\` `\t` `\n` `\r` `\0` `\Z` |
//! | 数字 / 日期时间 / 定点数 | 原文本（时间精度到微秒，不做任何取舍） |
//! | 二进制列 | **大写十六进制**，语句里 `col = unhex(@cn)` |
//!
//! 二进制必须走 hex 的原因：TSV 文件按 UTF-8 写，任意字节（`0xFF`、`0x80`）没法直接
//! 塞进文本；实测直接塞会被服务端按字符集「纠正」成 `C3BF`/`C280`（真机验证过）。
//! hex 全是 ASCII，`unhex` 落库字节与源端完全一致（含 NUL、BLOB、`bit`）。
//!
//! 语句统一用「用户变量 + `SET` 赋值」形态：所有列先读进 `@c0..@cn`，再逐列赋值。
//! 这样二进制列与非二进制列只有一处差异（`unhex(@cn)`），也就不必为「有没有 BLOB」
//! 维护两套语句模板。

use crate::api::error::{XError, XResult};
use crate::api::AppState;
use dbmind_core::CellValue;
use crate::api::dialect::Dialect;

/// 单个请求（一次 `LOAD DATA`）的数据量上限。
///
/// 为什么不跟着「每批条数」走：`LOAD DATA` 的收益来自「一次多灌」，按 1000 行拆会把
/// 收益抵消掉大半（每批一次 RPC + 一次临时文件落盘）。按字节切更贴合链路：
/// 4MB 的 TSV 在千兆/VPN 上都是「几秒内能传完、丢了也只重传这一批」的量级。
pub(crate) const LOAD_DATA_CHUNK_BYTES: usize = 4 * 1024 * 1024;

/// 一次 `LOAD DATA` 最多带多少行（防止极窄行把单条语句堆到几十万行）。
pub(crate) const LOAD_DATA_CHUNK_ROWS: usize = 20_000;

/// 每个目标列是不是二进制类型（决定走 hex + `unhex`）。
pub(crate) fn binary_columns(types: &[Option<String>]) -> Vec<bool> {
    types
        .iter()
        .map(|type_name| {
            type_name
                .as_deref()
                .map(Dialect::is_binary_type)
                .unwrap_or(false)
        })
        .collect()
}

/// 拼 `LOAD DATA` 语句：`{file}` 由宿主替换成真实的临时文件路径。
pub(crate) fn build_sql(
    target_name: &str,
    columns: &[String],
    binary: &[bool],
    kind: Dialect,
) -> String {
    let vars = (0..columns.len())
        .map(|index| format!("@c{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sets = columns
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let quoted = kind.quote(name);
            if binary.get(index).copied().unwrap_or(false) {
                format!("{quoted} = unhex(@c{index})")
            } else {
                format!("{quoted} = @c{index}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    // 注意转义层级：这里写给 MySQL 看的是 `'\t'` / `'\\'` / `'\n'`，
    // 所以 Rust 源码里要再各多一个反斜杠。
    format!(
        "load data local infile '{{file}}' into table {target_name} character set utf8mb4 \
         fields terminated by '\\t' escaped by '\\\\' lines terminated by '\\n' \
         ({vars}) set {sets}"
    )
}

/// 把一批行编码成 TSV（UTF-8 文本）。
///
/// 返回 `Err` 表示「这批行不适合走快通道」（有超大二进制值没取回内容、或文本列里
/// 出现了二进制值）—— 调用方直接回退 INSERT，不当作失败。
pub(crate) fn encode_rows(rows: &[Vec<CellValue>], binary: &[bool]) -> Result<String, String> {
    // 粗估一下容量，省掉反复扩容（每格按 16 字节估已经偏大，够用）
    let mut out = String::with_capacity(rows.len() * 64);
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index > 0 {
                out.push('\t');
            }
            let is_binary = binary.get(index).copied().unwrap_or(false);
            encode_cell(&mut out, cell, is_binary)?;
        }
        out.push('\n');
    }
    Ok(out)
}

fn encode_cell(out: &mut String, cell: &CellValue, binary: bool) -> Result<(), String> {
    match cell {
        CellValue::Null => out.push_str("\\N"),
        // 超过单值上限的二进制只带回了长度：内容不在内存里，写过去只会是错数据
        CellValue::Blob { len } => {
            return Err(format!(
                "有 {len} 字节的二进制值未取回内容（超过单值上限），无法走快速装载"
            ))
        }
        CellValue::Bytes(bytes) if binary => push_hex(out, bytes.as_slice()),
        CellValue::Bytes(_) => {
            return Err("文本列里出现了二进制值，无法走快速装载".to_string())
        }
        CellValue::Integer(value) => out.push_str(&value.to_string()),
        CellValue::Real(value) => out.push_str(&value.to_string()),
        CellValue::Decimal(text) | CellValue::Text(text) => push_escaped(out, text),
    }
    Ok(())
}

/// 大写十六进制（`unhex` 对大小写不敏感，大写便于肉眼核对）。
fn push_hex(out: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.reserve(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
}

/// 按 `LOAD DATA` 的转义规则写一段文本。
fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            '\u{1a}' => out.push_str("\\Z"),
            _ => out.push(ch),
        }
    }
}

/// 在目标会话上跑一批 `LOAD DATA`，返回服务端报告的行数。
///
/// 错误原样往上抛：调用方按错误文案区分「语句没生效（可安全回退 INSERT）」与
/// 「结果未知（不能重试）」，见 `writer::SqlWriter::try_load_data`。
pub(crate) async fn load_data_local(
    state: &AppState,
    target: &str,
    session: &str,
    sql: String,
    tsv: String,
    timeout_ms: u64,
) -> XResult<u64> {
    let request = dbmind_core::QueryRequest {
        schema: None,
        read_only: None,
        connection: target.to_string(),
        sql,
        options: dbmind_core::QueryOptions {
            max_rows: 1,
            timeout_ms,
        },
        execution_id: None,
        session: Some(session.to_string()),
        // 数据同步是界面功能驱动的批量写入：不进查询历史
        internal: true,
    };
    let engine = state.engine();
    let result = crate::api::blocking(move || {
        engine.execute_load_data_in(request, tsv, dbmind_core::AccessContext::Web)
    })
    .await?;
    Ok(result.affected_rows.unwrap_or(0) as u64)
}

/// 宿主回报的「语句已失败、这批行没进去」标记（见 agent-jdbc 的 `loadDataLocal`）。
///
/// 只有看到它才允许回退 INSERT：其它错误（连接中断、超时）可能已经写进去了一半，
/// 再写一遍就是重复数据。
pub(crate) const LOAD_DATA_ROLLED_BACK: &str = "LOAD_DATA_ROLLED_BACK";

/// 这条错误是不是「可以安全回退 INSERT」。
pub(crate) fn safe_to_fallback(err: &XError) -> bool {
    err.message.contains(LOAD_DATA_ROLLED_BACK)
}

/// 把宿主回报整理成**给用户看**的一句话：剥掉内部标记，再补一条可操作的建议。
///
/// 这条消息会原样进任务日志，所以既不能露出 `LOAD_DATA_ROLLED_BACK` 这种内部约定，
/// 也不能只写「local data is disabled」——用户不知道该怎么办。
pub(crate) fn user_message(message: &str) -> String {
    let body = message
        .find(LOAD_DATA_ROLLED_BACK)
        .map(|at| &message[at + LOAD_DATA_ROLLED_BACK.len()..])
        .unwrap_or(message)
        .trim_start_matches(['：', ':', ' ', '\t']);
    let lower = body.to_ascii_lowercase();
    if lower.contains("local data is disabled")
        || lower.contains("local_infile")
        || lower.contains("disabled the local infile capability")
    {
        return format!(
            "{body}（目标库需开启 local_infile：SET GLOBAL local_infile = 1 后重跑即可提速）"
        );
    }
    body.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> CellValue {
        CellValue::Text(value.to_string())
    }

    #[test]
    fn 转义对齐_load_data_规则() {
        let rows = vec![vec![
            text("a\tb"),
            text("l1\nl2\r\\x"),
            CellValue::Null,
            text("\\N-as-text"),
        ]];
        let encoded = encode_rows(&rows, &[false, false, false, false]).unwrap();
        assert_eq!(encoded, "a\\tb\tl1\\nl2\\r\\\\x\t\\N\t\\\\N-as-text\n");
    }

    #[test]
    fn 二进制列走十六进制() {
        let rows = vec![vec![
            CellValue::Integer(7),
            CellValue::Bytes(dbmind_core::BlobBytes(vec![0x00, 0xff, 0x80])),
            text("中文"),
        ]];
        let encoded = encode_rows(&rows, &[false, true, false]).unwrap();
        assert_eq!(encoded, "7\t00FF80\t中文\n");
    }

    #[test]
    fn 超出单值上限的二进制回退() {
        let rows = vec![vec![CellValue::Blob { len: 9_000_000 }]];
        assert!(encode_rows(&rows, &[true]).is_err());
    }

    #[test]
    fn 日志文案剥掉内部标记并补可操作建议() {
        // MySQL 8 默认 local_infile=OFF，原文只有「Loading local data is disabled」，
        // 用户在任务日志里看到这句不知道该干嘛
        let text = user_message(
            "LOAD_DATA_ROLLED_BACK：Loading local data is disabled; this must be enabled on both the client and server sides",
        );
        assert!(!text.contains(LOAD_DATA_ROLLED_BACK), "内部标记不该出现在日志里：{text}");
        assert!(text.starts_with("Loading local data is disabled"));
        assert!(text.contains("SET GLOBAL local_infile = 1"));
        // 其它错误原样返回，不硬塞无关建议
        assert_eq!(
            user_message("LOAD_DATA_ROLLED_BACK：Duplicate entry '1' for key 'PRIMARY'"),
            "Duplicate entry '1' for key 'PRIMARY'"
        );
    }

    #[test]
    fn 语句形状_含_文件占位与_unhex() {
        let columns = vec!["id".to_string(), "note".to_string(), "bin".to_string()];
        let sql = build_sql(
            "`demo`.`t`",
            &columns,
            &[false, false, true],
            Dialect::new(dbmind_core::ConnectionKind::Mysql),
        );
        assert!(sql.contains("load data local infile '{file}' into table `demo`.`t`"));
        assert!(sql.contains("fields terminated by '\\t' escaped by '\\\\' lines terminated by '\\n'"));
        // 普通名不加引号（与 INSERT 生成器同一套 quote 规则），二进制列走 unhex
        assert!(
            sql.contains("(@c0, @c1, @c2) set id = @c0, note = @c1, bin = unhex(@c2)"),
            "生成的语句是：{sql}"
        );
    }
}
