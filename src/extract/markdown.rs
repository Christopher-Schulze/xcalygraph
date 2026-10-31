//! Markdown and other prose documents: headings as symbols, relative links as path references,
//! backticked identifiers and paths as mentions (`docs/spec.md` sections 5 and 6).

use crate::domain::facts::{self, Facts, Ref, RefKind, SymKind, Symbol};

pub fn extract(text: &str) -> Facts {
    let mut facts = Facts::default();
    let mut fence: Option<&str> = None;
    // Open headings by level, as symbol indices.
    let mut open: Vec<(usize, u32)> = Vec::new();
    let mut first_paragraph: Vec<&str> = Vec::new();
    let mut paragraph_done = false;
    let line_count = text.lines().count() as u32;
    for (index, line) in text.lines().enumerate() {
        let number = index as u32 + 1;
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            continue;
        }
        if let Some(level) = heading_level(trimmed) {
            let title = trimmed[level..].trim().trim_end_matches('#').trim();
            if title.is_empty() {
                continue;
            }
            while open
                .last()
                .is_some_and(|&(open_level, _)| open_level >= level)
            {
                if let Some((_, closed)) = open.pop() {
                    facts.symbols[closed as usize].end_line = number - 1;
                }
            }
            let parent = open.last().map(|&(_, symbol)| symbol);
            let id = facts.symbols.len() as u32;
            facts.symbols.push(Symbol {
                kind: Some(SymKind::Heading),
                name: title.to_owned(),
                parent,
                line: number,
                end_line: line_count,
                exported: true,
                ..Symbol::default()
            });
            open.push((level, id));
            paragraph_done |= !first_paragraph.is_empty();
            continue;
        }
        if !paragraph_done {
            if trimmed.is_empty() {
                paragraph_done = !first_paragraph.is_empty();
            } else if !trimmed.starts_with(['<', '!', '[', '|', '-', '*', '>']) {
                first_paragraph.push(trimmed);
            }
        }
        let scope = open.last().map(|&(_, symbol)| symbol);
        for target in links(line) {
            facts.refs.push(Ref {
                kind: RefKind::Path,
                name: target,
                qualifier: None,
                scope,
                line: number,
            });
        }
        for mention in mentions(line) {
            facts.refs.push(Ref {
                kind: RefKind::Mention,
                name: mention,
                qualifier: None,
                scope,
                line: number,
            });
        }
    }
    facts.doc = facts::clip(&facts::normalize_space(&first_paragraph.join(" ")), 400);
    facts
}

fn heading_level(line: &str) -> Option<usize> {
    let level = line.bytes().take_while(|&byte| byte == b'#').count();
    ((1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ')).then_some(level)
}

/// Relative targets of `[text](target)` and `![alt](target)`; URLs, anchors and mail links
/// are skipped, `#fragment` and query parts are cut.
fn links(line: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else { break };
        let raw = rest[..end].split_whitespace().next().unwrap_or_default();
        rest = &rest[end..];
        let target = raw
            .trim_matches(['<', '>'])
            .split(['#', '?'])
            .next()
            .unwrap_or_default();
        let external =
            target.contains("://") || target.starts_with("mailto:") || target.starts_with("tel:");
        if !target.is_empty() && !external {
            targets.push(target.to_owned());
        }
    }
    targets
}

/// Backticked identifiers (`Store`, `store.Get`, `crate::x`) and relative paths (`src/a.rs`).
fn mentions(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut parts = line.split('`');
    parts.next();
    while let (Some(inside), _) = (parts.next(), parts.next()) {
        let inside = inside.trim().trim_end_matches("()");
        let identifier_like = !inside.is_empty()
            && inside.len() <= 120
            && inside
                .chars()
                .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.' | ':' | '/' | '-'))
            && inside.chars().any(char::is_alphabetic)
            && !inside.starts_with('-');
        if identifier_like && !found.iter().any(|existing| existing == inside) {
            found.push(inside.to_owned());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_links_and_mentions() {
        let text = "# Guide\n\nIntro text about `Store`.\nSee [api](docs/api.md#x) and [site](https://x.io).\n\n## Usage\n\nCall `store.Get()` from `src/store.go`, not `-v` or `a b`.\n\n```go\n# not a heading [x](y.md)\n```\n### Deep\n![logo](assets/logo.png)\n# Next\n";
        let facts = extract(text);
        let headings: Vec<(&str, u32, u32, Option<u32>)> = facts
            .symbols
            .iter()
            .map(|symbol| {
                (
                    symbol.name.as_str(),
                    symbol.line,
                    symbol.end_line,
                    symbol.parent,
                )
            })
            .collect();
        assert_eq!(
            headings,
            [
                ("Guide", 1, 14, None),
                ("Usage", 6, 14, Some(0)),
                ("Deep", 13, 14, Some(1)),
                ("Next", 15, 15, None)
            ]
        );
        let refs: Vec<(RefKind, &str, Option<u32>)> = facts
            .refs
            .iter()
            .map(|r| (r.kind, r.name.as_str(), r.scope))
            .collect();
        assert_eq!(
            refs,
            [
                (RefKind::Mention, "Store", Some(0)),
                (RefKind::Path, "docs/api.md", Some(0)),
                (RefKind::Mention, "store.Get", Some(1)),
                (RefKind::Mention, "src/store.go", Some(1)),
                (RefKind::Path, "assets/logo.png", Some(2)),
            ]
        );
        assert_eq!(
            facts.doc,
            "Intro text about `Store`. See [api](docs/api.md#x) and [site](https://x.io)."
        );
    }
}
