//! Document text and source locators: PDF pages, Word/RTF paragraphs, spreadsheet sheets and
//! presentation slides. Shared bounded XML reads account for partial and malformed packages;
//! legacy Office uses real compound-file parsers. Unreadable files retain their path and coverage.

use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::prose::Outline;
use crate::domain::facts::{self, Coverage, Facts};

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
    let mut coverage = Coverage::Complete;
    let mut entries = toc.iter().peekable();
    for &page in &pages {
        while let Some(entry) = entries.next_if(|entry| entry.page as u32 <= page) {
            outline.heading(entry.level, &entry.title, (entry.page as u32).max(1));
        }
        outline.at(page);
        match document.extract_text_with_limit(&[page], PART_LIMIT) {
            Ok(text) => {
                for line in text.lines() {
                    outline.text(line);
                }
            }
            Err(_) => coverage = coverage.max(Coverage::ParseErrors),
        }
    }
    for entry in entries {
        outline.heading(entry.level, &entry.title, (entry.page as u32).max(1));
    }
    let mut facts = outline.finish();
    facts.lines = pages.len() as u32;
    facts.coverage = coverage;
    facts
}

/// DOCX: `word/document.xml` paragraphs; `Title` and `HeadingN` styles are headings; relative
/// hyperlink targets from `word/_rels/document.xml.rels` are path references.
pub fn docx(content: &[u8]) -> Facts {
    let mut coverage = Coverage::Complete;
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(content)) else {
        return unreadable("unreadable DOCX (encrypted or malformed)");
    };
    let Some(document) = part(&mut archive, "word/document.xml", true, &mut coverage) else {
        return Facts {
            coverage,
            ..unreadable("DOCX main part missing, unreadable or over size limit")
        };
    };
    let paragraphs = docx_paragraphs(&document, &mut coverage);
    let mut outline = Outline::with_text(paragraphs.len() as u32);
    for (index, (level, text)) in paragraphs.iter().enumerate() {
        let number = index as u32 + 1;
        match level {
            Some(level) => outline.heading(*level, text, number),
            None => outline.prose(text, number),
        }
    }
    if let Some(relations) = part(
        &mut archive,
        "word/_rels/document.xml.rels",
        false,
        &mut coverage,
    ) {
        for target in external_targets(&relations, &mut coverage) {
            outline.path(&target, 1);
        }
    }
    let mut facts = outline.finish();
    facts.lines = paragraphs.len() as u32;
    facts.coverage = coverage;
    facts
}

/// Paragraphs with their heading level (`Title` 1, `HeadingN` N).
fn docx_paragraphs(xml: &str, coverage: &mut Coverage) -> Vec<(Option<usize>, String)> {
    let mut reader = XmlReader::new(xml, &["document"], coverage);
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
            Ok(Event::CData(chunk)) if in_text => text.push_str(&chunk),
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
    let mut coverage = Coverage::Complete;
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(content)) else {
        return unreadable("unreadable XLSX (encrypted or malformed)");
    };
    let Some(workbook) = part(&mut archive, "xl/workbook.xml", true, &mut coverage) else {
        return Facts {
            coverage,
            ..unreadable("unreadable XLSX workbook part (missing, damaged or over size limit)")
        };
    };
    let sheets = workbook_sheets(&workbook, &mut coverage);
    let relations = part(
        &mut archive,
        "xl/_rels/workbook.xml.rels",
        true,
        &mut coverage,
    )
    .map(|xml| relation_targets(&xml, &mut coverage))
    .unwrap_or_default();
    let shared = part(&mut archive, "xl/sharedStrings.xml", false, &mut coverage)
        .map(|xml| shared_strings(&xml, &mut coverage))
        .unwrap_or_default();
    let mut outline = Outline::with_text(sheets.len() as u32);
    for (index, (name, relation)) in sheets.iter().enumerate() {
        outline.heading(1, name, index as u32 + 1);
        let Some(target) = relations
            .iter()
            .find(|(id, _)| id == relation)
            .map(|(_, target)| target)
        else {
            coverage = coverage.max(Coverage::ParseErrors);
            continue;
        };
        let Some(path) = package_path("xl/workbook.xml", target) else {
            coverage = coverage.max(Coverage::ParseErrors);
            continue;
        };
        if let Some(sheet) = part(&mut archive, &path, true, &mut coverage) {
            for text in sheet_strings(&sheet, &shared, &mut coverage) {
                outline.text(&text);
            }
        }
    }
    let mut facts = outline.finish();
    facts.lines = sheets.len() as u32;
    facts.coverage = coverage;
    facts
}

