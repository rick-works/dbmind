//! `.xlsx`（Excel 2007+）：**读**（导入）与**写**（导出/文档）都在这里。
//!
//! 两个方向共用一个认知：.xlsx 是个 zip，里面若干 XML 描述工作簿。写的时候按部件直接拼，
//! 读的时候按部件解析 —— 都只需要 OpenXML 的一小片（工作表 + 内联字符串 + 样式里的日期判定）。
//!
//! ## 写（`Workbook`）
//!
//! 流式：`begin_sheet` → 若干 `row` → `end_sheet`，`finish` 时才补上容器部件
//! （`[Content_Types].xml` / `_rels` / `workbook.xml` / `styles.xml`）。这样导出百万行
//! 也不需要把整张表攒在内存里 —— 部件在 zip 里的先后顺序对 Excel 没有意义（按名字找）。
//!
//! 文本一律写 **inlineStr**：省掉共享字符串表，写入端不用维护「字符串 → 下标」的映射，
//! 而且流式导出时本来就没法回头去补那张表。
//!
//! ## 读（`parse`）
//!
//! 几个刻意的取舍：
//!
//! - **不能按出现顺序排单元格**。Excel 会省略空单元格（`<c r="A1">`、`<c r="C1">` 之间没有 B），
//!   按顺序读会把 C 列的值落到 B 列上 —— 静默错位，比报错难查得多。所以一律按 `r` 定位。
//! - **日期要转，但要保守**。Excel 把日期存成序列号（`45291` 其实是 2023-12-31），
//!   不转就是一串没人看得懂的数字；但把普通数字误判成日期更糟，所以只认内置日期格式号
//!   与自定义格式里含 `y`/`d`/`h`/`s` 的那些。
//! - **不读公式**。读的是 Excel 保存时算好的缓存值 `<v>`；公式文本与导入无关。
//! - **错误单元格给 NULL**。`#DIV/0!` 这类没有可用值，塞进数值列只会让整批导入失败。

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, Write};

use chrono::{Duration, NaiveDate};
use dbmind_core::CellValue;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::api::error::{XError, XResult};

const XML_HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

// ================================================================== 写

/// 一张工作表：名字 + 表头。
pub struct SheetSpec {
    pub name: String,
    pub headers: Vec<String>,
}

struct OpenSheet {
    /// 已写入的行数（含表头）；下一条数据行的行号 = 它 + 1
    written: usize,
}

/// 流式 xlsx 写入器。
pub struct Workbook<W: Write + Seek> {
    zip: ZipWriter<W>,
    names: Vec<String>,
    open: Option<OpenSheet>,
}

impl<W: Write + Seek> Workbook<W> {
    pub fn new(inner: W) -> Self {
        Self {
            zip: ZipWriter::new(inner),
            names: Vec::new(),
            open: None,
        }
    }

    /// 开始一张工作表（表头立刻写成第 1 行）。
    pub fn begin_sheet(&mut self, spec: &SheetSpec) -> Result<(), String> {
        // 上一张还开着就替它收尾，而不是报错 —— 见 `finish` 的说明。
        // 多表导出（整库转储）就是「begin_sheet → 若干 row → begin_sheet → … → finish」的写法。
        if self.open.is_some() {
            self.close_open_sheet()?;
        }
        let index = self.names.len() + 1;
        self.names.push(sheet_name(&spec.name, index));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        self.zip
            .start_file(sheet_path(index), options)
            .map_err(io_message)?;
        self.write_raw(XML_HEADER)?;
        self.write_raw(
            "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>",
        )?;
        self.open = Some(OpenSheet { written: 0 });
        let headers: Vec<CellValue> = spec
            .headers
            .iter()
            .map(|name| CellValue::Text(name.clone()))
            .collect();
        self.row(&headers)
    }

