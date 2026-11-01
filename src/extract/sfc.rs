//! Single-file components (Vue, Svelte, Astro): script blocks and Astro frontmatter are parsed
//! as TypeScript or JavaScript in place (other bytes blanked, so lines stay exact); component
//! tags of the template count as calls; the file itself is a component symbol named after its
//! stem (`docs/spec.md` section 3, embedded code). Svelte markup expressions (`{format(x)}`,
//! `onclick={save}`, `{#if ready}`) are parsed in place as statements and `use:enhance`
//! directives count as values, all scoped to the component, so script functions used only by
//! the markup are not dead; `$store` reads name the store.

use super::lang;
use crate::domain::corpus::FileRecord;
use crate::domain::facts::{Facts, Ref, RefKind, SymKind, Symbol};
use crate::domain::filetype::FileType;
use crate::extract::engine;

pub fn extract(record: &FileRecord, content: &[u8]) -> (Facts, crate::extract::engine::Spans) {
    let text = String::from_utf8_lossy(content);
    let (scripts, typescript) = script_ranges(&text, record.file_type == Some(FileType::Astro));
    let mut blanked: Vec<u8> = text
        .bytes()
        .map(|byte| if byte == b'\n' { b'\n' } else { b' ' })
        .collect();
    for range in &scripts {
        blanked[range.clone()].copy_from_slice(&text.as_bytes()[range.clone()]);
    }
    let svelte = record.file_type == Some(FileType::Svelte);
    let mut skipped = scripts.clone();
    if svelte {
        skipped.extend(element_ranges(&text, "<style", "</style>"));
        skipped.extend(element_ranges(&text, "<!--", "-->"));
        skipped.sort_by_key(|range| range.start);
        svelte_expressions(&text, &skipped, &mut blanked);
    }
    let (file_type, path) = if typescript {
        (FileType::TypeScript, format!("{}.ts", record.path))
    } else {
        (FileType::JavaScript, format!("{}.js", record.path))
    };
    let (mut facts, mut spans) = lang::lookup(file_type, &path)
        .map(|(pack, language, query)| {
            engine::extract(&language, query, &pack.hooks, &record.path, &blanked)
        })
        .unwrap_or_default();
    // Spans index the blanked text; they match `content` only when decoding kept every byte.
    if blanked.len() != content.len() {
        spans.clear();
    }
    let mut parts = record.path.rsplit('/');
    let stem = parts
        .next()
        .and_then(|name| name.split('.').next())
        .unwrap_or_default()
        // SvelteKit `+page.svelte` is the `Page` component of its route.
        .trim_start_matches('+');
    // `ArticleList/index.svelte` is imported and used as `ArticleList`.
    let stem = match (stem, parts.next()) {
        ("index", Some(dir)) => dir,
        _ => stem,
    };
    let lines = text.lines().count() as u32;
    let component = facts.symbols.len() as u32;
    facts.symbols.push(Symbol {
        kind: Some(SymKind::Class),
        name: pascal_case(stem),
        line: 1,
        end_line: lines.max(1),
        signature: format!("component {}", pascal_case(stem)),
        exported: true,
        ..Symbol::default()
    });
    for (line, tag) in component_tags(&text, &scripts) {
        facts.refs.push(Ref {
            kind: RefKind::Call,
            name: tag,
            qualifier: None,
            scope: Some(component),
            line,
            arity: crate::domain::facts::NO_ARITY,
        });
    }
    if svelte {
        for (line, name) in directive_values(&text, &skipped) {
            facts.refs.push(Ref {
                kind: RefKind::Value,
                name,
                qualifier: None,
                scope: None,
                line,
                arity: crate::domain::facts::NO_ARITY,
            });
        }
        facts.refs.retain(|reference| {
            !reference
                .name
                .strip_prefix('$')
                .is_some_and(|rune| RUNES.contains(&rune))
        });
        // Top-level script code and markup run per component instance.
        for reference in &mut facts.refs {
            reference.scope.get_or_insert(component);
            if let Some(store) = reference.name.strip_prefix('$')
                && !store.is_empty()
            {
                reference.name = store.to_owned();
            }
        }
    }
    super::routes::add(&record.path, &text, Some(component), &mut facts);
    (facts, spans)
}

/// Svelte 5 runes: compiler keywords, no stores.
const RUNES: [&str; 7] = [
    "state", "derived", "effect", "props", "bindable", "inspect", "host",
];

/// Byte ranges from each `open` to the end of the following `close`.
fn element_ranges(text: &str, open: &str, close: &str) -> Vec<std::ops::Range<usize>> {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(start) = lower[from..].find(open).map(|offset| from + offset) {
        let end = lower[start..]
            .find(close)
            .map_or(text.len(), |offset| start + offset + close.len());
        ranges.push(start..end);
        from = end;
    }
    ranges
}