/// PPTX: slides in order, line = slide; the title placeholder is the heading, all other text and
/// the speaker notes are its text.
pub fn pptx(content: &[u8]) -> Facts {
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(content)) else {
        return unreadable("unreadable PPTX (encrypted or malformed)");
    };
    let mut coverage = Coverage::Complete;
    let Some(presentation) = part(&mut archive, "ppt/presentation.xml", true, &mut coverage) else {
        return Facts {
            coverage,
            ..unreadable("PPTX presentation part missing, damaged or over size limit")
        };
    };
    let relations = part(
        &mut archive,
        "ppt/_rels/presentation.xml.rels",
        true,
        &mut coverage,
    )
    .map(|xml| relation_targets(&xml, &mut coverage))
    .unwrap_or_default();
    let mut reader = XmlReader::new(&presentation, &["presentation"], &mut coverage);
    let mut slides = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag))
                if tag.local_name().as_ref() == "sldId" =>
            {
                slides.push(attribute(&tag, "r:id"));
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    let mut outline = Outline::with_text(slides.len() as u32);
    for (index, relation) in slides.iter().enumerate() {
        let number = index as u32 + 1;
        let Some(slide) = relation
            .as_ref()
            .and_then(|id| relations.iter().find(|(key, _)| key == id))
            .and_then(|(_, target)| package_path("ppt/presentation.xml", target))
        else {
            coverage = coverage.max(Coverage::ParseErrors);
            continue;
        };
        let Some(xml) = part(&mut archive, &slide, true, &mut coverage) else {
            continue;
        };
        let (title, text) = slide_text(&xml, &mut coverage);
        let heading = title.unwrap_or_else(|| format!("Slide {number}"));
        outline.heading(1, &heading, number);
        for line in &text {
            outline.text(line);
        }
        let (directory, name) = slide.rsplit_once('/').unwrap_or(("", &slide));
        let notes = part(
            &mut archive,
            &format!("{directory}/_rels/{name}.rels"),
            false,
            &mut coverage,
        )
        .into_iter()
        .flat_map(|xml| relation_targets(&xml, &mut coverage))
        .filter_map(|(_, target)| package_path(&slide, &target))
        .find(|target| target.starts_with("ppt/notesSlides/"));
        if let Some(xml) = notes.and_then(|name| part(&mut archive, &name, true, &mut coverage)) {
            for line in slide_text(&xml, &mut coverage).1 {
                outline.text(&line);
            }
        }
    }
    let mut facts = outline.finish();
    facts.lines = slides.len() as u32;
    facts.coverage = coverage;
    facts
}

/// Title placeholder text and the other paragraphs of a slide or notes part.
fn slide_text(xml: &str, coverage: &mut Coverage) -> (Option<String>, Vec<String>) {
    let mut reader = XmlReader::new(xml, &["sld", "notes"], coverage);
    let mut title = None;
    let mut text = Vec::new();
    let mut is_title = false;
    let mut in_run = false;
    let mut paragraph = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) if tag.name().as_ref() == "p:sp" => is_title = false,
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag)) if tag.name().as_ref() == "p:ph" => {
                is_title = matches!(
                    attribute(&tag, "type").as_deref(),
                    Some("title" | "ctrTitle")
                );
            }
            Ok(Event::Start(tag)) if tag.name().as_ref() == "a:t" => in_run = true,
            Ok(Event::End(tag)) if tag.name().as_ref() == "a:t" => in_run = false,
            Ok(Event::Text(chunk)) if in_run => paragraph.push_str(&chunk),
            Ok(Event::CData(chunk)) if in_run => paragraph.push_str(&chunk),
            Ok(Event::GeneralRef(entity)) if in_run => paragraph.push_str(&resolve_entity(&entity)),
            Ok(Event::End(tag)) if tag.name().as_ref() == "a:p" => {
                let line = facts::normalize_space(&std::mem::take(&mut paragraph));
                if line.is_empty() {
                    continue;
                }
                if is_title && title.is_none() {
                    title = Some(line);
                } else {
                    text.push(line);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (title, text)
}

/// OpenDocument text, spreadsheet or presentation: `text:h` and sheet or page names are headings;
/// line = paragraph in a text document, sheet or page otherwise.
pub fn opendocument(content: &[u8]) -> Facts {
    let mut coverage = Coverage::Complete;
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(content)) else {
        return unreadable("unreadable OpenDocument (encrypted or malformed)");
    };
    let Some(xml) = part(&mut archive, "content.xml", true, &mut coverage) else {
        return Facts {
            coverage,
            ..unreadable("OpenDocument content missing, damaged or over size limit")
        };
    };
    let blocks = opendocument_blocks(&xml, &mut coverage);
    let mut outline = Outline::with_text(blocks.len() as u32);
    let mut pages = 0;
    for (index, (level, page, text)) in blocks.iter().enumerate() {
        let number = if *page { pages + 1 } else { index as u32 + 1 };
        match level {
            Some(level) => {
                if *page {
                    pages += 1;
                }
                outline.heading(*level, text, if *page { pages } else { number });
            }
            None if pages > 0 => outline.text(text),
            None => outline.prose(text, number),
        }
    }
    let mut facts = outline.finish();
    facts.lines = if pages > 0 {
        pages
    } else {
        blocks.len() as u32
    };
    facts.coverage = coverage;
    facts
}

