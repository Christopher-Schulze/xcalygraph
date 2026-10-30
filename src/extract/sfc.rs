//! Single-file components (Vue, Svelte, Astro): script blocks and Astro frontmatter are parsed
//! as TypeScript or JavaScript in place (other bytes blanked, so lines stay exact); component
//! tags of the template count as calls; the file itself is a component symbol named after its
//! stem (`docs/spec.md` section 3, embedded code).

use super::lang;
use crate::domain::corpus::FileRecord;
use crate::domain::facts::{Facts, Ref, RefKind, SymKind, Symbol};
use crate::domain::filetype::FileType;
use crate::extract::engine;

pub fn extract(record: &FileRecord, content: &[u8]) -> Facts {
    let text = String::from_utf8_lossy(content);
    let (scripts, typescript) = script_ranges(&text, record.file_type == Some(FileType::Astro));
    let mut blanked: Vec<u8> = text
        .bytes()
        .map(|byte| if byte == b'\n' { b'\n' } else { b' ' })
        .collect();
    for range in &scripts {
        blanked[range.clone()].copy_from_slice(&text.as_bytes()[range.clone()]);
    }
    let (file_type, path) = if typescript {
        (FileType::TypeScript, format!("{}.ts", record.path))
    } else {
        (FileType::JavaScript, format!("{}.js", record.path))
    };
    let mut facts = lang::lookup(file_type, &path)
        .map(|(pack, language, query)| {
            engine::extract(&language, query, &pack.hooks, &record.path, &blanked)
        })
        .unwrap_or_default();
    let stem = record
        .path
        .rsplit('/')
        .next()
        .and_then(|name| name.split('.').next())
        .unwrap_or_default();
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
        });
    }
    facts
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
        let facts = extract(
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
}