    /// 追加一行数据。
    pub fn row(&mut self, cells: &[CellValue]) -> Result<(), String> {
        let number = match self.open.as_mut() {
            Some(sheet) => {
                sheet.written += 1;
                sheet.written
            }
            None => return Err("xlsx：还没有开始工作表".to_string()),
        };
        let mut xml = format!("<row r=\"{number}\">");
        for (index, cell) in cells.iter().enumerate() {
            xml.push_str(&cell_xml(index, number, cell));
        }
        xml.push_str("</row>");
        self.write_raw(&xml)
    }

    /// 结束当前工作表。
    pub fn end_sheet(&mut self) -> Result<(), String> {
        if self.open.is_none() {
            return Err("xlsx：没有正在写的工作表".to_string());
        }
        self.close_open_sheet()
    }

    /// 给当前工作表补上结束标签并标记关闭（显式 `end_sheet` 与自动收尾共用）。
    fn close_open_sheet(&mut self) -> Result<(), String> {
        self.open = None;
        self.write_raw("</sheetData></worksheet>")
    }

    /// 补齐容器部件并收尾，返回底层 writer（文件或内存游标）。
    ///
    /// **没调过 `end_sheet` 也要能正常收尾**：`export.rs` 的写法就是
    /// 「`begin_sheet` → 逐行 `row` → `finish`」，它一直依赖这里替它关掉最后一张表。
    /// 我重建这个写入器时在此处直接报错（「还有工作表没结束」），结果导出跑到二十万行、
    /// 数据全部写完，却在最后一步整单失败 —— 把「内部可以自动收尾」误当成了
    /// 「调用方必须先调 end_sheet」。多表场景同理（见 `begin_sheet`）。
    pub fn finish(mut self) -> Result<W, String> {
        if self.open.is_some() {
            self.close_open_sheet()?;
        }
        for (name, body) in self.parts() {
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            self.zip.start_file(name, options).map_err(io_message)?;
            self.write_raw(&body)?;
        }
        self.zip.finish().map_err(io_message)
    }

    fn write_raw(&mut self, text: &str) -> Result<(), String> {
        self.zip.write_all(text.as_bytes()).map_err(io_message)
    }

    /// 收尾时才写的那些部件（内容类型、关系、工作簿清单、样式）。
    fn parts(&self) -> Vec<(String, String)> {
        let count = self.names.len();
        let mut content_types = String::from(XML_HEADER);
        content_types.push_str(
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        );
        content_types.push_str(
            "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
        );
        content_types.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");
        content_types.push_str(
            "<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>",
        );
        content_types.push_str(
            "<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>",
        );
        for index in 1..=count {
            content_types.push_str(&format!(
                "<Override PartName=\"/{0}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>",
                sheet_path(index)
            ));
        }
        content_types.push_str("</Types>");

        let root_rels = format!(
            "{XML_HEADER}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
             </Relationships>"
        );

        let mut sheets = String::new();
        let mut workbook_rels = String::new();
        for (index, name) in self.names.iter().enumerate() {
            let id = index + 1;
            sheets.push_str(&format!(
                "<sheet name=\"{}\" sheetId=\"{id}\" r:id=\"rId{id}\"/>",
                escape_xml(name)
            ));
            workbook_rels.push_str(&format!(
                "<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"{}\"/>",
                sheet_rel_target(id)
            ));
        }
        let workbook = format!(
            "{XML_HEADER}<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
             xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
             <sheets>{sheets}</sheets></workbook>"
        );
        let rels = format!(
            "{XML_HEADER}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             {workbook_rels}</Relationships>"
        );
        // 最小样式表：没有它 Excel 也能打开，但带上默认字体更规矩（字体缺失时它自己会挑一个难看的）
        let styles = format!(
            "{XML_HEADER}<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
             <fills count=\"1\"><fill><patternFill patternType=\"none\"/></fill></fills>\
             <borders count=\"1\"><border/></borders>\
             <cellStyleXfs count=\"1\"><xf/></cellStyleXfs>\
             <cellXfs count=\"1\"><xf/></cellXfs>\
             </styleSheet>"
        );

        vec![
            ("[Content_Types].xml".to_string(), content_types),
            ("_rels/.rels".to_string(), root_rels),
            ("xl/workbook.xml".to_string(), workbook),
            ("xl/_rels/workbook.xml.rels".to_string(), rels),
            ("xl/styles.xml".to_string(), styles),
        ]
    }
}