/// Blocks of `content.xml`: heading level (sheets and pages count as level 1), whether it names a
/// sheet or page, and the text.
fn opendocument_blocks(xml: &str, coverage: &mut Coverage) -> Vec<(Option<usize>, bool, String)> {
    let mut reader = XmlReader::new(
        xml,
        &["document-content", "text", "spreadsheet", "presentation"],
        coverage,
    );
    let mut blocks = Vec::new();
    let mut open: Option<Option<usize>> = None;
    let mut current = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag)) => match tag.name().as_ref() {
                "text:s" | "text:tab" | "text:line-break" if open.is_some() => current.push(' '),
                "table:table" | "draw:page" => {
                    if let Some(name) =
                        attribute(&tag, "table:name").or_else(|| attribute(&tag, "draw:name"))
                    {
                        blocks.push((Some(1), true, name));
                    }
                }
                "text:h" => {
                    let level = attribute(&tag, "text:outline-level")
                        .and_then(|level| level.parse().ok())
                        .unwrap_or(1);
                    open = Some(Some(level));
                    current.clear();
                }
                "text:p" if open.is_none() => {
                    open = Some(None);
                    current.clear();
                }
                _ => {}
            },
            Ok(Event::Text(chunk)) if open.is_some() => current.push_str(&chunk),
            Ok(Event::CData(chunk)) if open.is_some() => current.push_str(&chunk),
            Ok(Event::GeneralRef(entity)) if open.is_some() => {
                current.push_str(&resolve_entity(&entity));
            }
            Ok(Event::End(tag)) if matches!(tag.name().as_ref(), "text:h" | "text:p") => {
                if let Some(level) = open.take() {
                    let text = facts::normalize_space(&current);
                    if !text.is_empty() {
                        blocks.push((level, false, text));
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    blocks
}

/// RTF: the visible text, one line per paragraph; font, color, style and picture groups skipped.
pub fn rtf(content: &[u8]) -> Facts {
    if !content.starts_with(b"{\\rtf") {
        return unreadable("unreadable RTF");
    }
    let (lines, coverage) = rtf_lines(content);
    let mut outline = Outline::with_text(lines.len() as u32);
    for (index, line) in lines.iter().enumerate() {
        outline.prose(line, index as u32 + 1);
    }
    let mut facts = outline.finish();
    facts.lines = lines.len() as u32;
    facts.coverage = coverage;
    facts
}

fn rtf_lines(content: &[u8]) -> (Vec<String>, Coverage) {
    const HIDDEN: [&[u8]; 6] = [
        b"fonttbl",
        b"colortbl",
        b"stylesheet",
        b"info",
        b"pict",
        b"header",
    ];
    let mut lines = Vec::new();
    let mut line = String::new();
    // Depth at which a hidden group opened.
    let mut hidden: Option<usize> = None;
    let mut depth = 0usize;
    let mut position = 0;
    let mut coverage = Coverage::Complete;
    let mut unicode_skip = 1usize;
    let mut unicode_stack = Vec::new();
    let mut encoding = encoding_rs::WINDOWS_1252;
    let mut ansi = Vec::new();
    let mut fallback = 0usize;
    let mut high_surrogate = None;
    while let Some(&byte) = content.get(position) {
        position += 1;
        match byte {
            b'{' => {
                rtf_ansi(&mut ansi, encoding, &mut line, &mut coverage);
                depth += 1;
                unicode_stack.push((unicode_skip, encoding));
                fallback = 0;
            }
            b'}' => {
                rtf_ansi(&mut ansi, encoding, &mut line, &mut coverage);
                if hidden == Some(depth) {
                    hidden = None;
                }
                if let Some((skip, prior_encoding)) = unicode_stack.pop() {
                    unicode_skip = skip;
                    encoding = prior_encoding;
                } else {
                    coverage = coverage.max(Coverage::ParseErrors);
                }
                depth = depth.saturating_sub(1);
                fallback = 0;
            }
            b'\\' => {
                let start = position;
                while content.get(position).is_some_and(u8::is_ascii_alphabetic) {
                    position += 1;
                }
                let word = &content[start..position];
                let number_start = position;
                while content
                    .get(position)
                    .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'-')
                {
                    position += 1;
                }
                let number = std::str::from_utf8(&content[number_start..position])
                    .ok()
                    .and_then(|text| text.parse::<i32>().ok());
                if word.is_empty() {
                    // A control symbol: `\\'e4` is a code page byte, others stand for themselves.
                    let symbol = content.get(position).copied();
                    position += 1;
                    if hidden.is_some() {
                        continue;
                    }
                    match symbol {
                        Some(b'\'') => {
                            let hex = content.get(position..position + 2);
                            position += 2;
                            if fallback > 0 {
                                fallback -= 1;
                                continue;
                            }
                            if let Some(value) = hex
                                .and_then(|hex| std::str::from_utf8(hex).ok())
                                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                            {
                                ansi.push(value);
                            } else {
                                coverage = coverage.max(Coverage::ParseErrors);
                            }
                        }
                        Some(b'*') => hidden = hidden.or(Some(depth)),
                        Some(symbol @ (b'\\' | b'{' | b'}')) => {
                            if fallback > 0 {
                                fallback -= 1;
                            } else {
                                ansi.push(symbol);
                            }
                        }
                        // An escaped line break is a paragraph.
                        Some(b'\n' | b'\r') => {
                            rtf_ansi(&mut ansi, encoding, &mut line, &mut coverage);
                            lines.push(facts::normalize_space(&std::mem::take(&mut line)));
                        }
                        _ => {}
                    }
                    continue;
                }
                if content.get(position) == Some(&b' ') {
                    position += 1;
                }
                if word == b"bin" {
                    if let Some(length) = number.and_then(|number| usize::try_from(number).ok())
                        && let Some(end) = position
                            .checked_add(length)
                            .filter(|end| *end <= content.len())
                    {
                        position = end;
                    } else {
                        coverage = coverage.max(Coverage::ParseErrors);
                        break;
                    }
                    continue;
                }
                if HIDDEN.contains(&word) {
                    hidden = hidden.or(Some(depth));
                }
                if hidden.is_some() {
                    continue;
                }
                if fallback > 0 && word != b"u" && word != b"uc" {
                    fallback -= 1;
                    continue;
                }
                match word {
                    b"ansicpg" => {
                        rtf_ansi(&mut ansi, encoding, &mut line, &mut coverage);
                        let label = number.map(|number| match number {
                            932 => "shift_jis".to_owned(),
                            936 => "gbk".to_owned(),
                            949 => "euc-kr".to_owned(),
                            950 => "big5".to_owned(),
                            10000 => "macintosh".to_owned(),
                            65001 => "utf-8".to_owned(),
                            _ => format!("windows-{number}"),
                        });
                        if let Some(next) = label
                            .as_deref()
                            .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
                        {
                            encoding = next;
                        } else {
                            coverage = coverage.max(Coverage::NoGrammar);
                            break;
                        }
                    }
                    b"par" | b"line" | b"sect" | b"page" => {
                        rtf_ansi(&mut ansi, encoding, &mut line, &mut coverage);
                        lines.push(facts::normalize_space(&std::mem::take(&mut line)));
                    }
                    b"tab" | b"cell" => {
                        rtf_ansi(&mut ansi, encoding, &mut line, &mut coverage);
                        line.push(' ');
                    }
                    b"uc" => {
                        if let Some(skip) = number.and_then(|number| usize::try_from(number).ok()) {
                            unicode_skip = skip;
                        } else {
                            coverage = coverage.max(Coverage::ParseErrors);
                        }
                    }
                    b"u" => {
                        rtf_ansi(&mut ansi, encoding, &mut line, &mut coverage);
                        if let Some(unit) = number.and_then(|code| {
                            u16::try_from(if code < 0 { code + 65536 } else { code }).ok()
                        }) {
                            match unit {
                                0xD800..=0xDBFF => {
                                    if high_surrogate.replace(unit).is_some() {
                                        coverage = coverage.max(Coverage::ParseErrors);
                                    }
                                }
                                0xDC00..=0xDFFF => {
                                    if let Some(high) = high_surrogate.take()
                                        && let Some(character) = char::from_u32(
                                            0x10000
                                                + ((u32::from(high) - 0xD800) << 10)
                                                + u32::from(unit)
                                                - 0xDC00,
                                        )
                                    {
                                        line.push(character);
                                    } else {
                                        coverage = coverage.max(Coverage::ParseErrors);
                                    }
                                }
                                _ => {
                                    if high_surrogate.take().is_some() {
                                        coverage = coverage.max(Coverage::ParseErrors);
                                    }
                                    if let Some(character) = char::from_u32(u32::from(unit)) {
                                        line.push(character);
                                    }
                                }
                            }
                        } else {
                            coverage = coverage.max(Coverage::ParseErrors);
                        }
                        fallback = unicode_skip;
                    }
                    _ => {}
                }
            }
            b'\r' | b'\n' => {}
            _ if hidden.is_none() => {
                if fallback > 0 {
                    fallback -= 1;
                } else {
                    ansi.push(byte);
                }
            }
            _ => {}
        }
    }
    rtf_ansi(&mut ansi, encoding, &mut line, &mut coverage);
    lines.push(facts::normalize_space(&line));
    if depth != 0 || high_surrogate.is_some() {
        coverage = coverage.max(Coverage::ParseErrors);
    }
    (lines, coverage)
}

fn rtf_ansi(
    bytes: &mut Vec<u8>,
    encoding: &'static encoding_rs::Encoding,
    line: &mut String,
    coverage: &mut Coverage,
) {
    let (text, invalid) = encoding.decode_without_bom_handling(bytes);
    line.push_str(&text);
    if invalid {
        *coverage = (*coverage).max(Coverage::ParseErrors);
    }
    bytes.clear();
}

/// Real compound-file readers; never promote arbitrary binary byte runs to facts.
pub fn legacy_office(path: &str, content: &[u8]) -> Facts {
    use office_oxide::{Document, DocumentFormat, options::ParseOptions};
    let Some(format @ (DocumentFormat::Doc | DocumentFormat::Xls | DocumentFormat::Ppt)) = path
        .rsplit('.')
        .next()
        .and_then(DocumentFormat::from_extension)
    else {
        return unreadable("unsupported legacy Office format");
    };
    let mut options = ParseOptions::default();
    options.limits.part_bytes = PART_LIMIT as u64;
    options.limits.text_chars = PART_LIMIT;
    let document = match Document::from_reader_with(Cursor::new(content.to_vec()), format, &options)
    {
        Ok(document) => document,
        Err(_) => return unreadable("unreadable legacy Office file (encrypted or malformed)"),
    };
    let mut facts = if let Some(workbook) = document.as_xls() {
        legacy_sheets(workbook)
    } else if let Some(presentation) = document.as_ppt() {
        legacy_slides(presentation)
    } else {
        let text = document.plain_text();
        let mut outline = Outline::with_text(text.lines().count() as u32);
        for (index, line) in text.lines().enumerate() {
            outline.prose(line, index as u32 + 1);
        }
        let mut facts = outline.finish();
        facts.lines = text.lines().count() as u32;
        facts
    };
    if let Some(damage) = document.damage().iter().find(|damage| damage.is_loss()) {
        facts.coverage = Coverage::ParseErrors;
        facts.doc = format!(
            "! incomplete legacy Office: {}",
            facts::clip_space(&damage.to_string(), 120)
        );
    }
    facts
}

fn legacy_sheets(workbook: &office_oxide::xls::XlsDocument) -> Facts {
    let mut outline = Outline::with_text(workbook.sheets.len() as u32);
    for (index, sheet) in workbook.sheets.iter().enumerate() {
        outline.heading(1, &sheet.name, index as u32 + 1);
        for (row, cells) in sheet.rows.iter().enumerate() {
            for column in 0..cells.len() {
                if let Some(text) = sheet.display_text(row, column) {
                    outline.text(&text);
                }
            }
        }
        for text in &sheet.chart_text {
            outline.text(text);
        }
        for comment in &sheet.comments {
            outline.text(&comment.text);
        }
    }
    let mut facts = outline.finish();
    facts.lines = workbook.sheets.len() as u32;
    facts
}

fn legacy_slides(presentation: &office_oxide::ppt::PptDocument) -> Facts {
    use office_oxide::ppt::TextType;
    let mut outline = Outline::with_text(presentation.slides.len() as u32);
    for (index, slide) in presentation.slides.iter().enumerate() {
        let number = index as u32 + 1;
        let title = slide
            .text_runs
            .iter()
            .find(|run| matches!(run.text_type, TextType::Title | TextType::CenterTitle));
        let heading = title.map_or_else(
            || format!("Slide {number}"),
            |run| facts::normalize_space(&run.text),
        );
        outline.heading(1, &heading, number);
        for run in slide.text_runs.iter().chain(
            slide
                .tables
                .iter()
                .flat_map(|table| table.rows.iter().flatten().flatten()),
        ) {
            outline.text(&run.text);
        }
    }
    let mut facts = outline.finish();
    facts.lines = presentation.slides.len() as u32;
    facts
}

/// `(name, relationship id)` per `<sheet>` of the workbook.
fn workbook_sheets(xml: &str, coverage: &mut Coverage) -> Vec<(String, String)> {
    let mut reader = XmlReader::new(xml, &["workbook"], coverage);
    let mut sheets = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag))
                if tag.local_name().as_ref() == "sheet" =>
            {
                if let (Some(name), Some(id)) = (attribute(&tag, "name"), attribute(&tag, "r:id")) {
                    sheets.push((name, id));
                } else {
                    *reader.coverage = (*reader.coverage).max(Coverage::ParseErrors);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    sheets
}

/// `(Id, Target)` per relationship.
fn relation_targets(xml: &str, coverage: &mut Coverage) -> Vec<(String, String)> {
    let mut reader = XmlReader::new(xml, &["Relationships"], coverage);
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
fn external_targets(xml: &str, coverage: &mut Coverage) -> Vec<String> {
    let mut reader = XmlReader::new(xml, &["Relationships"], coverage);
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

fn shared_strings(xml: &str, coverage: &mut Coverage) -> Vec<String> {
    let mut reader = XmlReader::new(xml, &["sst"], coverage);
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
            Ok(Event::CData(chunk)) if in_text => current.push_str(&chunk),
            Ok(Event::GeneralRef(entity)) if in_text => current.push_str(&resolve_entity(&entity)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    strings
}

/// String cells of a sheet in document order: shared (`t="s"`), inline and formula strings.
fn sheet_strings(xml: &str, shared: &[String], coverage: &mut Coverage) -> Vec<String> {
    let mut reader = XmlReader::new(xml, &["worksheet"], coverage);
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
                        "s" => {
                            let value = text
                                .trim()
                                .parse::<usize>()
                                .ok()
                                .and_then(|index| shared.get(index).cloned());
                            if value.is_none() {
                                *reader.coverage = (*reader.coverage).max(Coverage::ParseErrors);
                            }
                            value
                        }
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
            Ok(Event::CData(chunk)) if in_value => value.push_str(&chunk),
            Ok(Event::GeneralRef(entity)) if in_value => value.push_str(&resolve_entity(&entity)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    strings
}

/// A ZIP member as UTF-8, bounded by `PART_LIMIT`.
fn part(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
    required: bool,
    coverage: &mut Coverage,
) -> Option<String> {
    let file = match archive.by_name(name) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) if !required => return None,
        Err(_) => {
            *coverage = (*coverage).max(Coverage::ParseErrors);
            return None;
        }
    };
    if file.size() > PART_LIMIT as u64 {
        *coverage = (*coverage).max(Coverage::MatchLimit);
        return None;
    }
    let mut text = String::new();
    if file
        .take(PART_LIMIT as u64 + 1)
        .read_to_string(&mut text)
        .is_err()
    {
        *coverage = (*coverage).max(Coverage::ParseErrors);
        return None;
    }
    if text.len() > PART_LIMIT {
        *coverage = (*coverage).max(Coverage::MatchLimit);
        return None;
    }
    Some(text)
}

/// Resolve a package relationship relative to the part that owns it.
fn package_path(owner: &str, target: &str) -> Option<String> {
    if target.contains([':', '\\', '#', '?']) {
        return None;
    }
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        owner
            .rsplit_once('/')
            .map_or(Vec::new(), |(directory, _)| directory.split('/').collect())
    };
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(segment),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Account for malformed and unfinished XML in the same pass that extracts its text.
struct XmlReader<'source, 'coverage> {
    reader: Reader<&'source [u8]>,
    coverage: &'coverage mut Coverage,
    roots: &'static [&'static str],
    depth: usize,
    seen_root: bool,
}

impl<'source, 'coverage> XmlReader<'source, 'coverage> {
    fn new(
        xml: &'source str,
        roots: &'static [&'static str],
        coverage: &'coverage mut Coverage,
    ) -> Self {
        Self {
            reader: Reader::from_str(xml),
            coverage,
            roots,
            depth: 0,
            seen_root: false,
        }
    }

    fn read_event(&mut self) -> Result<Event<'source>, quick_xml::Error> {
        let event = self.reader.read_event();
        match &event {
            Ok(Event::Start(tag) | Event::Empty(tag)) => {
                if self.depth == 0 {
                    if self.seen_root
                        || !self
                            .roots
                            .iter()
                            .any(|root| tag.local_name().as_ref() == *root)
                    {
                        *self.coverage = (*self.coverage).max(Coverage::ParseErrors);
                    }
                    self.seen_root = true;
                }
                if tag.attributes().any(|attribute| attribute.is_err()) {
                    *self.coverage = (*self.coverage).max(Coverage::ParseErrors);
                }
                if matches!(&event, Ok(Event::Start(_))) {
                    self.depth += 1;
                }
            }
            Ok(Event::End(_)) => self.depth = self.depth.saturating_sub(1),
            Ok(Event::Eof) if self.depth != 0 || !self.seen_root => {
                *self.coverage = (*self.coverage).max(Coverage::ParseErrors);
            }
            Ok(Event::Text(text)) if self.depth == 0 && !text.trim().is_empty() => {
                *self.coverage = (*self.coverage).max(Coverage::ParseErrors);
            }
            Ok(Event::GeneralRef(entity)) if resolve_entity(entity).is_empty() => {
                *self.coverage = (*self.coverage).max(Coverage::NoGrammar);
            }
            Err(_) => *self.coverage = (*self.coverage).max(Coverage::ParseErrors),
            _ => {}
        }
        event
    }
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
        coverage: Coverage::Failed,
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
        assert!(
            facts
                .symbols
                .iter()
                .any(|symbol| symbol.terms.contains("max")),
            "{facts:?}"
        );
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
        let terms: Vec<(&str, u32, &str)> = facts
            .symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol.line, symbol.terms.as_str()))
            .collect();
        assert_eq!(
            terms,
            [
                ("Limits", 1, "max rows retry budget per job"),
                ("Owners", 2, "team ops")
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
        let terms: Vec<(&str, u32, &str)> = facts
            .symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol.line, symbol.terms.as_str()))
            .collect();
        assert_eq!(
            terms,
            [
                ("Intro", 1, "stor design note"),
                ("Limits", 2, "limit apply rows")
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

    fn section_terms(facts: &Facts) -> Vec<(String, u32, String)> {
        facts
            .symbols
            .iter()
            .map(|symbol| (symbol.name.clone(), symbol.line, symbol.terms.clone()))
            .collect()
    }

    #[test]
    fn audit_document_errors_never_prove_complete_coverage() {
        use crate::domain::facts::Coverage;
        for facts in [
            docx(&zip_of(&[(
                "word/document.xml",
                "<w:document><w:p><w:t>Paris",
            )])),
            xlsx(&zip_of(&[(
                "xl/workbook.xml",
                "<workbook><sheets><sheet name=\"A\" r:id=\"r1\"/></sheets></workbook>",
            )])),
            opendocument(&zip_of(&[("content.xml", "<office:text><text:p>Paris")])),
            rtf(br"{\rtf1 unclosed"),
            pdf(b"broken"),
        ] {
            assert_ne!(facts.coverage, Coverage::Complete, "{facts:?}");
        }
        let partial = docx(&zip_of(&[(
            "word/document.xml",
            "<w:document><w:p><w:t>Paris</w:t></w:p><w:p>",
        )]));
        assert!(
            partial
                .symbols
                .iter()
                .any(|symbol| symbol.terms.contains("paris"))
        );
        assert_ne!(partial.coverage, Coverage::Complete);
        let oversized = " ".repeat(PART_LIMIT + 1);
        assert_eq!(
            docx(&zip_of(&[("word/document.xml", &oversized)])).coverage,
            Coverage::MatchLimit
        );
        let cdata = docx(&zip_of(&[(
            "word/document.xml",
            "<w:document><w:p><w:r><w:t><![CDATA[Paris & London]]></w:t></w:r></w:p></w:document>",
        )]));
        assert_eq!(cdata.doc, "Paris & London");
        assert_eq!(cdata.coverage, Coverage::Complete);
    }

    #[test]
    fn audit_presentation_order_comes_from_relationships() {
        let slide = |title: &str| {
            format!(
                "<p:sld><p:sp><p:nvPr><p:ph type=\"title\"/></p:nvPr><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:sp></p:sld>"
            )
        };
        let first = slide("First");
        let second = slide("Second");
        let facts = pptx(&zip_of(&[
            (
                "ppt/presentation.xml",
                "<p:presentation><p:sldIdLst><p:sldId r:id=\"second\"/><p:sldId r:id=\"first\"/></p:sldIdLst></p:presentation>",
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                "<Relationships><Relationship Id=\"first\" Target=\"slides/slide1.xml\"/><Relationship Id=\"second\" Target=\"slides/slide2.xml\"/></Relationships>",
            ),
            ("ppt/slides/slide1.xml", &first),
            ("ppt/slides/slide2.xml", &second),
        ]));
        assert_eq!(
            section_terms(&facts)
                .iter()
                .map(|(name, line, _)| (name.as_str(), *line))
                .collect::<Vec<_>>(),
            [("Second", 1), ("First", 2)]
        );
    }

    #[test]
    fn audit_opendocument_explicit_spaces_keep_words_separate() {
        let facts = opendocument(&zip_of(&[(
            "content.xml",
            "<office:text><text:p>alpha<text:s text:c=\"3\"/>beta<text:tab/>gamma<text:line-break/>delta</text:p></office:text>",
        )]));
        assert_eq!(facts.doc, "alpha beta gamma delta");
        assert!(facts.symbols[0].terms.contains("alpha beta gamma delta"));
    }

    #[test]
    fn audit_rtf_unicode_fallback_is_group_scoped_and_counts_characters() {
        assert_eq!(
            rtf_lines(br"{\rtf1\uc2 \u945abX{\uc0 \u946}\u947\'3f\'3fY}").0,
            ["αXβγY"]
        );
        assert_eq!(rtf_lines(br"{\rtf1\uc1 \u-10179?\u-8704?}").0, ["😀"]);
        assert_eq!(
            rtf_lines(br"{\rtf1\ansi\ansicpg1252 \u8364x \'80}").0,
            ["€ €"]
        );
        assert_eq!(
            rtf_lines(br"{\rtf1\ansi\ansicpg932 \'93\'8c\'8b\'9e}").0,
            ["東京"]
        );
    }

    #[test]
    fn audit_legacy_binary_noise_is_not_document_text() {
        let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0, 0, 0, 0];
        bytes.extend(b"Not an actual document stream with Paris");
        let facts = legacy_office("broken.doc", &bytes);
        assert!(facts.symbols.is_empty(), "{facts:?}");
        assert_ne!(facts.coverage, crate::domain::facts::Coverage::Complete);
    }

    #[test]
    fn pptx_slides_titles_text_and_notes() {
        let slide = r#"<p:sld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Quarterly plan</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:txBody><a:p><a:r><a:t>Budget for </a:t></a:r><a:r><a:t>wombats</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:sld>"#;
        let untitled = r#"<p:sld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Second page body</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:sld>"#;
        let relations = r#"<Relationships><Relationship Id="rId1" Target="../notesSlides/notesSlide7.xml"/></Relationships>"#;
        let notes = r#"<p:notes><p:sp><p:txBody><a:p><a:r><a:t>Speaker reminder</a:t></a:r></a:p></p:txBody></p:sp></p:notes>"#;
        let facts = pptx(&zip_of(&[
            (
                "ppt/presentation.xml",
                "<p:presentation><p:sldIdLst><p:sldId r:id=\"one\"/><p:sldId r:id=\"two\"/></p:sldIdLst></p:presentation>",
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                "<Relationships><Relationship Id=\"one\" Target=\"slides/slide1.xml\"/><Relationship Id=\"two\" Target=\"slides/slide2.xml\"/></Relationships>",
            ),
            ("ppt/slides/slide2.xml", untitled),
            ("ppt/slides/slide1.xml", slide),
            ("ppt/slides/_rels/slide1.xml.rels", relations),
            ("ppt/notesSlides/notesSlide7.xml", notes),
        ]));
        let terms = section_terms(&facts);
        assert_eq!(terms[0].0, "Quarterly plan");
        assert_eq!(terms[0].1, 1);
        assert!(terms[0].2.contains("wombat"), "{terms:?}");
        assert!(terms[0].2.contains("speaker"), "{terms:?}");
        assert_eq!((terms[1].0.as_str(), terms[1].1), ("Slide 2", 2));
        assert_eq!(facts.lines, 2);
        assert!(pptx(b"no zip").doc.starts_with("! unreadable PPTX"));
    }

    #[test]
    fn opendocument_text_and_spreadsheet() {
        let text = r#"<office:text><text:h text:outline-level="1">Chapter</text:h><text:p>Opening <text:span>wombat</text:span> line</text:p><text:h text:outline-level="2">Part</text:h><text:p>Second</text:p></office:text>"#;
        let facts = opendocument(&zip_of(&[("content.xml", text)]));
        let terms = section_terms(&facts);
        assert_eq!((terms[0].0.as_str(), terms[0].1), ("Chapter", 1));
        assert!(terms[0].2.contains("wombat"), "{terms:?}");
        assert_eq!((terms[1].0.as_str(), terms[1].1), ("Part", 3));
        assert_eq!(facts.symbols[1].parent, Some(0));
        let sheets = r#"<office:spreadsheet><table:table table:name="Costs"><table:table-row><table:table-cell><text:p>rent</text:p></table:table-cell></table:table-row></table:table><table:table table:name="Staff"><table:table-row><table:table-cell><text:p>wombat keeper</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet>"#;
        let facts = opendocument(&zip_of(&[("content.xml", sheets)]));
        let terms = section_terms(&facts);
        assert_eq!((terms[0].0.as_str(), terms[0].1), ("Costs", 1));
        assert_eq!((terms[1].0.as_str(), terms[1].1), ("Staff", 2));
        assert!(terms[1].2.contains("wombat"), "{terms:?}");
        assert_eq!(facts.lines, 2);
    }

    #[test]
    fn rtf_text_skips_tables_and_decodes_escapes() {
        let source = br#"{\rtf1\ansi{\fonttbl{\f0 Hidden Font;}}{\*\generator Hidden;}\f0 Gr\'fc\'df wombat\par Next \u8364? line\par}"#;
        let lines = rtf_lines(source).0;
        assert_eq!(lines[0], "Grüß wombat");
        assert_eq!(lines[1], "Next € line");
        assert_eq!(
            rtf_lines(b"{\\rtf1 First\\\nSecond}").0,
            ["First", "Second"]
        );
        assert!(!lines.join(" ").contains("Hidden"), "{lines:?}");
        let facts = rtf(source);
        assert_eq!(facts.lines, 3);
        assert!(facts.symbols[0].terms.contains("wombat"), "{facts:?}");
        assert!(rtf(b"plain").doc.starts_with("! unreadable RTF"));
    }

    #[test]
    fn legacy_office_reads_real_short_documents_and_preserves_locators() {
        // Exported by LibreOffice 26.8 from original text/sheet/slide documents, not binary runs.
        let doc = legacy_office(
            "short.doc",
            include_bytes!("../../tests/fixtures/documents/short.doc"),
        );
        assert_eq!(doc.coverage, Coverage::Complete, "{doc:?}");
        assert_eq!(doc.doc, "Paris");
        assert!(doc.symbols.iter().any(|symbol| symbol.terms == "paris"));
        for (path, bytes) in [
            (
                "workbook.xls",
                include_bytes!("../../tests/fixtures/documents/workbook.xls").as_slice(),
            ),
            (
                "slides.ppt",
                include_bytes!("../../tests/fixtures/documents/slides.ppt").as_slice(),
            ),
        ] {
            let facts = legacy_office(path, bytes);
            assert_eq!(facts.coverage, Coverage::Complete, "{path}: {facts:?}");
            assert_eq!(facts.lines, 2, "{path}: {facts:?}");
            assert!(
                facts
                    .symbols
                    .iter()
                    .any(|symbol| symbol.line == 1 && symbol.terms.contains("paris")),
                "{path}: {facts:?}"
            );
            assert!(
                facts
                    .symbols
                    .iter()
                    .any(|symbol| symbol.line == 2 && symbol.terms.contains("quokka")),
                "{path}: {facts:?}"
            );
        }
        assert!(legacy_office("broken.doc", b"plain").symbols.is_empty());
    }
}
