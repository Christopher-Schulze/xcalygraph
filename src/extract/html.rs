//! HTML outline and inline JavaScript facts, with source locations kept in document coordinates.

use std::ops::Range;

use crate::domain::corpus::FileRecord;
use crate::domain::facts::{Facts, SymKind};
use crate::domain::filetype::FileType;

use super::{engine, lang, markup, prose};

pub(super) fn extract(
    record: &FileRecord,
    content: &[u8],
    capture_identities: bool,
) -> engine::Parsed {
    let text = String::from_utf8_lossy(content);
    let mut parsed = engine::Parsed {
        facts: prose::extract(FileType::Html, &text),
        ..engine::Parsed::default()
    };
    if capture_identities {
        parsed.identities = parsed
            .facts
            .symbols
            .iter()
            .map(|symbol| (symbol.kind() == SymKind::Heading).then(|| symbol.name.to_string()))
            .collect();
    }
    if let Some((pack, language, query)) = lang::lookup(FileType::JavaScript, &record.path) {
        for range in script_blocks(&text) {
            let script_source = &text.as_bytes()[range.clone()];
            let mut script = engine::extract(
                &language,
                query,
                &pack.hooks,
                &record.path,
                script_source,
                capture_identities,
            );
            super::terms::add(script_source, &mut script.facts, &script.spans);
            merge_script(
                &mut parsed,
                script,
                line_offset(&text, range.start),
                u32::try_from(range.start).unwrap_or(crate::domain::facts::NO_BYTE),
                capture_identities,
            );
        }
    }
    if text.len() != content.len() {
        for reference in &mut parsed.facts.refs {
            reference.byte = crate::domain::facts::NO_BYTE;
        }
        for binding in &mut parsed.facts.bindings {
            // Lossy text moved the bytes: scope and value are unknown, the name still shadows.
            binding.value_byte = crate::domain::facts::NO_BYTE;
            binding.visibility = None;
            binding.type_name = crate::domain::facts::PATTERN_BINDING.into();
            binding.declared = false;
        }
    }
    parsed.facts.lines = super::count_lines(content);
    parsed
}

fn script_blocks(text: &str) -> Vec<Range<usize>> {
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut blocks = Vec::new();
    let mut position = 0;
    let mut template_depth = 0usize;
    while position < bytes.len() {
        if bytes[position] == b'<' {
            if text[position..].starts_with("<!--") {
                position = text[position + 4..]
                    .find("-->")
                    .map_or(text.len(), |offset| position + 4 + offset + 3);
                continue;
            }
            if text[position..].starts_with("<![CDATA[") {
                position = text[position + 9..]
                    .find("]]>")
                    .map_or(text.len(), |offset| position + 9 + offset + 3);
                continue;
            }
            if matches!(bytes.get(position + 1), Some(b'!' | b'?')) {
                let (end, terminated) = markup::markup_end(bytes, position);
                if !terminated {
                    break;
                }
                position = end;
                continue;
            }
            if let Some(tag) = markup::markup_tag(text, position) {
                if !tag.terminated {
                    break;
                }
                if tag.name.eq_ignore_ascii_case("template") {
                    if tag.closing {
                        template_depth = template_depth.saturating_sub(1);
                    } else {
                        template_depth = template_depth.saturating_add(1);
                    }
                }
                if !tag.closing && tag.name.eq_ignore_ascii_case("plaintext") {
                    break;
                }
                if !tag.closing && is_raw_text_element(tag.name) {
                    let close = markup::raw_element_close(text, &lower, tag.end, tag.name)
                        .filter(|close| close.terminated);
                    if tag.name.eq_ignore_ascii_case("script")
                        && template_depth == 0
                        && let Some(close) = close.as_ref()
                        && supports_inline_javascript(&script_attributes(text, &tag))
                    {
                        blocks.push(tag.end..close.start);
                    }
                    position = close.map_or(text.len(), |close| close.end);
                    continue;
                }
                position = tag.end;
                continue;
            }
        }
        position += 1;
    }
    blocks
}

fn is_raw_text_element(name: &str) -> bool {
    [
        "script", "style", "title", "textarea", "iframe", "noembed", "noframes", "noscript", "xmp",
    ]
    .iter()
    .any(|element| name.eq_ignore_ascii_case(element))
}

#[derive(Default)]
struct ScriptAttributes<'a> {
    type_value: Option<&'a str>,
    language: Option<&'a str>,
    has_src: bool,
    nomodule: bool,
}

