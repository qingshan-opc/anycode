//! Extract readable text from OOXML office uploads (xlsx / docx / pptx).
//!
//! xlsx goes through calamine (shared-strings and typed cells need real
//! resolution — string splitting would silently corrupt them); docx/pptx are
//! single-pass text-run extraction over the zip members, mirroring the
//! read-side pattern in `anycode-tools` `verification/office.rs`.

use anyhow::{bail, Context, Result};
use calamine::{open_workbook_from_rs, Data, Reader, Xlsx};
use std::io::{Cursor, Read};

const MAX_ROWS_PER_SHEET: usize = 2000;

/// xlsx → per-sheet `## sheet: <name>` + tab-joined rows (empty rows skipped).
pub fn extract_xlsx_text(raw: &[u8]) -> Result<String> {
    let mut wb: Xlsx<_> = open_workbook_from_rs(Cursor::new(raw))
        .map_err(|e| anyhow::anyhow!("xlsx open failed: {e}"))?;
    let mut out = String::new();
    for name in wb.sheet_names().to_vec() {
        let Ok(range) = wb.worksheet_range(&name) else {
            continue;
        };
        out.push_str(&format!("## sheet: {name}\n"));
        let mut rows = 0usize;
        for row in range.rows() {
            if rows >= MAX_ROWS_PER_SHEET {
                out.push_str("...[sheet truncated]\n");
                break;
            }
            let cells: Vec<String> = row.iter().map(cell_text).collect();
            if cells.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            out.push_str(&cells.join("\t"));
            out.push('\n');
            rows += 1;
        }
        out.push('\n');
    }
    if out.trim().is_empty() {
        bail!("xlsx: no readable sheet content");
    }
    Ok(out)
}

fn cell_text(c: &Data) -> String {
    match c {
        Data::Empty => String::new(),
        other => other.to_string(),
    }
}

/// docx → paragraph text from `word/document.xml` (`<w:t>` runs, break on `</w:p>`).
pub fn extract_docx_text(raw: &[u8]) -> Result<String> {
    let xml = zip_member_to_string(raw, "word/document.xml")?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = String::new();
    let mut buf = Vec::new();
    let mut in_run = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => in_run = e.name().as_ref() == b"w:t",
            Ok(quick_xml::events::Event::Text(t)) if in_run => {
                let text = t.unescape().context("docx: malformed text run")?;
                out.push_str(&text);
            }
            Ok(quick_xml::events::Event::End(e)) => match e.name().as_ref() {
                b"w:t" => in_run = false,
                b"w:p" => out.push('\n'),
                _ => {}
            },
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("docx xml parse failed: {e}")),
            _ => {}
        }
        buf.clear();
    }
    if out.trim().is_empty() {
        bail!("docx: no readable text");
    }
    Ok(out)
}

/// pptx → `--- slide N ---` + `<a:t>` runs, slides in natural order.
pub fn extract_pptx_text(raw: &[u8]) -> Result<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(raw)).context("pptx: open zip failed")?;
    let mut slides: Vec<(u64, String)> = Vec::new();
    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            continue;
        };
        let Some(n) = slide_number(entry.name()) else {
            continue;
        };
        let mut xml = String::new();
        if entry.read_to_string(&mut xml).is_ok() {
            slides.push((n, xml));
        }
    }
    if slides.is_empty() {
        bail!("pptx: no slides found");
    }
    slides.sort_by_key(|(n, _)| *n);
    let mut out = String::new();
    for (n, xml) in &slides {
        out.push_str(&format!("--- slide {n} ---\n"));
        out.push_str(&collect_text_runs(xml, b"a:t"));
        out.push('\n');
    }
    Ok(out)
}

/// `ppt/slides/slide12.xml` → `Some(12)`; layouts/masters live under other dirs.
fn slide_number(name: &str) -> Option<u64> {
    name.strip_prefix("ppt/slides/slide")?
        .strip_suffix(".xml")?
        .parse()
        .ok()
}

