//! Single-file components (Vue, Svelte, Astro): script blocks and Astro frontmatter are parsed
//! as TypeScript or JavaScript in place (other bytes blanked, so lines stay exact); component
//! tags outside comments, scripts and styles count as calls; the file itself is a component
//! symbol named after its stem (`docs/spec.md` section 3, embedded code). Svelte markup expressions (`{format(x)}`,
//! `onclick={save}`, `{#if ready}`) are parsed in place as statements and `use:enhance`
//! directives count as values, all scoped to the component, so script functions used only by
//! the markup are not dead; `$store` reads name the store.

use super::lang;
use crate::domain::corpus::FileRecord;
use crate::domain::facts::{Coverage, Facts, Ref, RefKind, SymKind, Symbol};
use crate::domain::filetype::FileType;
use crate::extract::engine;

use super::markup::{markup_tag, raw_element_close};

pub fn extract(record: &FileRecord, content: &[u8], capture_identities: bool) -> engine::Parsed {
    let text = String::from_utf8_lossy(content);
    let MarkupContext {
        scripts,
        skipped,
        typescript,
    } = markup_context(&text, record.file_type == Some(FileType::Astro));
    let mut blanked: Vec<u8> = text
        .bytes()
        .map(|byte| if byte == b'\n' { b'\n' } else { b' ' })
        .collect();
    for range in &scripts {
        blanked[range.clone()].copy_from_slice(&text.as_bytes()[range.clone()]);
    }
    let svelte = record.file_type == Some(FileType::Svelte);
    if svelte {
        svelte_expressions(&text, &skipped, &mut blanked);
    }
    let (file_type, path) = if typescript {
        (FileType::TypeScript, format!("{}.ts", record.path))
    } else {
        (FileType::JavaScript, format!("{}.js", record.path))
    };
    let parsed = lang::lookup(file_type, &path)
        .map(|(pack, language, query)| {
            engine::extract(
                &language,
                query,
                &pack.hooks,
                &record.path,
                &blanked,
                capture_identities,
            )
        })
        .unwrap_or_else(|| engine::Parsed {
            facts: Facts {
                coverage: Coverage::NoGrammar,
                ..Facts::default()
            },
            incomplete: capture_identities,
            ..engine::Parsed::default()
        });
    let engine::Parsed {
        mut facts,
        mut spans,
        mut identities,
        mut incomplete,
    } = parsed;
    // Spans index the blanked text; they match `content` only when decoding kept every byte.
    if blanked.len() != content.len() {
        spans.clear();
        for reference in &mut facts.refs {
            reference.byte = crate::domain::facts::NO_BYTE;
        }
        for binding in &mut facts.bindings {
            binding.value_byte = crate::domain::facts::NO_BYTE;
            binding.visibility = None;
        }
        if capture_identities {
            identities.fill(None);
            incomplete = true;
        }
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
    let component_signature = format!("component {}", pascal_case(stem));
    if capture_identities {
        identities.push(Some(component_signature.clone()));
    }
    facts.symbols.push(Symbol {
        kind: Some(SymKind::Class),
        name: pascal_case(stem).into(),
        line: 1,
        end_line: lines.max(1),
        signature: component_signature.into(),
        exported: true,
        ..Symbol::default()
    });
    for (line, tag) in component_tags(&text, &skipped) {
        facts.refs.push(Ref {
            kind: RefKind::Call,
            name: tag.into(),
            qualifier: None,
            scope: Some(component),
            line,
            arity: crate::domain::facts::NO_ARITY,
            byte: crate::domain::facts::NO_BYTE,
        });
    }
    if svelte {
        for (line, name) in directive_values(&text, &skipped) {
            facts.refs.push(Ref {
                kind: RefKind::Value,
                name: name.into(),
                qualifier: None,
                scope: None,
                line,
                arity: crate::domain::facts::NO_ARITY,
                byte: crate::domain::facts::NO_BYTE,
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
                reference.name = store.into();
            }
        }
    }
    super::routes::add(&record.path, &text, Some(component), &mut facts);
    if capture_identities {
        identities.resize(facts.symbols.len(), None);
    }
    engine::Parsed {
        facts,
        spans,
        identities,
        incomplete,
    }
}

/// Svelte 5 runes: compiler keywords, no stores.
const RUNES: [&str; 7] = [
    "state", "derived", "effect", "props", "bindable", "inspect", "host",
];

struct MarkupContext {
    scripts: Vec<std::ops::Range<usize>>,
    skipped: Vec<std::ops::Range<usize>>,
    typescript: bool,
}

/// Finds script contents and template regions outside comments, scripts and styles.
fn markup_context(text: &str, astro: bool) -> MarkupContext {
    let mut context = MarkupContext {
        scripts: Vec::new(),
        skipped: Vec::new(),
        typescript: astro,
    };
    let mut from = 0;
    if astro
        && text.starts_with("---")
        && let Some(end) = text[3..].find("\n---").map(|offset| offset + 3)
    {
        let close_end = end + "\n---".len();
        context.scripts.push(3..end);
        context.skipped.push(0..close_end);
        from = close_end;
    }

    let lower = text.to_ascii_lowercase();
    while from < text.len() {
        if let Some(rest) = text.get(from..)
            && rest.starts_with("<!--")
        {
            let end = rest
                .find("-->")
                .map_or(text.len(), |offset| from + offset + "-->".len());
            context.skipped.push(from..end);
            from = end;
            continue;
        }
        if text.as_bytes()[from] == b'<'
            && let Some(tag) = markup_tag(text, from)
        {
            if !tag.closing
                && (tag.name.eq_ignore_ascii_case("script")
                    || tag.name.eq_ignore_ascii_case("style"))
            {
                let is_script = tag.name.eq_ignore_ascii_case("script");
                let attributes = &lower[from..tag.end];
                if is_script
                    && (attributes.contains("lang=\"ts\"")
                        || attributes.contains("lang='ts'")
                        || attributes.contains("lang=\"typescript\""))
                {
                    context.typescript = true;
                }
                let close = raw_element_close(text, &lower, tag.end, tag.name);
                if is_script && let Some(close) = close.as_ref() {
                    context.scripts.push(tag.end..close.start);
                }
                let block_end = close.map_or(text.len(), |close| close.end);
                context.skipped.push(from..block_end);
                from = block_end;
                continue;
            }
            from = tag.end;
            continue;
        }
        from += 1;
    }
    context
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

/// Component tags outside excluded markup: `<PascalCase` and multi-word `<kebab-case`, with line.
fn component_tags(text: &str, skipped: &[std::ops::Range<usize>]) -> Vec<(u32, String)> {
    let mut tags = Vec::new();
    let bytes = text.as_bytes();
    let mut line = 1u32;
    let mut index = 0;
    let mut skipped_index = 0;
    while index < bytes.len() {
        while skipped
            .get(skipped_index)
            .is_some_and(|range| range.end <= index)
        {
            skipped_index += 1;
        }
        if let Some(range) = skipped.get(skipped_index)
            && range.start <= index
        {
            line += bytes[index..range.end]
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count() as u32;
            index = range.end;
            skipped_index += 1;
            continue;
        }
        if bytes[index] == b'<'
            && let Some(tag) = markup_tag(text, index)
        {
            let is_component = tag.name.starts_with(|ch: char| ch.is_ascii_uppercase())
                || (tag.name.contains('-')
                    && tag.name.starts_with(|ch: char| ch.is_ascii_lowercase()));
            if !tag.closing && is_component && tag.name != "DOCTYPE" {
                tags.push((line, pascal_case(tag.name)));
            }
            line += bytes[index..tag.end]
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count() as u32;
            index = tag.end;
            continue;
        }
        if bytes[index] == b'\n' {
            line += 1;
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
        let facts = extract(
            &record("src/components/user-page.vue", FileType::Vue),
            source.as_bytes(),
            false,
        )
        .facts;
        let symbols: Vec<(&str, u32)> = facts
            .symbols
            .iter()
            .map(|s| (s.name.as_ref(), s.line))
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
        let invalid = [b"\xff".as_slice(), source.as_bytes()].concat();
        let invalid = extract(
            &record("src/components/user-page.vue", FileType::Vue),
            &invalid,
            false,
        )
        .facts;
        assert!(!invalid.refs.is_empty());
        assert!(
            invalid
                .refs
                .iter()
                .all(|reference| reference.byte == crate::domain::facts::NO_BYTE)
        );
    }

    #[test]
    fn component_extraction_ignores_markup_comments_and_styles() {
        let cases = [
            (
                "src/components/probe.vue",
                FileType::Vue,
                "<!--\n<GhostComment />\n<script>function ghostHtml() {}</script>\n-->\n<RealWidget /> • markup text\n<style>\n/* <GhostStyle />\n<script>function ghostCss() {}</script> */\n</style>\n<script>function liveScript() {}</script>\n",
                &[("liveScript", 10)][..],
            ),
            (
                "src/components/probe.svelte",
                FileType::Svelte,
                "<!--\n<GhostComment />\n<script>function ghostHtml() {}</script>\n-->\n<RealWidget /> • markup text\n<style>\n/* <GhostStyle />\n<script>function ghostCss() {}</script> */\n</style>\n<script>function liveScript() {}</script>\n",
                &[("liveScript", 10)][..],
            ),
            (
                "src/components/probe.astro",
                FileType::Astro,
                "---\nfunction liveFrontmatter() {}\n---\n<!--\n<GhostComment />\n<script>function ghostHtml() {}</script>\n-->\n<RealWidget /> • markup text\n<style>\n/* <GhostStyle />\n<script>function ghostCss() {}</script> */\n</style>\n<script>function liveScript() {}</script>\n",
                &[("liveFrontmatter", 2), ("liveScript", 13)][..],
            ),
        ];

        for (path, file_type, source, expected_symbols) in cases {
            let facts = extract(&record(path, file_type), source.as_bytes(), false).facts;
            let symbols: Vec<(&str, u32)> = facts
                .symbols
                .iter()
                .map(|symbol| (symbol.name.as_ref(), symbol.line))
                .collect();
            for expected in expected_symbols {
                assert!(symbols.contains(expected), "{path}: missing {expected:?}");
            }
            for unexpected in ["ghostHtml", "ghostCss"] {
                assert!(
                    symbols.iter().all(|(name, _)| *name != unexpected),
                    "{path}: extracted {unexpected}"
                );
            }

            let calls: Vec<(&str, u32)> = facts
                .refs
                .iter()
                .filter(|reference| reference.kind == RefKind::Call)
                .map(|reference| (reference.name.as_str(), reference.line))
                .collect();
            let expected_tag_line = if file_type == FileType::Astro { 8 } else { 5 };
            assert_eq!(
                calls,
                [("RealWidget", expected_tag_line)],
                "{path}: unexpected component calls"
            );
        }
    }

    #[test]
    fn astro_frontmatter_is_typescript() {
        let source = "---\nimport Layout from '../layouts/Layout.astro';\nconst title = getTitle();\n---\n<Layout title={title}><Header /></Layout>\n";
        let facts = extract(
            &record("src/pages/index.astro", FileType::Astro),
            source.as_bytes(),
            false,
        )
        .facts;
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
        let facts = extract(
            &record("src/routes/items/+page.svelte", FileType::Svelte),
            source.as_bytes(),
            false,
        )
        .facts;
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
        let component = facts.symbols.iter().position(|s| *s.name == *"Page");
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
            .map(|s| s.name.as_ref())
            .collect();
        assert_eq!(routes, ["page /items"]);
    }
}
