//! 附件文本抽取（upgrade-chat-ai-context-coverage E4）。
//!
//! 支持族：纯文本类（txt/md/csv/json/…）、`pdf`、Office（`docx` / `xlsx` / `xls` / `ods`）。
//!
//! 上限（E4-c，对齐既有图片 4MiB 与出向 4000 字符截断口径）：
//! - 字节：`ATTACHMENT_MAX_BYTES`，超出 → `TooLarge`（不再解析）
//! - 字符：`ATTACHMENT_MAX_CHARS`，超出截断并追加标记
//!
//! 失败一律返回 `Err(reason)`——调用方标注 `failed` 并在 prompt 显式提示
//! 「附件未解析」，MUST NOT 静默丢弃或阻断该轮对话。
//!
//! 通道（E4-b）：文档字节来自文件存储（`api/files`）的引用解析结果，
//! MUST NOT 走 base64 内联通道（图片维持 `data_base64` 唯一通道）。

/// 单附件字节上限（4 MiB，与图片口径一致）。
pub const ATTACHMENT_MAX_BYTES: usize = 4 * 1024 * 1024;

/// 解压/解码后字节上限（zip 家族防御：DEFLATE 可 1000:1 放大，只约束压缩态
/// 输入等于放任 zip 炸弹）。docx/xlsx 在解压前先按条目声明总长预检，docx 正文
/// 另做有界读取。
pub const ATTACHMENT_MAX_DECODED_BYTES: usize = 4 * 1024 * 1024;

/// 解析用时上限：超时 → `Failed`，避免病态输入长期占用解析线程。
pub const ATTACHMENT_PARSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// PDF 页数上限：超限即在抽取前拒绝——`spawn_blocking` 任务超时后不可取消，
/// 页数预检把「不可取消窗口」压到与输入字节上限相称的量级。
pub const ATTACHMENT_MAX_PDF_PAGES: usize = 50;

/// 单附件抽取字符上限（与工具出向 4000 字符截断口径一致）。
pub const ATTACHMENT_MAX_CHARS: usize = 4000;

/// 抽取失败原因（`Display` 直接落附件记录的 `reason`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// 超出字节上限（未解析）。
    TooLarge,
    /// 扩展名不受支持。
    Unsupported(String),
    /// 解析失败（含损坏文件、缺内部条目）。
    Failed(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "超出 {} 字节上限", ATTACHMENT_MAX_BYTES),
            Self::Unsupported(ext) => write!(f, "不支持的文件类型: .{ext}"),
            Self::Failed(msg) => write!(f, "解析失败: {msg}"),
        }
    }
}

/// 异步解析入口：CPU / 解压密集工作移入 blocking 线程池，并施加用时上限。
/// 超时 → `Failed("解析超时")`（不阻断该轮，调用方标「附件未解析」）。
pub async fn extract_text_async(filename: String, bytes: Vec<u8>) -> Result<String, ParseError> {
    let task = tokio::task::spawn_blocking(move || extract_text(&filename, &bytes));
    match tokio::time::timeout(ATTACHMENT_PARSE_TIMEOUT, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(join_err)) => Err(ParseError::Failed(format!("解析任务失败: {join_err}"))),
        Err(_) => Err(ParseError::Failed("解析超时".to_string())),
    }
}

/// PDF 页数预检（lopdf 读页树）：超 `ATTACHMENT_MAX_PDF_PAGES` → 拒绝（抽取前）。
/// 文档不可解析时交由下游 `pdf-extract` 报「解析失败」（此处不抢先判死）。
fn guard_pdf_page_count(bytes: &[u8]) -> Result<(), ParseError> {
    let Ok(document) = lopdf::Document::load_mem(bytes) else {
        return Ok(());
    };
    let pages = document.get_pages().len();
    if pages > ATTACHMENT_MAX_PDF_PAGES {
        return Err(ParseError::Failed(format!(
            "PDF 页数 {pages} 超过上限 {ATTACHMENT_MAX_PDF_PAGES}"
        )));
    }
    Ok(())
}

