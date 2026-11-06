//! Source-backed Go embed directives, before filesystem pattern resolution.

use tree_sitter::Node;

use super::engine::Parsed;
use crate::domain::facts::{GoEmbed, SymKind};

pub(super) fn collect(root: Node<'_>, source: &[u8], parsed: &mut Parsed) {
    if !imports_embed(root, source) {
        return;
    }
    let mut cursor = root.walk();
    for declaration in root.named_children(&mut cursor) {
        if declaration.kind() != "var_declaration" {
            continue;
        }
        let mut child_cursor = declaration.walk();
        for child in declaration.named_children(&mut child_cursor) {
            if child.kind() == "var_spec" {
                collect_spec(child, declaration, source, parsed);
            } else if child.kind() == "var_spec_list" {
                let mut list_cursor = child.walk();
                for spec in child.named_children(&mut list_cursor) {
                    if spec.kind() == "var_spec" {
                        collect_spec(spec, spec, source, parsed);
                    }
                }
            }
        }
    }
    parsed
        .facts
        .go_embeds
        .sort_by_key(|embed| (embed.line, embed.symbol));
}

fn imports_embed(root: Node<'_>, source: &[u8]) -> bool {
    let mut cursor = root.walk();
    for declaration in root.named_children(&mut cursor) {
        if declaration.kind() != "import_declaration" {
            continue;
        }
        let mut pending = vec![declaration];
        while let Some(node) = pending.pop() {
            if node.kind() == "import_spec"
                && let Some(path) = node.child_by_field_name("path")
                && let Ok(text) = path.utf8_text(source)
                && literal(text).as_deref() == Some(b"embed")
            {
                return true;
            }
            let mut children = node.walk();
            pending.extend(node.named_children(&mut children));
        }
    }
    false
}

fn collect_spec(spec: Node<'_>, anchor: Node<'_>, source: &[u8], parsed: &mut Parsed) {
    if spec.child_by_field_name("type").is_none() || spec.child_by_field_name("value").is_some() {
        return;
    }
    let mut cursor = spec.walk();
    let mut names = spec.children_by_field_name("name", &mut cursor);
    let Some(name) = names.next() else { return };
    if names.next().is_some() {
        return;
    }
    let Some(symbol) = parsed
        .spans
        .iter()
        .enumerate()
        .find_map(|(index, &(start, end))| {
            (start == spec.start_byte()
                && end == spec.end_byte()
                && parsed.facts.symbols.get(index).is_some_and(|item| {
                    item.kind() == SymKind::Var
                        && name.utf8_text(source).is_ok_and(|name| *item.name == *name)
                }))
            .then(|| u32::try_from(index).ok())
            .flatten()
        })
    else {
        return;
    };
    let mut next_start = anchor.start_byte();
    let mut previous = anchor.prev_named_sibling();
    while let Some(comment) = previous {
        if comment.kind() != "comment"
            || !source[comment.end_byte()..next_start]
                .iter()
                .all(u8::is_ascii_whitespace)
        {
            break;
        }
        let Ok(text) = comment.utf8_text(source) else {
            break;
        };
        if !text.starts_with("//") {
            break;
        }
        let before = &source[..comment.start_byte()];
        let line_start = before
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |p| p + 1);
        if before[line_start..].iter().all(u8::is_ascii_whitespace)
            && let Some(arguments) = text.strip_prefix("//go:embed ")
            && let Some(patterns) = patterns(arguments)
        {
            let line = u32::try_from(comment.start_position().row + 1).unwrap_or(u32::MAX);
            parsed
                .facts
                .go_embeds
                .extend(patterns.into_iter().map(|pattern| GoEmbed {
                    symbol,
                    line,
                    pattern,
                }));
        }
        next_start = comment.start_byte();
        previous = comment.prev_named_sibling();
    }
}

fn patterns(mut text: &str) -> Option<Vec<Vec<u8>>> {
    let mut result = Vec::new();
    loop {
        text = text.trim_start_matches(char::is_whitespace);
        if text.is_empty() {
            return (!result.is_empty()).then_some(result);
        }
        let bytes = text.as_bytes();
        let end = match bytes[0] {
            b'`' => text[1..].find('`')? + 2,
            b'"' => {
                let mut index = 1;
                loop {
                    match *bytes.get(index)? {
                        b'\\' => index += 2,
                        b'"' => break index + 1,
                        _ => index += 1,
                    }
                }
            }
            _ => text.find(char::is_whitespace).unwrap_or(text.len()),
        };
        let (argument, rest) = text.split_at(end);
        if rest.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
            return None;
        }
        result.push(if matches!(bytes[0], b'`' | b'"') {
            literal(argument)?
        } else {
            argument.as_bytes().to_vec()
        });
        text = rest;
    }
}