/// 写入失败的统一文案。泛型是因为 zip 落盘报 `ZipError`、写字节报 `io::Error`，
/// 两者都只需要「一句话」。写成具体类型会逼出两套几乎相同的闭包。
fn io_message<E: std::fmt::Display>(error: E) -> String {
    format!("写入 xlsx 失败：{error}")
}

fn sheet_path(index: usize) -> String {
    format!("xl/worksheets/sheet{index}.xml")
}

/// 工作表在 xl/_rels/workbook.xml.rels 里的 Target。
///
/// 必须是【相对 workbook.xml 所在目录（xl/）】的路径，不能带 xl/ 前缀：
/// 写成 "xl/worksheets/sheet1.xml" 会被解析到 "xl/xl/worksheets/sheet1.xml" ——
/// 那个部件不存在，Excel 于是报「发现…中的部分内容有问题，是否恢复」。
/// 按部件名直接读文件的阅读器（PowerShell、多数解包工具）不理会关系，所以一直没暴露。
fn sheet_rel_target(index: usize) -> String {
    format!("worksheets/sheet{index}.xml")
}

/// 单元格 XML。
fn cell_xml(column: usize, row: usize, cell: &CellValue) -> String {
    let reference = format!("{}{row}", column_name(column));
    match cell {
        CellValue::Null => format!("<c r=\"{reference}\"/>"),
        CellValue::Integer(value) => format!("<c r=\"{reference}\"><v>{value}</v></c>"),
        CellValue::Real(value) => {
            // NaN / Inf 不是合法的数字单元格（Excel 会判定文件损坏），降级成文本
            if value.is_finite() {
                format!("<c r=\"{reference}\"><v>{value}</v></c>")
            } else {
                text_cell(&reference, &value.to_string())
            }
        }
        CellValue::Text(text) => text_cell(&reference, text),
        // 二进制不塞进单元格（Excel 存不了任意字节）：给一个可读占位，
        // 而不是让整张表导不出来
        CellValue::Blob { len } => text_cell(&reference, &format!("<blob {len}B>")),
    }
}

fn text_cell(reference: &str, text: &str) -> String {
    format!(
        "<c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
        escape_xml(text)
    )
}

/// 0 → `A`、25 → `Z`、26 → `AA`。
fn column_name(index: usize) -> String {
    let mut value = index + 1;
    let mut out = String::new();
    while value > 0 {
        let remainder = (value - 1) % 26;
        out.insert(0, (b'A' + remainder as u8) as char);
        value = (value - 1) / 26;
    }
    out
}

/// 工作表名合不合法由一个硬约束决定：Excel 只接受 **31 字符以内**、且不含 `[ ] : * ? / \` 的名字。
/// 不处理的话文件照样生成，但 Excel 打开时弹「已修复」—— 而真实库里长表名很常见。
fn sheet_name(raw: &str, index: usize) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return format!("Sheet{index}");
    }
    trimmed.chars().take(31).collect()
}

/// XML 文本转义。顺带丢掉 XML 1.0 不允许的控制字符（`\x00`-`\x08` 等）——
/// 数据库里的文本确实可能带上它们，留在文件里会让 Excel 判定损坏。
fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

// ================================================================== 读