/// zip 家族解压预算校验：**流式解压**并累计实际字节，超 `ATTACHMENT_MAX_DECODED_BYTES`
/// 即拒（不信任中央目录声明值——手写「少报 size」的容器无法绕过）。docx / xlsx /
/// ods 共用；顺带在 calamine 物化前把真实解压量压到上限内。
fn guard_zip_expansion(bytes: &[u8], format: &str) -> Result<(), ParseError> {
    use std::io::Read;

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| ParseError::Failed(format!("{format} 容器不可读: {e}")))?;
    let mut budget = ATTACHMENT_MAX_DECODED_BYTES as u64;
    let mut buf = vec![0u8; 64 * 1024];
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| ParseError::Failed(format!("{format} 条目不可读: {e}")))?;
        loop {
            let read = entry
                .read(&mut buf)
                .map_err(|e| ParseError::Failed(format!("{format} 解压失败: {e}")))?;
            if read == 0 {
                break;
            }
            if (read as u64) > budget {
                return Err(ParseError::TooLarge);
            }
            budget -= read as u64;
        }
    }
    Ok(())
}

/// 按扩展名分派抽取文本。`filename` 仅取扩展名判定（内容嗅探不做——
/// 前端上传前已按扩展名/MIME 约束）。
pub fn extract_text(filename: &str, bytes: &[u8]) -> Result<String, ParseError> {
    if bytes.len() > ATTACHMENT_MAX_BYTES {
        return Err(ParseError::TooLarge);
    }
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let text = match ext.as_str() {
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "log" | "yaml" | "yml" | "xml" => {
            String::from_utf8_lossy(bytes).into_owned()
        }
        "pdf" => {
            guard_pdf_page_count(bytes)?;
            pdf_extract::extract_text_from_mem(bytes)
                .map_err(|e| ParseError::Failed(e.to_string()))?
        }
        "docx" => {
            guard_zip_expansion(bytes, "docx")?;
            extract_docx(bytes)?
        }
        "xlsx" | "xls" | "xlsm" | "ods" => {
            guard_zip_expansion(bytes, &ext)?;
            extract_spreadsheet(bytes)?
        }
        other => return Err(ParseError::Unsupported(other.to_string())),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Failed("未抽取到文本内容".to_string()));
    }
    Ok(truncate_chars(trimmed, ATTACHMENT_MAX_CHARS))
}

/// 字符级截断（按 char 边界，避免切裂多字节字符）。
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("…[truncated]");
    out
}

/// 电子表格抽取（calamine 统一处理 xlsx/xls/ods：共享字符串、单元格类型、日期）。
fn extract_spreadsheet(bytes: &[u8]) -> Result<String, ParseError> {
    use calamine::{open_workbook_auto_from_rs, Reader};

    let cursor = std::io::Cursor::new(bytes.to_vec());
    let mut workbook =
        open_workbook_auto_from_rs(cursor).map_err(|e| ParseError::Failed(e.to_string()))?;
    let mut out = String::new();
    for sheet in workbook.sheet_names().to_vec() {
        let Ok(range) = workbook.worksheet_range(&sheet) else {
            continue;
        };
        out.push_str(&format!("[工作表 {sheet}]\n"));
        for row in range.rows() {
            let cells: Vec<String> = row.iter().map(|c| c.to_string()).collect();
            if cells.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            out.push_str(&cells.join("\t"));
            out.push('\n');
        }
    }
    Ok(out)
}

/// docx 抽取：`word/document.xml` 内 `<w:t>` 文本节点按 `<w:p>` 段落分行。
/// 只读该条目（正文），不解析样式/批注——附件上下文只需要正文。
fn extract_docx(bytes: &[u8]) -> Result<String, ParseError> {
    use quick_xml::events::Event;
    use std::io::{Cursor, Read};

    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| ParseError::Failed(e.to_string()))?;
    let mut entry = archive
        .by_name("word/document.xml")
        .map_err(|_| ParseError::Failed("docx 缺少 word/document.xml".to_string()))?;
    let mut xml = String::new();
    // 有界读取（预检之外的第二道闸：读取量硬上限 + 1 字节用于判定越界）
    entry
        .by_ref()
        .take(ATTACHMENT_MAX_DECODED_BYTES as u64 + 1)
        .read_to_string(&mut xml)
        .map_err(|e| ParseError::Failed(e.to_string()))?;
    if xml.len() > ATTACHMENT_MAX_DECODED_BYTES {
        return Err(ParseError::TooLarge);
    }

    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == "t" {
                    in_text = true;
                }
            }
            Ok(Event::Text(t)) => {
                if in_text {
                    out.push_str(&quick_xml::escape::unescape(&t).unwrap_or_default());
                }
            }
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                "t" => in_text = false,
                "p" => out.push('\n'),
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(ParseError::Failed(e.to_string())),
            _ => {}
        }
    }
    Ok(out)
}