/// Copies the code of each markup expression into `blanked` as a parenthesized statement
/// (`;(format(x))`): the braces become parentheses, a blank byte before them the separator.
/// Block tags keep their expression (`{#if a}`, `{#each items as item}` -> `items`), closing and
/// branch tags without one vanish.
fn svelte_expressions(text: &str, skipped: &[std::ops::Range<usize>], blanked: &mut [u8]) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(range) = skipped.iter().find(|range| range.contains(&index)) {
            index = range.end;
            continue;
        }
        if bytes[index] != b'{' {
            index += 1;
            continue;
        }
        let Some(close) = matching_brace(bytes, index) else {
            break;
        };
        let keep = expression_range(text, index + 1, close);
        let separator = (0..index)
            .rev()
            .find(|&at| blanked[at] != b'\n')
            .filter(|&at| blanked[at] == b' ');
        if let (Some(keep), Some(separator)) = (keep, separator) {
            blanked[keep.clone()].copy_from_slice(&bytes[keep]);
            blanked[separator] = b';';
            blanked[index] = b'(';
            blanked[close] = b')';
        }
        index = close + 1;
    }
}

/// Index of the `}` closing the `{` at `open`, skipping nested braces and string literals.
fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut index = open;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(_) if byte == b'\\' => index += 1,
            Some(delimiter) if byte == delimiter => quote = None,
            Some(_) => {}
            None => match byte {
                b'"' | b'\'' | b'`' => quote = Some(byte),
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            },
        }
        index += 1;
    }
    None
}

/// The code part of the expression between `start` and `end` (the braces excluded).
fn expression_range(text: &str, start: usize, end: usize) -> Option<std::ops::Range<usize>> {
    let inner = &text[start..end];
    let lead = inner.len() - inner.trim_start().len();
    let trimmed = inner.trim_start();
    let (skip, cut): (usize, Option<&str>) = if let Some(block) = trimmed.strip_prefix('#') {
        let keyword = block.split_whitespace().next().unwrap_or_default();
        match keyword {
            "if" | "key" => (1 + keyword.len(), None),
            "each" => (1 + keyword.len(), Some(" as ")),
            "await" => (1 + keyword.len(), Some(" then ")),
            _ => return None,
        }
    } else if trimmed.starts_with(":else if") {
        (":else if".len(), None)
    } else if trimmed.starts_with([':', '/']) {
        return None;
    } else if let Some(tag) = trimmed.strip_prefix('@') {
        let keyword = tag.split_whitespace().next().unwrap_or_default();
        (1 + keyword.len(), None)
    } else {
        (0, None)
    };
    let code_start = start + lead + skip;
    let code = &text[code_start..end];
    let code_end = cut
        .and_then(|cut| code.find(cut))
        .map_or(end, |offset| code_start + offset);
    (!text[code_start..code_end].trim().is_empty()).then_some(code_start..code_end)
}

/// Action, transition and animation directives name functions: `use:enhance`, `in:fly`.
fn directive_values(text: &str, skipped: &[std::ops::Range<usize>]) -> Vec<(u32, String)> {
    let mut found = Vec::new();
    for prefix in ["use:", "transition:", "in:", "out:", "animate:"] {
        let mut from = 0;
        while let Some(at) = text[from..].find(prefix).map(|offset| from + offset) {
            from = at + prefix.len();
            let attribute = at > 0 && text.as_bytes()[at - 1].is_ascii_whitespace();
            if !attribute || skipped.iter().any(|range| range.contains(&at)) {
                continue;
            }
            let name: String = text[from..]
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '$'))
                .collect();
            if !name.is_empty() {
                let line = text[..at].matches('\n').count() as u32 + 1;
                found.push((line, name));
            }
        }
    }
    found.sort();
    found
}

/// Byte ranges of script contents and whether any is TypeScript.
fn script_ranges(text: &str, astro: bool) -> (Vec<std::ops::Range<usize>>, bool) {
    let mut ranges = Vec::new();
    let mut typescript = astro;
    if astro
        && text.starts_with("---")
        && let Some(end) = text[3..].find("\n---")
    {
        ranges.push(3..3 + end);
    }
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(open) = lower[from..].find("<script") {
        let open = from + open;
        let Some(tag_end) = lower[open..].find('>').map(|end| open + end + 1) else {
            break;
        };
        let tag = &lower[open..tag_end];
        if tag.contains("lang=\"ts\"")
            || tag.contains("lang='ts'")
            || tag.contains("lang=\"typescript\"")
        {
            typescript = true;
        }
        let Some(close) = lower[tag_end..].find("</script>").map(|end| tag_end + end) else {
            break;
        };
        ranges.push(tag_end..close);
        from = close + "</script>".len();
    }
    (ranges, typescript)
}

