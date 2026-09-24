//! `POST /api/ai/export/doc` —— 把 AI 的回答导出成文件。
//!
//! 支持 `md` / `xlsx` / `docx`；**`pdf` 不在范围内**（要内嵌中文字体 + 排版引擎，
//! 那是另一个量级的事），所以它得到一条明确的「尚未接入」，而不是一个损坏的文件。
//!
//! ## 两个实现要点
//!
//! 1. **xlsx 复用导出那套写出器**（`api::xlsx`）—— 同一个容器格式，没有理由写两遍。
//! 2. **docx 也是 zip 容器**：Word 的最小可打开文档只要三部件
//!    （`[Content_Types].xml` / `_rels/.rels` / `word/document.xml`）。
//!    比引入一个 Word 库轻得多，而且不会因为库里的小版本差异行为漂移。

use std::io::{Cursor, Write};

use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::api::error::{XError, XResult};
use crate::api::tasks;
use crate::api::xlsx::{SheetSpec, Workbook};

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DocRequest {
    #[serde(default)]
    title: String,
    #[serde(default)]
    markdown: String,
    #[serde(default)]
    format: String,
    #[serde(default)]
    file_name: String,
}

/// `POST /api/ai/export/doc`。
pub async fn export_doc(Json(req): Json<DocRequest>) -> XResult<Response> {
    let format = req.format.trim().to_ascii_lowercase();
    let content = req.markdown;
    if content.trim().is_empty() {
        return Err(XError::bad_request("没有可导出的内容"));
    }
    let title = if req.title.trim().is_empty() {
        "AI 回答".to_string()
    } else {
        req.title.clone()
    };
    let base = if req.file_name.trim().is_empty() {
        format!("{}_{}", safe_name(&title), tasks::stamp())
    } else {
        safe_name(&req.file_name)
    };

    match format.as_str() {
        "" | "md" | "markdown" => Ok(binary(
            content.into_bytes(),
            "text/markdown; charset=UTF-8",
            &format!("{base}.md"),
        )),
        "xlsx" | "excel" => {
            let bytes = markdown_workbook(&title, &content)?;
            Ok(binary(
                bytes,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                &format!("{base}.xlsx"),
            ))
        }
        "docx" | "word" => {
            let bytes = markdown_docx(&title, &content)?;
            Ok(binary(
                bytes,
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                &format!("{base}.docx"),
            ))
        }
        "pdf" => Err(XError::not_implemented(
            "PDF 导出（需要内嵌中文字体与排版引擎；请先导出 Word 或 Markdown）",
        )),
        other => Err(XError::bad_request(format!("不支持的导出格式：{other}"))),
    }
}

fn binary(bytes: Vec<u8>, media: &str, filename: &str) -> Response {
    (
        [
            (header::CONTENT_TYPE, media.to_string()),
            (
                header::CONTENT_DISPOSITION,
                tasks::content_disposition(filename),
            ),
        ],
        bytes,
    )
        .into_response()
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r') {
                '_'
            } else {
                c
            }
        })
        .take(60)
        .collect();
    let trimmed = cleaned.trim().to_string();
    if trimmed.is_empty() {
        "export".to_string()
    } else {
        trimmed
    }
}

/// 从 markdown 里取出第一张表（表头 + 数据行）。
fn first_table(markdown: &str) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let mut header: Option<Vec<String>> = None;
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in markdown.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            if header.is_some() && !rows.is_empty() {
                break;
            }
            continue;
        }
        let cells: Vec<String> = line
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect();
        let is_separator = cells
            .iter()
            .all(|cell| !cell.is_empty() && cell.chars().all(|c| matches!(c, '-' | ':' | ' ')));
        if is_separator {
            continue;
        }
        if header.is_none() {
            header = Some(cells);
        } else {
            rows.push(cells);
        }
    }
    header.map(|header| (header, rows))
}

/// markdown → xlsx。
///
/// 有表格就把表格写成一张表（表头 + 数据）；没有表格就把每行当成一列内容 ——
/// **不假装它是表格**，那样导出的 Excel 打开就是一堆错位的格子。
fn markdown_workbook(title: &str, markdown: &str) -> XResult<Vec<u8>> {
    use dbmind_core::CellValue;

    let mut book = Workbook::new(Cursor::new(Vec::new()));
    match first_table(markdown) {
        Some((header, rows)) => {
            let spec = SheetSpec {
                name: title.to_string(),
                headers: header.clone(),
            };
            book.begin_sheet(&spec).map_err(XError::internal)?;
            for row in rows {
                let mut cells: Vec<CellValue> = row
                    .iter()
                    .map(|cell| {
                        // 纯数字写成数字，其余当文本：Excel 里能直接求和
                        match cell.parse::<f64>() {
                            Ok(value) if !cell.is_empty() => CellValue::Real(value),
                            _ => CellValue::Text(cell.clone()),
                        }
                    })
                    .collect();
                while cells.len() < header.len() {
                    cells.push(CellValue::Null);
                }
                book.row(&cells).map_err(XError::internal)?;
            }
            book.end_sheet().map_err(XError::internal)?;
        }
        None => {
            let spec = SheetSpec {
                name: title.to_string(),
                headers: vec!["内容".to_string()],
            };
            book.begin_sheet(&spec).map_err(XError::internal)?;
            for line in markdown.lines() {
                book.row(&[CellValue::Text(line.to_string())])
                    .map_err(XError::internal)?;
            }
            book.end_sheet().map_err(XError::internal)?;
        }
    }
    let cursor = book.finish().map_err(XError::internal)?;
    Ok(cursor.into_inner())
}

/// markdown → docx（最小可打开的 Word 文档）。
fn markdown_docx(title: &str, markdown: &str) -> XResult<Vec<u8>> {
    let mut paragraphs = String::new();
    paragraphs.push_str(&paragraph(title, true));
    for line in markdown.lines() {
        // 只做最轻的 markdown 处理：标题层级与「# * >」这类行首记号去掉，
        // 剩下的交给 Word 的样式 —— 试图在 Word 里还原 markdown 全貌不现实
        let text = line
            .trim_start_matches('#')
            .trim_start_matches('>')
            .trim_start_matches(['*', '-', ' '])
            .trim();
        if text.is_empty() {
            paragraphs.push_str(&paragraph("", false));
            continue;
        }
        paragraphs.push_str(&paragraph(text, false));
    }

    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>{paragraphs}<w:sectPr/></w:body></w:document>"#
    );
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, body) in [
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", rels),
        ("word/document.xml", document.as_str()),
    ] {
        zip.start_file(name, options)
            .map_err(|e| XError::internal(format!("写入 docx 失败：{e}")))?;
        zip.write_all(body.as_bytes())
            .map_err(|e| XError::internal(format!("写入 docx 失败：{e}")))?;
    }
    let cursor = zip
        .finish()
        .map_err(|e| XError::internal(format!("打包 docx 失败：{e}")))?;
    Ok(cursor.into_inner())
}

fn paragraph(text: &str, bold: bool) -> String {
    let run = if bold {
        format!(
            r#"<w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r>"#,
            escape_xml(text)
        )
    } else {
        format!(
            r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#,
            escape_xml(text)
        )
    };
    format!(r#"<w:p>{run}</w:p>"#)
}

fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// 未使用的 JSON 辅助（保留以便将来加 `format: json`）。
#[allow(dead_code)]
fn as_json(value: &Value) -> String {
    json!(value).to_string()
}