/// XML 限定名去掉前缀后的本地名（`w:t` → `t`）。
fn local_name(name: &str) -> &str {
    match name.find(':') {
        Some(idx) => &name[idx + 1..],
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn text_families_extracted_verbatim() {
        assert_eq!(
            extract_text("a.md", "# 标题\n正文".as_bytes()).unwrap(),
            "# 标题\n正文"
        );
        assert_eq!(extract_text("a.CSV", b"a,b\n1,2").unwrap(), "a,b\n1,2");
        assert_eq!(extract_text("a.json", br#"{"k":1}"#).unwrap(), r#"{"k":1}"#);
    }

    #[test]
    fn unsupported_extension_reports_type() {
        assert_eq!(
            extract_text("a.bin", b"xx"),
            Err(ParseError::Unsupported("bin".to_string()))
        );
        assert_eq!(
            extract_text("noext", b"xx"),
            Err(ParseError::Unsupported(String::new()))
        );
    }

    #[test]
    fn oversized_bytes_rejected_before_parse() {
        let big = vec![b'a'; ATTACHMENT_MAX_BYTES + 1];
        assert_eq!(extract_text("a.txt", &big), Err(ParseError::TooLarge));
    }

    #[test]
    fn empty_text_content_is_failure_not_empty_string() {
        assert!(matches!(
            extract_text("a.txt", b"   \n  "),
            Err(ParseError::Failed(_))
        ));
    }

    #[test]
    fn long_text_truncated_at_char_limit_with_marker() {
        let long = "汉".repeat(ATTACHMENT_MAX_CHARS + 50);
        let out = extract_text("a.txt", long.as_bytes()).unwrap();
        assert!(out.ends_with("…[truncated]"));
        assert_eq!(
            out.chars().count(),
            ATTACHMENT_MAX_CHARS + "…[truncated]".chars().count()
        );
    }

    #[test]
    fn docx_body_text_extracted_with_paragraph_breaks() {
        let bytes = docx_fixture(&["第一段", "第二段"]);
        let out = extract_text("契约.docx", &bytes).unwrap();
        assert_eq!(out, "第一段\n第二段");
    }

    #[test]
    fn xlsx_cells_extracted_per_sheet() {
        let bytes = xlsx_fixture(&[("Sheet1", &[&["科目", "金额"], &["现金", "100"]])]);
        let out = extract_text("账簿.xlsx", &bytes).unwrap();
        assert!(out.contains("[工作表 Sheet1]"), "got: {out}");
        assert!(out.contains("科目\t金额"), "got: {out}");
        assert!(out.contains("现金\t100"), "got: {out}");
    }

    #[test]
    fn zip_expansion_rejected_before_decompression() {
        // 5 MiB 正文、压缩后极小：预检 MUST 在解压前拒绝（否则 DEFLATE 放大打爆内存）
        let huge = "a".repeat(5 * 1024 * 1024);
        let bytes = docx_fixture(&[huge.as_str()]);
        assert!(
            bytes.len() < ATTACHMENT_MAX_BYTES,
            "测试前提：压缩态输入本身在字节上限内（{})",
            bytes.len()
        );
        assert_eq!(extract_text("炸弹.docx", &bytes), Err(ParseError::TooLarge));
    }

    #[test]
    fn zip_budget_is_streamed_not_declared() {
        // xlsx 最终由 calamine 物化，中央目录声明值不可信：少报 size 的容器
        // MUST 仍被**流式**解压预算拒绝（否则等价于放开 zip 炸弹）。
        let huge = "a".repeat(5 * 1024 * 1024);
        let mut bytes = xlsx_fixture(&[("S", &[&[huge.as_str()]])]);
        under_report_declared_size(&mut bytes, "xl/worksheets/sheet1.xml");
        assert_eq!(extract_text("炸弹.xlsx", &bytes), Err(ParseError::TooLarge));
    }

    #[test]
    fn corrupt_pdf_reports_failure_without_panic() {
        assert!(matches!(
            extract_text("broken.pdf", b"%PDF-1.4 garbage not a pdf"),
            Err(ParseError::Failed(_))
        ));
    }

    #[test]
    fn pdf_page_cap_rejects_over_limit_before_extraction() {
        // 上限内放行（页数预检独立于文本抽取质量）
        assert!(guard_pdf_page_count(&pdf_fixture(ATTACHMENT_MAX_PDF_PAGES)).is_ok());
        let oversized = pdf_fixture(ATTACHMENT_MAX_PDF_PAGES + 1);
        let error = guard_pdf_page_count(&oversized).expect_err("超页数上限 MUST 在抽取前拒绝");
        assert!(
            error.to_string().contains("页数"),
            "拒绝原因 MUST 说明页数: {error}"
        );
        // 入口同拒（失败降级为「未解析」，不进入 pdf-extract 的长任务）
        assert!(matches!(
            extract_text("大文档.pdf", &oversized),
            Err(ParseError::Failed(_))
        ));
        // 不可解析字节不在此处判死（交由下游报「解析失败」）
        assert!(guard_pdf_page_count(b"%PDF-1.4 not parseable").is_ok());
    }

    /// 最小可解析 PDF（lopdf 构造 N 个 Page 对象 + Pages/Catalog 树）。
    fn pdf_fixture(page_count: usize) -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};

        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let mut kids: Vec<Object> = Vec::new();
        for index in 0..page_count {
            let content_id = doc.add_object(Stream::new(
                dictionary! {},
                format!("BT (page {index}) Tj ET").into_bytes(),
            ));
            let page_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                "Contents" => content_id,
            });
            kids.push(page_id.into());
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => page_count as i64,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut out = Vec::new();
        doc.save_to(&mut out).expect("build pdf fixture");
        out
    }

    /// 把中央目录中指定条目的「声明解压大小」改写为 0（模拟少报 size 的恶意容器）。
    /// 中央目录文件头布局：签名(4) … 压缩大小 @20(4) / 解压大小 @24(4) / 名称长 @28(2)
    /// … 名称 @46。仅用于测试。
    fn under_report_declared_size(bytes: &mut [u8], entry_name: &str) {
        let mut cursor = 0usize;
        while cursor + 46 <= bytes.len() {
            if bytes[cursor..].starts_with(b"PK\x01\x02") {
                let name_len =
                    u16::from_le_bytes([bytes[cursor + 28], bytes[cursor + 29]]) as usize;
                if cursor + 46 + name_len <= bytes.len()
                    && &bytes[cursor + 46..cursor + 46 + name_len] == entry_name.as_bytes()
                {
                    bytes[cursor + 24..cursor + 28].copy_from_slice(&0u32.to_le_bytes());
                    return;
                }
            }
            cursor += 1;
        }
        panic!("中央目录中未找到条目 {entry_name}");
    }

    /// 最小 docx：仅 `word/document.xml`（抽取器只读该条目）。
    fn docx_fixture(paragraphs: &[&str]) -> Vec<u8> {
        let mut body = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
        );
        for p in paragraphs {
            body.push_str(&format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>"));
        }
        body.push_str("</w:body></w:document>");
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(body.as_bytes()).unwrap();
        zip.finish().unwrap().into_inner()
    }

    /// 最小 xlsx：sheet1.xml（内联字符串），工作表名 Sheet1。
    fn xlsx_fixture(sheets: &[(&str, &[&[&str]])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", opts).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        )
        .unwrap();
        zip.start_file("_rels/.rels", opts).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        )
        .unwrap();
        zip.start_file("xl/workbook.xml", opts).unwrap();
        let sheet_refs: String = sheets
            .iter()
            .enumerate()
            .map(|(i, (name, _))| {
                format!(
                    r#"<sheet name="{name}" sheetId="{}" r:id="rId{}"/>"#,
                    i + 1,
                    i + 1
                )
            })
            .collect();
        zip.write_all(
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{sheet_refs}</sheets></workbook>"#
            )
            .as_bytes(),
        )
        .unwrap();
        zip.start_file("xl/_rels/workbook.xml.rels", opts).unwrap();
        let rels: String = sheets
            .iter()
            .enumerate()
            .map(|(i, _)| {
                format!(
                    r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{}.xml"/>"#,
                    i + 1,
                    i + 1
                )
            })
            .collect();
        zip.write_all(
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
            )
            .as_bytes(),
        )
        .unwrap();
        for (i, (_, rows)) in sheets.iter().enumerate() {
            zip.start_file(format!("xl/worksheets/sheet{}.xml", i + 1), opts)
                .unwrap();
            let mut xml = String::from(
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
            );
            for (r, row) in rows.iter().enumerate() {
                xml.push_str(&format!(r#"<row r="{}">"#, r + 1));
                for (c, cell) in row.iter().enumerate() {
                    let col = (b'A' + c as u8) as char;
                    xml.push_str(&format!(
                        r#"<c r="{col}{}" t="inlineStr"><is><t>{cell}</t></is></c>"#,
                        r + 1
                    ));
                }
                xml.push_str("</row>");
            }
            xml.push_str("</sheetData></worksheet>");
            zip.write_all(xml.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
}
