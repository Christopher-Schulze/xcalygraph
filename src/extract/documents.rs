//! Binary documents (`docs/spec.md` section 5): the PDF text layer and outline, DOCX paragraphs
//! with heading styles and XLSX sheets with their strings. Locators are pages (PDF), paragraphs
//! (DOCX) and sheets (XLSX). Unreadable files keep their path and one notice as file doc.

use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::prose::Outline;
use crate::domain::facts::Facts;

/// Decompressed content read per PDF page or XML part, against decompression bombs.
const PART_LIMIT: usize = 16 << 20;

pub fn pdf(content: &[u8]) -> Facts {
    let document = match lopdf::Document::load_mem(content) {
        Ok(document) => document,
        Err(lopdf::Error::Decryption(_)) => return unreadable("encrypted PDF"),
        Err(_) if contains(content, b"/Encrypt") => return unreadable("encrypted PDF"),
        Err(_) => return unreadable("malformed PDF"),
    };
    let pages: Vec<u32> = document.get_pages().keys().copied().collect();
    let toc = document.get_toc().map(|toc| toc.toc).unwrap_or_default();
    let mut outline = Outline::with_text(pages.len() as u32);
    let mut entries = toc.iter().peekable();
    for &page in &pages {
        while let Some(entry) = entries.next_if(|entry| entry.page as u32 <= page) {
            outline.heading(entry.level, &entry.title, (entry.page as u32).max(1));
        }
        if outline.text_full() {
            continue;
        }
        if let Ok(text) = document.extract_text_with_limit(&[page], PART_LIMIT) {
            for line in text.lines() {
                outline.text(line);
            }
        }
    }
    for entry in entries {
        outline.heading(entry.level, &entry.title, (entry.page as u32).max(1));
    }
    let mut facts = outline.finish();
    facts.lines = pages.len() as u32;
    facts
}

/// DOCX: `word/document.xml` paragraphs; `Title` and `HeadingN` styles are headings; relative
/// hyperlink targets from `word/_rels/document.xml.rels` are path references.
pub fn docx(content: &[u8]) -> Facts {
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(content)) else {
        return unreadable("unreadable DOCX (encrypted or malformed)");
    };
    let Some(document) = part(&mut archive, "word/document.xml") else {
        return unreadable("unreadable DOCX (encrypted or malformed)");
    };
    let paragraphs = docx_paragraphs(&document);
    let mut outline = Outline::with_text(paragraphs.len() as u32);
    for (index, (level, text)) in paragraphs.iter().enumerate() {
        let number = index as u32 + 1;
        match level {
            Some(level) => outline.heading(*level, text, number),
            None => outline.prose(text, number),
        }
    }
    if let Some(relations) = part(&mut archive, "word/_rels/document.xml.rels") {
        for target in external_targets(&relations) {
            outline.path(&target, 1);
        }
    }
    let mut facts = outline.finish();
    facts.lines = paragraphs.len() as u32;
    facts
}

/// Paragraphs with their heading level (`Title` 1, `HeadingN` N).
fn docx_paragraphs(xml: &str) -> Vec<(Option<usize>, String)> {
    let mut reader = Reader::from_str(xml);
    let mut paragraphs = Vec::new();
    let mut text = String::new();
    let mut level = None;
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag)) => match tag.local_name().as_ref() {
                "pStyle" => {
                    level = attribute(&tag, "w:val").and_then(|style| heading_level(&style))
                }
                "t" => in_text = true,
                "tab" | "br" => text.push(' '),
                _ => {}
            },
            Ok(Event::End(tag)) => match tag.local_name().as_ref() {
                "t" => in_text = false,
                "p" => {
                    paragraphs.push((level.take(), std::mem::take(&mut text)));
                }
                _ => {}
            },
            Ok(Event::Text(chunk)) if in_text => text.push_str(&chunk),
            Ok(Event::GeneralRef(entity)) if in_text => text.push_str(&resolve_entity(&entity)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    paragraphs
}