/// 解析第一个工作表：返回「表头 + 数据行」。
///
/// 这里只负责**取出来**，表头命名（`column_1` 兜底）、行对齐、跳过空行等口径
/// 由 `import.rs` 统一处理 —— 与 CSV / JSON 走同一套收尾，避免两条路径长出两种行为。
pub fn parse(bytes: &[u8]) -> XResult<Vec<Vec<Option<String>>>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| XError::bad_request(format!("这不是有效的 .xlsx（zip 打不开）：{e}")))?;

    let shared = entry_text(&mut zip, "xl/sharedStrings.xml")
        .map(|xml| shared_strings(&xml))
        .unwrap_or_default();

    let workbook = entry_text(&mut zip, "xl/workbook.xml").unwrap_or_default();
    let date1904 = workbook.contains("date1904=\"1\"") || workbook.contains("date1904=\"true\"");
    let date_xf = entry_text(&mut zip, "xl/styles.xml")
        .map(|xml| date_styles(&xml))
        .unwrap_or_default();

    let sheet_path = sheet_path_in(&mut zip, &workbook);
    let sheet = entry_text(&mut zip, &sheet_path).ok_or_else(|| {
        XError::bad_request(format!(
            "这个 .xlsx 里找不到工作表（期望 {sheet_path}）—— 文件可能损坏，或只存了宏"
        ))
    })?;

    let rows = sheet_rows(&sheet, &shared, &date_xf, date1904);
    if rows.is_empty() {
        return Err(XError::bad_request("这个 .xlsx 的第一个工作表是空的"));
    }
    Ok(rows)
}