fn script_attributes<'a>(text: &'a str, tag: &markup::MarkupTag<'_>) -> ScriptAttributes<'a> {
    let bytes = text.as_bytes();
    let mut position = tag.start + 1 + tag.name.len();
    let end = tag.end - 1;
    let mut attributes = ScriptAttributes::default();
    while position < end {
        skip_space(bytes, &mut position, end);
        if position == end {
            break;
        }
        if bytes[position] == b'/' {
            position += 1;
            continue;
        }
        let name_start = position;
        while position < end && !is_attribute_name_end(bytes[position]) {
            position += 1;
        }
        if name_start == position {
            position += 1;
            continue;
        }
        let name = &bytes[name_start..position];
        skip_space(bytes, &mut position, end);
        let value = attribute_value(text, bytes, &mut position, end);
        if name.eq_ignore_ascii_case(b"type") && attributes.type_value.is_none() {
            attributes.type_value = Some(value);
        } else if name.eq_ignore_ascii_case(b"language") && attributes.language.is_none() {
            attributes.language = Some(value);
        } else if name.eq_ignore_ascii_case(b"src") {
            attributes.has_src = true;
        } else if name.eq_ignore_ascii_case(b"nomodule") {
            attributes.nomodule = true;
        }
    }
    attributes
}

fn is_attribute_name_end(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'/' | b'=' | b'>')
}

fn skip_space(bytes: &[u8], position: &mut usize, end: usize) {
    while *position < end && bytes[*position].is_ascii_whitespace() {
        *position += 1;
    }
}

fn attribute_value<'a>(text: &'a str, bytes: &[u8], position: &mut usize, end: usize) -> &'a str {
    if *position == end || bytes[*position] != b'=' {
        return "";
    }
    *position += 1;
    skip_space(bytes, position, end);
    if *position == end {
        return "";
    }
    let quote = matches!(bytes[*position], b'\'' | b'"').then_some(bytes[*position]);
    if quote.is_some() {
        *position += 1;
    }
    let start = *position;
    while *position < end
        && if let Some(quote) = quote {
            bytes[*position] != quote
        } else {
            !bytes[*position].is_ascii_whitespace() && bytes[*position] != b'>'
        }
    {
        *position += 1;
    }
    let value = text.get(start..*position).unwrap_or_default();
    if quote.is_some() && *position < end {
        *position += 1;
    }
    value
}