fn literal(text: &str) -> Option<Vec<u8>> {
    if let Some(raw) = text
        .strip_prefix('`')
        .and_then(|text| text.strip_suffix('`'))
    {
        return Some(raw.bytes().filter(|&byte| byte != b'\r').collect());
    }
    let inner = text.strip_prefix('"')?.strip_suffix('"')?.as_bytes();
    let mut output = Vec::with_capacity(inner.len());
    let mut index = 0;
    while index < inner.len() {
        let byte = inner[index];
        index += 1;
        if byte != b'\\' {
            if matches!(byte, b'\n' | b'\r' | b'"') {
                return None;
            }
            output.push(byte);
            continue;
        }
        let escape = *inner.get(index)?;
        index += 1;
        match escape {
            b'a' => output.push(7),
            b'b' => output.push(8),
            b'f' => output.push(12),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'v' => output.push(11),
            b'\\' => output.push(b'\\'),
            b'"' => output.push(b'"'),
            b'0'..=b'7' => {
                let digits = inner.get(index..index + 2)?;
                if !digits.iter().all(|byte| (b'0'..=b'7').contains(byte)) {
                    return None;
                }
                let value = u32::from(escape - b'0') * 64
                    + u32::from(digits[0] - b'0') * 8
                    + u32::from(digits[1] - b'0');
                output.push(u8::try_from(value).ok()?);
                index += 2;
            }
            b'x' | b'u' | b'U' => {
                let count = match escape {
                    b'x' => 2,
                    b'u' => 4,
                    _ => 8,
                };
                let digits = std::str::from_utf8(inner.get(index..index + count)?).ok()?;
                if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return None;
                }
                let value = u32::from_str_radix(digits, 16).ok()?;
                index += count;
                if escape == b'x' {
                    output.push(u8::try_from(value).ok()?);
                } else {
                    let mut buf = [0; 4];
                    output
                        .extend_from_slice(char::from_u32(value)?.encode_utf8(&mut buf).as_bytes());
                }
            }
            _ => return None,
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{corpus::FileRecord, filetype::FileType, role::Role};

    fn extract(source: &str) -> crate::domain::facts::Facts {
        let record = FileRecord {
            path: "main.go".to_owned(),
            size: source.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::Go),
        };
        crate::extract::extract(&record, source.as_bytes())
    }

    #[test]
    fn go_directives_follow_package_variable_syntax() {
        let source = "package p\nimport _ \"embed\"\n//go:embed a.txt \"space name.txt\" `raw name`\n// ordinary comment\nvar Content string\nvar (\n //go:embed group/*.txt\n Group []byte\n)\n//go:embed bad\nvar One, Two string\n//go:embed bad\nvar Initialized string = \"\"\nfunc f() {\n //go:embed bad\n var local string\n}\n";
        let facts = extract(source);
        let actual: Vec<_> = facts
            .go_embeds
            .iter()
            .map(|embed| {
                (
                    facts.symbols[embed.symbol as usize].name.as_ref(),
                    embed.line,
                    embed.pattern.as_slice(),
                )
            })
            .collect();
        assert_eq!(
            actual,
            [
                ("Content", 3, b"a.txt".as_slice()),
                ("Content", 3, b"space name.txt".as_slice()),
                ("Content", 3, b"raw name".as_slice()),
                ("Group", 7, b"group/*.txt".as_slice()),
            ]
        );
        assert!(
            extract("package p\n//go:embed x\nvar X string\n")
                .go_embeds
                .is_empty()
        );
    }

    #[test]
    fn go_patterns_decode_escapes_and_reject_bad_quotes() {
        assert_eq!(
            patterns(r#"plain "space name" `raw name` "\xff\141\u00e9""#),
            Some(vec![
                b"plain".to_vec(),
                b"space name".to_vec(),
                b"raw name".to_vec(),
                vec![255, b'a', 0xc3, 0xa9]
            ])
        );
        for invalid in [
            "",
            "\"unterminated",
            "`unterminated",
            "\"x\"tail",
            r#""\400""#,
            r#""\ud800""#,
            r#""\x+1""#,
        ] {
            assert_eq!(patterns(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn go_blank_variable_and_escaped_import_are_recognized() {
        let facts =
            extract("package p\nimport _ \"em\\x62ed\"\n//go:embed data.txt\nvar _ string\n");
        assert_eq!(facts.go_embeds.len(), 1);
        assert_eq!(facts.go_embeds[0].pattern, b"data.txt");
        assert_eq!(facts.symbols[facts.go_embeds[0].symbol as usize].name, "_");
        assert!(
            extract("package p\nimport _ \"embed\"\n//go:embed\tdata.txt\nvar X string\n")
                .go_embeds
                .is_empty()
        );
    }

    #[test]
    fn go_real_package_fixture_preserves_directives_and_order() {
        let facts = extract(include_str!("../../tests/fixtures/go-embed/main.go"));
        let actual: Vec<_> = facts
            .go_embeds
            .iter()
            .map(|embed| {
                (
                    facts.symbols[embed.symbol as usize].name.as_ref(),
                    embed.line,
                    embed.pattern.as_slice(),
                )
            })
            .collect();
        assert_eq!(
            actual,
            [
                ("Content", 5, b"data.txt".as_slice()),
                ("Content", 5, b"dir/space name.txt".as_slice()),
                ("Content", 5, b"dir/raw.txt".as_slice()),
                ("Group", 9, b"dir/sub/*".as_slice()),
            ]
        );
    }

    #[test]
    fn go_directives_reject_unrelated_comments_and_preserve_repeated_lines() {
        let source = "package p\nimport \"embed\"\nvar A int //go:embed trailing\nvar Tail string\n//go:embed blocked\n/* block comment */\nvar Blocked string\n//go:embed \"\\q\"\nvar Bad string\n//go:embed first\n\n// ordinary\n//go:embed second third\nvar Repeat embed.FS\n";
        let facts = extract(source);
        let actual: Vec<_> = facts
            .go_embeds
            .iter()
            .map(|embed| {
                (
                    facts.symbols[embed.symbol as usize].name.as_ref(),
                    embed.line,
                    embed.pattern.as_slice(),
                )
            })
            .collect();
        assert_eq!(
            actual,
            [
                ("Repeat", 10, b"first".as_slice()),
                ("Repeat", 13, b"second".as_slice()),
                ("Repeat", 13, b"third".as_slice()),
            ]
        );
    }
}