/// 读 zip 里的一个文本条目（不存在 / 解码失败都当「没有」，调用方各自兜底）。
fn entry_text<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut file = zip.by_name(name).ok()?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).ok()?;
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// 第一个工作表在 zip 里的路径。
///
/// **不能假设它叫 `sheet1.xml`**：用户在 Excel 里调整过工作表顺序、或删过又建过之后，
/// 第一个工作表完全可能是 `sheet2.xml`。所以先按 `workbook.xml` 的声明 + 关系文件解析，
/// 解析不出来再退回「编号最小的那个」。
fn sheet_path_in<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, workbook: &str) -> String {
    // 先把兜底路径算出来：`file_names()` 是对 zip 的**只读**借用，
    // 与后面读文件条目的可变借用不能共存（借用检查器会直接拦下）
    let fallback = {
        let mut candidates: Vec<String> = zip
            .file_names()
            .filter(|name| name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml"))
            .map(str::to_string)
            .collect();
        candidates.sort();
        candidates
            .first()
            .cloned()
            .unwrap_or_else(|| "xl/worksheets/sheet1.xml".to_string())
    };

    // workbook.xml 里第一个 <sheet ... r:id="rIdN"/> 就是「第一个工作表」
    let Some(rid) = first_sheet_rel(workbook) else {
        return fallback;
    };
    let Some(rels) = entry_text(zip, "xl/_rels/workbook.xml.rels") else {
        return fallback;
    };
    let Some(target) = relationship_target(&rels, &rid) else {
        return fallback;
    };
    // 关系里的 Target 可能是相对路径（`worksheets/sheet1.xml`）或绝对路径（`/xl/worksheets/...`）
    let trimmed = target.trim_start_matches('/');
    if trimmed.starts_with("xl/") {
        trimmed.to_string()
    } else {
        format!("xl/{trimmed}")
    }
}

/// `workbook.xml` 里第一个工作表的 `r:id`。
fn first_sheet_rel(workbook: &str) -> Option<String> {
    let tag = find_tag(workbook, "sheet")?;
    attr(&tag, "r:id").or_else(|| attr(&tag, "id"))
}

/// 关系文件里某个 Id 的 Target。
fn relationship_target(rels: &str, id: &str) -> Option<String> {
    let mut search = 0;
    while let Some(tag) = find_tag_from(rels, "Relationship", search) {
        if attr(&tag, "Id").as_deref() == Some(id) {
            return attr(&tag, "Target");
        }
        let at = rels[search..].find(&tag)? + search;
        search = at + tag.len();
    }
    None
}

/// 一个工作表的全部行（外层按行、内层按列，空单元格是 `None`）。
fn sheet_rows(
    xml: &str,
    shared: &[String],
    date_xf: &[bool],
    date1904: bool,
) -> Vec<Vec<Option<String>>> {
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut row: Vec<Option<String>> = Vec::new();
    let mut next_index = 0usize;

    // 当前单元格的状态
    let mut cell_ref: Option<String> = None;
    let mut cell_type: Option<String> = None;
    let mut cell_style: Option<usize> = None;
    let mut cell_value = String::new();
    let mut cell_inline = String::new();
    let mut in_value = false;
    let mut in_text = false;

    let mut pos = 0usize;
    while let Some(lt) = xml[pos..].find('<') {
        let lt = pos + lt;
        let text = &xml[pos..lt];
        if in_value {
            cell_value.push_str(text);
        } else if in_text {
            cell_inline.push_str(text);
        }
        let Some(gt) = xml[lt..].find('>') else { break };
        let gt = lt + gt + 1;
        let tag = &xml[lt + 1..gt - 1];
        pos = gt;

        if let Some(name) = tag.strip_prefix('/') {
            match name {
                "v" => in_value = false,
                "t" => in_text = false,
                "c" => {
                    let index = cell_ref
                        .as_deref()
                        .and_then(column_index)
                        .unwrap_or(next_index);
                    let value = cell_text(
                        cell_type.as_deref(),
                        &cell_value,
                        &cell_inline,
                        cell_style,
                        shared,
                        date_xf,
                        date1904,
                    );
                    put(&mut row, index, value);
                    next_index = index + 1;
                    cell_ref = None;
                    cell_type = None;
                    cell_style = None;
                    cell_value.clear();
                    cell_inline.clear();
                }
                "row" => {
                    if row.iter().any(Option::is_some) {
                        rows.push(std::mem::take(&mut row));
                    } else {
                        row.clear();
                    }
                    next_index = 0;
                }
                _ => {}
            }
            continue;
        }
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let self_closing = tag.ends_with('/');
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        match name {
            "c" => {
                cell_ref = attr(tag, "r");
                cell_type = attr(tag, "t");
                cell_style = attr(tag, "s").and_then(|s| s.parse::<usize>().ok());
                cell_value.clear();
                cell_inline.clear();
                if self_closing {
                    // 自闭合的 `<c/>` 也是空单元格：**要占住列位置**，否则后面所有列都会左移
                    let index = cell_ref
                        .as_deref()
                        .and_then(column_index)
                        .unwrap_or(next_index);
                    put(&mut row, index, None);
                    next_index = index + 1;
                }
            }
            "v" => {
                if !self_closing {
                    in_value = true;
                }
            }
            "t" => {
                if !self_closing {
                    in_text = true;
                }
            }
            _ => {}
        }
    }
    if row.iter().any(Option::is_some) {
        rows.push(row);
    }
    rows
}

/// 单个单元格 → 字符串值。
fn cell_text(
    kind: Option<&str>,
    value: &str,
    inline: &str,
    style: Option<usize>,
    shared: &[String],
    date_xf: &[bool],
    date1904: bool,
) -> Option<String> {
    match kind {
        // 共享字符串：`<v>` 里是下标
        Some("s") => value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|index| shared.get(index).cloned()),
        Some("inlineStr") => non_empty(unescape(inline)),
        Some("str") => non_empty(unescape(value)),
        Some("b") => Some(if value.trim() == "1" { "TRUE" } else { "FALSE" }.to_string()),
        // 错误单元格（#DIV/0! / #N/A …）没有可用值，给 NULL 比塞进去让整批导入失败好
        Some("e") => None,
        // ISO 8601 日期字符串（少见：Excel 只在少数场景写它）
        Some("d") => non_empty(value.trim().to_string()),
        // 默认就是数字（也可能是一张伪装成数字的日期）
        _ => {
            let raw = value.trim();
            if raw.is_empty() {
                return None;
            }
            let is_date = style
                .map(|index| date_xf.get(index).copied().unwrap_or(false))
                .unwrap_or(false);
            if is_date {
                if let Ok(serial) = raw.parse::<f64>() {
                    if let Some(text) = serial_to_datetime(serial, date1904) {
                        return Some(text);
                    }
                }
            }
            Some(raw.to_string())
        }
    }
}