fn collect_text_runs(xml: &str, tag: &[u8]) -> String {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = String::new();
    let mut buf = Vec::new();
    let mut in_run = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => in_run = e.name().as_ref() == tag,
            Ok(quick_xml::events::Event::Text(t)) if in_run => {
                if let Ok(text) = t.unescape() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push_str(trimmed);
                        out.push('\n');
                    }
                }
            }
            Ok(quick_xml::events::Event::End(e)) if e.name().as_ref() == tag => in_run = false,
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

fn zip_member_to_string(raw: &[u8], member: &str) -> Result<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(raw)).context("open zip failed")?;
    let mut entry = archive
        .by_name(member)
        .with_context(|| format!("missing {member}"))?;
    let mut s = String::new();
    entry
        .read_to_string(&mut s)
        .with_context(|| format!("read {member} failed"))?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_bytes(members: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in members {
            w.start_file(*name, opts).expect("start_file");
            w.write_all(body.as_bytes()).expect("write");
        }
        w.finish().expect("finish").into_inner()
    }

    #[test]
    fn docx_extracts_paragraph_text() {
        let doc = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
<w:p><w:r><w:t>Hello </w:t></w:r><w:r><w:t>world</w:t></w:r></w:p>
<w:p><w:r><w:t>second &amp; para</w:t></w:r></w:p>
</w:body>
</w:document>"#;
        let raw = zip_bytes(&[("word/document.xml", doc)]);
        let text = extract_docx_text(&raw).expect("extract");
        assert!(text.contains("Hello world"));
        assert!(text.contains("second & para"));
        // 两个段落之间有换行
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.iter().any(|l| l.contains("Hello world")));
        assert!(lines.iter().any(|l| l.contains("second & para")));
    }

    #[test]
    fn docx_without_document_xml_errors() {
        let raw = zip_bytes(&[("word/other.xml", "<x/>")]);
        assert!(extract_docx_text(&raw).is_err());
    }

    #[test]
    fn pptx_extracts_slides_in_order() {
        let slide = |t: &str| {
            format!(
                r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
            )
        };
        // 故意乱序放 slide10 与 slide2，验证自然排序
        let raw = zip_bytes(&[
            ("ppt/slides/slide10.xml", &slide("ten")),
            ("ppt/slides/slide2.xml", &slide("two")),
            ("ppt/slides/slide1.xml", &slide("one")),
        ]);
        let text = extract_pptx_text(&raw).expect("extract");
        let i1 = text.find("--- slide 1 ---").expect("slide 1");
        let i2 = text.find("--- slide 2 ---").expect("slide 2");
        let i10 = text.find("--- slide 10 ---").expect("slide 10");
        assert!(i1 < i2 && i2 < i10);
        assert!(text.contains("one") && text.contains("two") && text.contains("ten"));
    }

    #[test]
    fn pptx_without_slides_errors() {
        let raw = zip_bytes(&[("ppt/presentation.xml", "<x/>")]);
        assert!(extract_pptx_text(&raw).is_err());
    }

    #[test]
    fn xlsx_extracts_sheet_rows() {
        let content_types = r#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#;
        let workbook = r#"<?xml version="1.0"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="S1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
        let rels = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
        let sheet = r#"<?xml version="1.0"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>name</t></is></c><c r="B1" t="inlineStr"><is><t>score</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>alice</t></is></c><c r="B2"><v>42</v></c></row>
</sheetData>
</worksheet>"#;
        let raw = zip_bytes(&[
            ("[Content_Types].xml", content_types),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", rels),
            ("xl/worksheets/sheet1.xml", sheet),
        ]);
        let text = extract_xlsx_text(&raw).expect("extract");
        assert!(text.contains("## sheet: S1"));
        assert!(text.contains("name\tscore"));
        assert!(text.contains("alice\t42"));
    }

    #[test]
    fn xlsx_rejects_garbage() {
        assert!(extract_xlsx_text(b"not a zip").is_err());
    }
}