fn heading_level(style: &str) -> Option<usize> {
    if style.eq_ignore_ascii_case("title") {
        return Some(1);
    }
    let digits = style
        .strip_prefix("Heading")
        .or_else(|| style.strip_prefix("heading"))?;
    digits
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|level| (1..=9).contains(level))
}

/// XLSX: sheets in workbook order as headings (locator = sheet number), their shared, inline
/// and formula strings as text.
pub fn xlsx(content: &[u8]) -> Facts {
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(content)) else {
        return unreadable("unreadable XLSX (encrypted or malformed)");
    };
    let Some(workbook) = part(&mut archive, "xl/workbook.xml") else {
        return unreadable("unreadable XLSX (encrypted or malformed)");
    };
    let sheets = workbook_sheets(&workbook);
    let relations = part(&mut archive, "xl/_rels/workbook.xml.rels")
        .map(|xml| relation_targets(&xml))
        .unwrap_or_default();
    let shared = part(&mut archive, "xl/sharedStrings.xml")
        .map(|xml| shared_strings(&xml))
        .unwrap_or_default();
    let mut outline = Outline::with_text(sheets.len() as u32);
    for (index, (name, relation)) in sheets.iter().enumerate() {
        outline.heading(1, name, index as u32 + 1);
        let Some(target) = relations
            .iter()
            .find(|(id, _)| id == relation)
            .map(|(_, target)| target)
        else {
            continue;
        };
        let path = match target.strip_prefix('/') {
            Some(absolute) => absolute.to_owned(),
            None => format!("xl/{target}"),
        };
        if outline.text_full() {
            continue;
        }
        if let Some(sheet) = part(&mut archive, &path) {
            for text in sheet_strings(&sheet, &shared) {
                outline.text(&text);
            }
        }
    }
    let mut facts = outline.finish();
    facts.lines = sheets.len() as u32;
    facts
}