/// `"AB12"` → 27（0 基的列下标）。
fn column_index(reference: &str) -> Option<usize> {
    let mut value = 0usize;
    let mut found = false;
    for c in reference.chars() {
        if c.is_ascii_alphabetic() {
            value = value * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1);
            found = true;
        } else {
            break;
        }
    }
    found.then(|| value - 1)
}

fn put(row: &mut Vec<Option<String>>, index: usize, value: Option<String>) {
    if index >= row.len() {
        row.resize(index + 1, None);
    }
    row[index] = value;
}

fn non_empty(text: String) -> Option<String> {
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Excel 序列号 → 日期字符串。
///
/// 1900 历法有个著名的 bug：Excel 认为 1900 年是闰年（多出一个并不存在的 2 月 29 日），
/// 所以序列号 60 之后要按「1899-12-30 起算」把那天补回来。这不是学究气 ——
/// 基准取错会让**每一个**日期都偏一天，而且看起来完全正常。
fn serial_to_datetime(serial: f64, date1904: bool) -> Option<String> {
    if !serial.is_finite() || serial < 0.0 {
        return None;
    }
    let days = serial.floor() as i64;
    let fraction = serial - serial.floor();
    let base = if date1904 {
        NaiveDate::from_ymd_opt(1904, 1, 1)?
    } else if days < 60 {
        NaiveDate::from_ymd_opt(1899, 12, 31)?
    } else {
        NaiveDate::from_ymd_opt(1899, 12, 30)?
    };
    let date = base.checked_add_signed(Duration::days(days))?;
    let seconds = (fraction * 86_400.0).round() as i64;
    if seconds <= 0 {
        return Some(date.format("%Y-%m-%d").to_string());
    }
    let datetime = date.and_hms_opt(0, 0, 0)? + Duration::seconds(seconds);
    Some(datetime.format("%Y-%m-%d %H:%M:%S").to_string())
}

// ------------------------------------------------------------------ 共享字符串 / 样式

/// `xl/sharedStrings.xml` → 字符串数组（下标即索引）。
fn shared_strings(xml: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_si = false;
    let mut in_t = false;
    // `<rPh>` 是日文注音，不属于单元格的值
    let mut in_phonetic = false;

    let mut pos = 0usize;
    while let Some(lt) = xml[pos..].find('<') {
        let lt = pos + lt;
        let text = &xml[pos..lt];
        if in_t && !in_phonetic {
            current.push_str(text);
        }
        let Some(gt) = xml[lt..].find('>') else { break };
        let gt = lt + gt + 1;
        let tag = &xml[lt + 1..gt - 1];
        pos = gt;

        if let Some(name) = tag.strip_prefix('/') {
            match name {
                "t" => in_t = false,
                "rPh" => in_phonetic = false,
                "si" => {
                    out.push(unescape(&current));
                    current.clear();
                    in_si = false;
                }
                _ => {}
            }
            continue;
        }
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let self_closing = tag.ends_with('/');
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        match name {
            "si" => in_si = true,
            "t" if in_si && !self_closing => in_t = true,
            "rPh" => in_phonetic = true,
            _ => {}
        }
    }
    out
}

/// 每个单元格样式（`cellXfs` 的下标，即单元格的 `s` 属性）是否是日期格式。
fn date_styles(xml: &str) -> Vec<bool> {
    // 自定义格式号（>= 164）的判定结果
    let mut custom: HashMap<u32, bool> = HashMap::new();
    let mut search = 0;
    while let Some(tag) = find_tag_from(xml, "numFmt", search) {
        if let (Some(id), Some(code)) = (attr(&tag, "numFmtId"), attr(&tag, "formatCode")) {
            if let Ok(id) = id.parse::<u32>() {
                custom.insert(id, looks_like_date(&unescape(&code)));
            }
        }
        let Some(at) = xml[search..].find(&tag) else { break };
        search += at + tag.len();
    }

    let mut out: Vec<bool> = Vec::new();
    let Some(start) = xml.find("<cellXfs") else {
        return out;
    };
    let rest = &xml[start..];
    let end = rest.find("</cellXfs>").unwrap_or(rest.len());
    let block = &rest[..end];
    let mut search = 0;
    while let Some(tag) = find_tag_from(block, "xf", search) {
        let id = attr(&tag, "numFmtId")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        out.push(builtin_date_format(id) || custom.get(&id).copied().unwrap_or(false));
        let Some(at) = block[search..].find(&tag) else { break };
        search += at + tag.len();
    }
    out
}

/// Excel 内置的日期/时间格式号（ECMA-376 里定死的那些）。
fn builtin_date_format(id: u32) -> bool {
    matches!(id, 14..=22 | 27..=36 | 45..=47 | 50..=58)
}

/// 自定义格式串是否像日期/时间。
///
/// 先把引号/方括号里的字面量（`0.00"天"`、`[$-409]`）去掉，再看有没有 `y/d/h/s`；
/// **不**单独看 `m` —— 它既表示月也表示分，还出现在很多非日期格式里，太容易误判。
fn looks_like_date(code: &str) -> bool {
    let mut stripped = String::new();
    let mut skip = false;
    for c in code.chars() {
        match c {
            '"' => skip = !skip,
            '[' => skip = true,
            ']' => skip = false,
            other if !skip => stripped.push(other),
            _ => {}
        }
    }
    let lower = stripped.to_ascii_lowercase();
    lower.contains('y') || lower.contains('d') || lower.contains('h') || lower.contains('s')
}

// ------------------------------------------------------------------ 小工具

/// 找到名为 `name` 的第一个开始标签的**标签体**（不含 `<` `>`）。
fn find_tag(xml: &str, name: &str) -> Option<String> {
    find_tag_from(xml, name, 0)
}

fn find_tag_from(xml: &str, name: &str, from: usize) -> Option<String> {
    let mut search = from;
    while let Some(lt) = xml[search..].find('<') {
        let lt = search + lt;
        let gt = lt + xml[lt..].find('>')? + 1;
        let tag = &xml[lt + 1..gt - 1];
        let tag_name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        if tag_name == name {
            return Some(tag.to_string());
        }
        search = gt;
    }
    None
}

/// 取属性值。名字前必须是空白，避免 `s="` 命中 `numFmtId="…` 之类子串。
fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let mut search = 0;
    while let Some(rel) = tag[search..].find(&needle) {
        let at = search + rel;
        let boundary = at == 0 || tag.as_bytes()[at - 1].is_ascii_whitespace();
        if boundary {
            let start = at + needle.len();
            let end = start + tag[start..].find('"')?;
            return Some(tag[start..end].to_string());
        }
        search = at + needle.len();
    }
    None
}