/// Component tags outside scripts: `<PascalCase` and multi-word `<kebab-case`, with line.
fn component_tags(text: &str, scripts: &[std::ops::Range<usize>]) -> Vec<(u32, String)> {
    let mut tags = Vec::new();
    let bytes = text.as_bytes();
    let mut line = 1u32;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            line += 1;
        }
        let inside_script = scripts.iter().any(|range| range.contains(&index));
        if bytes[index] == b'<' && !inside_script {
            let name: String = text[index + 1..]
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.'))
                .collect();
            let is_component = name.starts_with(|ch: char| ch.is_ascii_uppercase())
                || (name.contains('-') && name.starts_with(|ch: char| ch.is_ascii_lowercase()));
            if is_component && name != "DOCTYPE" {
                tags.push((line, pascal_case(&name)));
            }
        }
        index += 1;
    }
    tags.dedup();
    tags
}

/// `my-button` and `my_button` -> `MyButton`; already PascalCase names stay.
fn pascal_case(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::role::Role;

    fn record(path: &str, file_type: FileType) -> FileRecord {
        FileRecord {
            path: path.to_owned(),
            size: 0,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(file_type),
        }
    }

    #[test]
    fn vue_script_and_template_components() {
        let source = "<template>\n  <div>\n    <UserCard :user=\"u\" />\n    <app-footer></app-footer>\n  </div>\n</template>\n<script setup lang=\"ts\">\nimport UserCard from './UserCard.vue'\nfunction load(): void { fetchUser() }\n</script>\n";
        let (facts, _) = extract(
            &record("src/components/user-page.vue", FileType::Vue),
            source.as_bytes(),
        );
        let symbols: Vec<(&str, u32)> = facts
            .symbols
            .iter()
            .map(|s| (s.name.as_str(), s.line))
            .collect();
        assert_eq!(symbols, [("load", 9), ("UserPage", 1)]);
        let calls: Vec<(&str, u32)> = facts
            .refs
            .iter()
            .filter(|r| r.kind == RefKind::Call)
            .map(|r| (r.name.as_str(), r.line))
            .collect();
        assert_eq!(calls, [("fetchUser", 9), ("UserCard", 3), ("AppFooter", 4)]);
        assert_eq!(
            facts.imports.first().map(|i| i.spec.as_str()),
            Some("./UserCard.vue")
        );
    }

    #[test]
    fn astro_frontmatter_is_typescript() {
        let source = "---\nimport Layout from '../layouts/Layout.astro';\nconst title = getTitle();\n---\n<Layout title={title}><Header /></Layout>\n";
        let (facts, _) = extract(
            &record("src/pages/index.astro", FileType::Astro),
            source.as_bytes(),
        );
        let calls: Vec<&str> = facts
            .refs
            .iter()
            .filter(|r| r.kind == RefKind::Call)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(calls, ["getTitle", "Layout", "Header"]);
    }

    #[test]
    fn svelte_markup_expressions_directives_and_stores() {
        let source = "<script>\n  import { enhance } from '$app/forms';\n  import { user } from '$lib/stores';\n  let { items } = $props();\n  function format(x) { return x }\n  function save() {}\n</script>\n\n{#each items as item}\n  <p>{format(item)}</p>\n{/each}\n<button onclick={save}>{$user.name}</button>\n<form use:enhance></form>\n<style>\n  p { color: red }\n</style>\n";
        let (facts, _) = extract(
            &record("src/routes/items/+page.svelte", FileType::Svelte),
            source.as_bytes(),
        );
        let mut refs: Vec<(&str, u32)> = facts
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Call | RefKind::Value))
            .map(|r| (r.name.as_str(), r.line))
            .collect();
        refs.sort_unstable();
        assert_eq!(
            refs,
            [
                ("Page", 1),
                ("enhance", 13),
                ("format", 10),
                ("item", 10),
                ("save", 12),
                ("user", 12)
            ]
        );
        let component = facts.symbols.iter().position(|s| s.name == "Page");
        assert!(
            facts
                .refs
                .iter()
                .filter(|r| r.kind != RefKind::Call || r.name != "Page")
                .all(|r| r.scope.is_some())
        );
        assert!(component.is_some());
        let routes: Vec<&str> = facts
            .symbols
            .iter()
            .filter(|s| s.kind == Some(SymKind::Op))
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(routes, ["page /items"]);
    }
}