/// `(name, relationship id)` per `<sheet>` of the workbook.
fn workbook_sheets(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let mut sheets = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag))
                if tag.local_name().as_ref() == "sheet" =>
            {
                if let (Some(name), Some(id)) = (attribute(&tag, "name"), attribute(&tag, "r:id")) {
                    sheets.push((name, id));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    sheets
}

/// `(Id, Target)` per relationship.
fn relation_targets(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let mut relations = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag))
                if tag.local_name().as_ref() == "Relationship" =>
            {
                if let (Some(id), Some(target)) = (attribute(&tag, "Id"), attribute(&tag, "Target"))
                {
                    relations.push((id, target));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    relations
}

/// Targets of external relationships that name local files (no URL scheme).
fn external_targets(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut targets = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag))
                if tag.local_name().as_ref() == "Relationship" =>
            {
                let external = attribute(&tag, "TargetMode").is_some_and(|mode| mode == "External");
                if let Some(target) = attribute(&tag, "Target").filter(|_| external) {
                    let target = target
                        .strip_prefix("file:///")
                        .map_or(target.clone(), str::to_owned);
                    if !target.contains(':') {
                        targets.push(target);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    targets
}

fn shared_strings(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) if tag.local_name().as_ref() == "t" => in_text = true,
            Ok(Event::End(tag)) => match tag.local_name().as_ref() {
                "t" => in_text = false,
                "si" => strings.push(std::mem::take(&mut current)),
                _ => {}
            },
            Ok(Event::Text(chunk)) if in_text => current.push_str(&chunk),
            Ok(Event::GeneralRef(entity)) if in_text => current.push_str(&resolve_entity(&entity)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    strings
}

/// String cells of a sheet in document order: shared (`t="s"`), inline and formula strings.
fn sheet_strings(xml: &str, shared: &[String]) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut strings = Vec::new();
    let mut cell_type = String::new();
    let mut value = String::new();
    let mut in_value = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => match tag.local_name().as_ref() {
                "c" => cell_type = attribute(&tag, "t").unwrap_or_default(),
                "v" | "t" => in_value = true,
                _ => {}
            },
            Ok(Event::End(tag)) => match tag.local_name().as_ref() {
                "v" | "t" => in_value = false,
                "c" => {
                    let text = std::mem::take(&mut value);
                    let resolved = match cell_type.as_str() {
                        "s" => text
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| shared.get(index).cloned()),
                        "inlineStr" | "str" => Some(text),
                        _ => None,
                    };
                    if let Some(text) = resolved.filter(|text| !text.trim().is_empty()) {
                        strings.push(text);
                    }
                }
                _ => {}
            },
            Ok(Event::Text(chunk)) if in_value => value.push_str(&chunk),
            Ok(Event::GeneralRef(entity)) if in_value => value.push_str(&resolve_entity(&entity)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    strings
}

/// A ZIP member as UTF-8, bounded by `PART_LIMIT`.
fn part(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let file = archive.by_name(name).ok()?;
    let mut text = String::new();
    file.take(PART_LIMIT as u64)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

fn attribute(tag: &BytesStart, name: &str) -> Option<String> {
    let attribute = tag.try_get_attribute(name).ok()??;
    attribute
        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
        .ok()
        .map(|value| value.into_owned())
}

fn resolve_entity(name: &str) -> String {
    match name {
        "amp" => "&".to_owned(),
        "lt" => "<".to_owned(),
        "gt" => ">".to_owned(),
        "quot" => "\"".to_owned(),
        "apos" => "'".to_owned(),
        _ => {
            let code = name
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| {
                    name.strip_prefix('#')
                        .and_then(|decimal| decimal.parse().ok())
                });
            code.and_then(char::from_u32)
                .map(String::from)
                .unwrap_or_default()
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn unreadable(reason: &str) -> Facts {
    Facts {
        doc: format!("! {reason}, text unavailable"),
        ..Facts::default()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            for (name, content) in parts {
                let started = writer.start_file(*name, options);
                assert!(started.is_ok());
                assert!(writer.write_all(content.as_bytes()).is_ok());
            }
            assert!(writer.finish().is_ok());
        }
        buffer.into_inner()
    }

    #[test]
    fn docx_headings_text_and_relative_links() {
        let document = r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Store design</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Call `Store.open` &amp; </w:t></w:r><w:r><w:t>wait.</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Limits</w:t></w:r></w:p>
<w:p><w:r><w:t>Max 10 rows.</w:t></w:r></w:p>
</w:body></w:document>"#;
        let relations = r#"<Relationships><Relationship Id="r1" Type="hyperlink" Target="../docs/api.md" TargetMode="External"/><Relationship Id="r2" Type="hyperlink" Target="https://x.io" TargetMode="External"/><Relationship Id="r3" Type="styles" Target="styles.xml"/></Relationships>"#;
        let facts = docx(&zip_of(&[
            ("word/document.xml", document),
            ("word/_rels/document.xml.rels", relations),
        ]));
        let headings: Vec<String> = facts
            .symbols
            .iter()
            .map(|symbol| {
                format!(
                    "{} {}-{} {:?}",
                    symbol.name, symbol.line, symbol.end_line, symbol.parent
                )
            })
            .collect();
        assert_eq!(headings, ["Store design 1-4 None", "Limits 3-4 Some(0)"]);
        let refs: Vec<String> = facts
            .refs
            .iter()
            .map(|r| format!("{:?} {}", r.kind, r.name))
            .collect();
        assert_eq!(refs, ["Mention Store.open", "Path ../docs/api.md"]);
        assert_eq!(facts.doc, "Call `Store.open` & wait.");
        assert_eq!(facts.lines, 4);
        assert_eq!(facts.texts[1].text, "Max 10 rows.");
    }

    #[test]
    fn xlsx_sheets_and_strings() {
        let workbook = r#"<workbook><sheets><sheet name="Limits" sheetId="1" r:id="rId1"/><sheet name="Owners" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
        let relations = r#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Target="/xl/worksheets/sheet2.xml"/></Relationships>"#;
        let shared = r#"<sst><si><t>max_rows</t></si><si><r><t>retry</t></r><r><t> budget</t></r></si></sst>"#;
        let sheet1 = r#"<worksheet><sheetData><row><c r="A1" t="s"><v>0</v></c><c r="B1"><v>10</v></c></row><row><c r="A2" t="s"><v>1</v></c><c r="B2" t="inlineStr"><is><t>per job</t></is></c></row></sheetData></worksheet>"#;
        let sheet2 = r#"<worksheet><sheetData><row><c r="A1" t="str"><f>A</f><v>team &amp; ops</v></c></row></sheetData></worksheet>"#;
        let facts = xlsx(&zip_of(&[
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", relations),
            ("xl/sharedStrings.xml", shared),
            ("xl/worksheets/sheet1.xml", sheet1),
            ("xl/worksheets/sheet2.xml", sheet2),
        ]));
        let sheets: Vec<&str> = facts
            .symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert_eq!(sheets, ["Limits", "Owners"]);
        let texts: Vec<(String, u32, String)> = facts
            .texts
            .iter()
            .map(|segment| {
                (
                    segment.heading.clone(),
                    segment.locator,
                    segment.text.clone(),
                )
            })
            .collect();
        assert_eq!(
            texts,
            [
                (
                    "Limits".to_owned(),
                    1,
                    "max_rows retry budget per job".to_owned()
                ),
                ("Owners".to_owned(), 2, "team & ops".to_owned())
            ]
        );
    }

    /// A two-page PDF with a text layer and a two-entry outline, xref offsets computed.
    fn sample_pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /Outlines 5 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R 7 0 R] /Count 2 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /Font << /F1 9 0 R >> >> >>".to_owned(),
            stream("BT /F1 12 Tf 10 100 Td (Store design notes) Tj ET"),
            "<< /Type /Outlines /First 6 0 R /Last 10 0 R /Count 2 >>".to_owned(),
            "<< /Title (Intro) /Parent 5 0 R /Next 10 0 R /Dest [3 0 R /Fit] >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 8 0 R /Resources << /Font << /F1 9 0 R >> >> >>".to_owned(),
            stream("BT /F1 12 Tf 10 100 Td (Limits apply to rows) Tj ET"),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_owned(),
            "<< /Title (Limits) /Parent 5 0 R /Prev 6 0 R /Dest [7 0 R /Fit] >>".to_owned(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    fn stream(content: &str) -> String {
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
    }

    #[test]
    fn pdf_outline_pages_and_text() {
        let facts = pdf(&sample_pdf());
        let headings: Vec<String> = facts
            .symbols
            .iter()
            .map(|symbol| format!("{} {}-{}", symbol.name, symbol.line, symbol.end_line))
            .collect();
        assert_eq!(headings, ["Intro 1-1", "Limits 2-2"]);
        assert_eq!(facts.lines, 2);
        let texts: Vec<(String, u32, String)> = facts
            .texts
            .iter()
            .map(|segment| {
                (
                    segment.heading.clone(),
                    segment.locator,
                    segment.text.clone(),
                )
            })
            .collect();
        assert_eq!(
            texts,
            [
                ("Intro".to_owned(), 1, "Store design notes".to_owned()),
                ("Limits".to_owned(), 2, "Limits apply to rows".to_owned())
            ]
        );
        assert_eq!(facts.doc, "Store design notes");
    }

    #[test]
    fn unreadable_documents_keep_one_notice() {
        assert_eq!(
            pdf(b"%PDF-1.7 broken").doc,
            "! malformed PDF, text unavailable"
        );
        assert_eq!(
            pdf(b"%PDF-1.4\n1 0 obj << /Encrypt 2 0 R >> endobj\n").doc,
            "! encrypted PDF, text unavailable"
        );
        assert_eq!(
            docx(b"\xd0\xcf\x11\xe0 ole").doc,
            "! unreadable DOCX (encrypted or malformed), text unavailable"
        );
        assert!(
            xlsx(&zip_of(&[("other.xml", "<x/>")]))
                .doc
                .starts_with("! unreadable XLSX")
        );
    }
}