/// XML 实体还原。Excel 会把 `&`、`<` 和中文标点转义，不还原就会把 `&amp;` 原样写进数据库。
fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(semi) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..semi];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix('#')
                .and_then(|code| {
                    if let Some(hex) = code.strip_prefix(['x', 'X']) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        code.parse::<u32>().ok()
                    }
                })
                .and_then(char::from_u32),
        };
        match replacement {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            // 不认识的实体原样保留（宁可留着，也不要悄悄吃掉）
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn sheet(name: &str, headers: &[&str]) -> SheetSpec {
        SheetSpec {
            name: name.to_string(),
            headers: headers.iter().map(|h| h.to_string()).collect(),
        }
    }

    /// **不调 `end_sheet` 也要能收尾**（这条是被踩过一次的回归，必须有测试守住）。
    ///
    /// `export.rs` 的写法就是「`begin_sheet` → 逐行 `row` → `finish`」。我重建写入器时
    /// 让 `finish` 在这种情况下报错「还有工作表没结束」，结果导出跑到二十万行、数据全部写完，
    /// 却在最后一步整单失败 —— 把「内部可以自动收尾」误当成了「调用方必须先调 end_sheet」。
    #[test]
    fn 不调end_sheet也能收尾() {
        let mut book = Workbook::new(Cursor::new(Vec::new()));
        book.begin_sheet(&sheet("t", &["a", "b"])).unwrap();
        book.row(&[CellValue::Text("甲".to_string()), CellValue::Integer(7)])
            .unwrap();
        let bytes = book
            .finish()
            .expect("finish 不该因为调用方没调 end_sheet 而失败")
            .into_inner();

        let records = parse(&bytes).expect("应能读回自己写的文件");
        assert_eq!(records.len(), 2, "1 行表头 + 1 行数据");
        assert_eq!(records[0], vec![Some("a".to_string()), Some("b".to_string())]);
        assert_eq!(records[1], vec![Some("甲".to_string()), Some("7".to_string())]);
    }

    /// 连续 `begin_sheet` 也要自动关上一张（整库转储的场景：每张表一个新工作表）。
    #[test]
    fn 连续两张表都不调end_sheet() {
        let mut book = Workbook::new(Cursor::new(Vec::new()));
        for name in ["一", "二"] {
            book.begin_sheet(&sheet(name, &["c"])).unwrap();
            book.row(&[CellValue::Text("x".to_string())]).unwrap();
        }
        let bytes = book.finish().unwrap().into_inner();

        let archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        for expected in [
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "xl/workbook.xml",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "缺少部件 {expected}（现有：{names:?}）"
            );
        }
        // 读回时取第一张表
        let records = parse(&bytes).unwrap();
        assert_eq!(records[0], vec![Some("c".to_string())]);
    }

    /// 显式 `end_sheet` 照旧可用（`ai/doc.rs` 就是显式调的），且重复调用要报错。
    #[test]
    fn 显式end_sheet仍然可用() {
        let mut book = Workbook::new(Cursor::new(Vec::new()));
        book.begin_sheet(&sheet("s", &["h"])).unwrap();
        book.row(&[CellValue::Real(1.5)]).unwrap();
        book.end_sheet().unwrap();
        assert!(book.end_sheet().is_err(), "没有开着的工作表时应报错");
        let bytes = book.finish().unwrap().into_inner();
        let records = parse(&bytes).unwrap();
        assert_eq!(records[1], vec![Some("1.5".to_string())]);
    }

    /// 表名要净化：Excel 只接受 ≤31 字符、且不含 `[ ] : * ? / \`，
    /// 否则文件能生成、Excel 打开却提示「已修复」（真实库里长表名很常见）。
    #[test]
    fn 表名净化() {
        let mut book = Workbook::new(Cursor::new(Vec::new()));
        let tricky = format!("x/y:z*w?[q]\\{}", "a".repeat(40));
        book.begin_sheet(&sheet(&tricky, &["h"])).unwrap();
        let bytes = book.finish().unwrap().into_inner();

        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let workbook = {
            let mut entry = archive.by_name("xl/workbook.xml").unwrap();
            let mut text = String::new();
            std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
            text
        };
        let name = workbook
            .split("name=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("workbook.xml 里应有工作表名");
        assert!(name.chars().count() <= 31, "表名超长：{name}");
        for bad in ['[', ']', ':', '*', '?', '/', '\\'] {
            assert!(!name.contains(bad), "表名含 Excel 非法字符 {bad}：{name}");
        }
    }
}