fn supports_inline_javascript(attributes: &ScriptAttributes<'_>) -> bool {
    if attributes.has_src {
        return false;
    }
    let script_type = match attributes.type_value {
        Some("") => Some(ScriptType::Classic),
        Some(value) if value.eq_ignore_ascii_case("module") => Some(ScriptType::Module),
        Some(value) if is_javascript_mime_type(value) => Some(ScriptType::Classic),
        Some(_) => None,
        None => match attributes.language {
            None | Some("") => Some(ScriptType::Classic),
            Some(language) if is_javascript_mime_type(&format!("text/{language}")) => {
                Some(ScriptType::Classic)
            }
            Some(_) => None,
        },
    };
    script_type.is_some_and(|kind| kind == ScriptType::Module || !attributes.nomodule)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScriptType {
    Classic,
    Module,
}

fn is_javascript_mime_type(value: &str) -> bool {
    const TYPES: [&str; 16] = [
        "application/ecmascript",
        "application/javascript",
        "application/x-ecmascript",
        "application/x-javascript",
        "text/ecmascript",
        "text/javascript",
        "text/javascript1.0",
        "text/javascript1.1",
        "text/javascript1.2",
        "text/javascript1.3",
        "text/javascript1.4",
        "text/javascript1.5",
        "text/jscript",
        "text/livescript",
        "text/x-ecmascript",
        "text/x-javascript",
    ];
    let essence = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim_matches(|character: char| character.is_ascii_whitespace());
    TYPES
        .iter()
        .any(|mime_type| essence.eq_ignore_ascii_case(mime_type))
}

fn merge_script(
    parsed: &mut engine::Parsed,
    mut script: engine::Parsed,
    line_offset: u32,
    byte_offset: u32,
    capture_identities: bool,
) {
    let symbol_count = script.facts.symbols.len();
    if capture_identities {
        script.identities.resize(symbol_count, None);
    }
    merge_facts(
        &mut parsed.facts,
        &mut script.facts,
        line_offset,
        byte_offset,
    );
    parsed.incomplete |= script.incomplete;
    if capture_identities {
        parsed.identities.append(&mut script.identities);
    }
}

fn merge_facts(document: &mut Facts, script: &mut Facts, line_offset: u32, byte_offset: u32) {
    document.coverage = document.coverage.max(script.coverage);
    let symbol_offset = u32::try_from(document.symbols.len()).unwrap_or(u32::MAX);
    for symbol in &mut script.symbols {
        symbol.line = symbol.line.saturating_add(line_offset);
        symbol.end_line = symbol.end_line.saturating_add(line_offset);
        symbol.parent = offset_index(symbol.parent, symbol_offset);
    }
    for import in &mut script.imports {
        import.line = import.line.saturating_add(line_offset);
    }
    for reference in &mut script.refs {
        reference.line = reference.line.saturating_add(line_offset);
        reference.byte = offset_byte(reference.byte, byte_offset);
        reference.scope = offset_index(reference.scope, symbol_offset);
    }
    for binding in &mut script.bindings {
        binding.line = binding.line.saturating_add(line_offset);
        binding.value_byte = offset_byte(binding.value_byte, byte_offset);
        binding.visibility = binding.visibility.take().and_then(|range| {
            Some(range.start.checked_add(byte_offset)?..range.end.checked_add(byte_offset)?)
        });
        binding.scope = offset_index(binding.scope, symbol_offset);
    }
    for env in &mut script.envs {
        env.line = env.line.saturating_add(line_offset);
        env.scope = offset_index(env.scope, symbol_offset);
    }
    for fields in &mut script.fields {
        fields.owner = fields.owner.saturating_add(symbol_offset);
    }
    document.symbols.append(&mut script.symbols);
    document.imports.append(&mut script.imports);
    document.refs.append(&mut script.refs);
    document.bindings.append(&mut script.bindings);
    document.envs.append(&mut script.envs);
    document.fields.append(&mut script.fields);
    document.entry |= script.entry;
}

fn offset_index(index: Option<u32>, offset: u32) -> Option<u32> {
    index.map(|index| index.saturating_add(offset))
}

fn offset_byte(byte: u32, offset: u32) -> u32 {
    byte.checked_add(offset)
        .unwrap_or(crate::domain::facts::NO_BYTE)
}

fn line_offset(text: &str, byte_offset: usize) -> u32 {
    u32::try_from(
        text.as_bytes()[..byte_offset]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count(),
    )
    .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::domain::corpus::FileRecord;
    use crate::domain::filetype::FileType;
    use crate::domain::role::Role;

    #[test]
    fn inline_script_facts_keep_document_lines() {
        let content = concat!(
            "<html>\n",
            "<body>\n",
            "<script>\n",
            "function run() {\n",
            "  const client = new Client();\n",
            "  return process.env.API_KEY;\n",
            "}\n",
            "</script>\n",
            "</body></html>\n",
        );
        let record = FileRecord {
            path: "index.html".to_owned(),
            size: content.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Doc,
            file_type: Some(FileType::Html),
        };
        let facts = extract(&record, content.as_bytes(), false).facts;
        let run = facts.symbols.iter().find(|symbol| *symbol.name == *"run");
        assert_eq!(run.map(|symbol| symbol.line), Some(4));
        let binding = facts
            .bindings
            .iter()
            .find(|binding| *binding.name == *"client");
        assert_eq!(binding.map(|binding| binding.line), Some(5));
        assert_eq!(
            binding.map(|binding| binding.value_byte as usize),
            content.find("client ="),
            "inline binding bytes must address the complete document"
        );
        let constructor = facts
            .refs
            .iter()
            .find(|reference| reference.name == "Client");
        assert_eq!(
            constructor.map(|reference| reference.byte as usize),
            content.find("Client()"),
            "inline reference bytes must address the complete document"
        );
        let env = facts.envs.iter().find(|env| env.name == "API_KEY");
        assert_eq!(env.map(|env| env.line), Some(6));
        let invalid = [b"\xff".as_slice(), content.as_bytes()].concat();
        let invalid = extract(&record, &invalid, false).facts;
        assert!(!invalid.refs.is_empty() && !invalid.bindings.is_empty());
        assert!(
            invalid
                .refs
                .iter()
                .all(|reference| reference.byte == crate::domain::facts::NO_BYTE)
        );
        assert!(invalid.bindings.iter().all(|binding| {
            binding.value_byte == crate::domain::facts::NO_BYTE && binding.visibility.is_none()
        }));
    }
}
