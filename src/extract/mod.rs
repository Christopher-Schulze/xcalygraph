//! Fact extraction: content bytes to per-file facts (`docs/spec.md` section 5). Pure; no IO.

mod assets;
mod build_manifests;
mod config;
mod contracts;
mod documents;
pub mod engine;
#[cfg(test)]
mod golden;
mod hcl;
pub mod json;
pub mod lang;
pub mod manifest;
pub mod markdown;
mod prose;
pub mod sfc;
mod sql;
mod structural;
mod tasks;

use crate::domain::corpus::FileRecord;
use crate::domain::facts::Facts;
use crate::domain::filetype::FileType;
use crate::domain::role::{self, Role};

/// Bumped whenever extraction code changes output, so existing indexes rebuild.
const EXTRACTION_VERSION: u32 = 36;

/// Extraction version stored in the index: [`EXTRACTION_VERSION`] combined with a hash of every
/// compiled-in query, so a changed query re-extracts without a manual bump.
pub fn extraction_version() -> u32 {
    let mut hash = xxhash_rust::xxh3::xxh3_64(&EXTRACTION_VERSION.to_le_bytes());
    for pack in lang::all_packs() {
        hash ^= xxhash_rust::xxh3::xxh3_64(pack.query.as_bytes())
            .rotate_left(u32::from(pack.file_type as u8));
    }
    (hash ^ (hash >> 32)) as u32
}

/// Extracts the facts of one file of an extracted role.
pub fn extract(record: &FileRecord, content: &[u8]) -> Facts {
    if matches!(
        record.file_type,
        Some(FileType::Vue | FileType::Svelte | FileType::Astro)
    ) {
        let mut facts = sfc::extract(record, content);
        facts.lines = count_lines(content);
        return facts;
    }
    if let Some(
        file_type @ (FileType::Text
        | FileType::Rst
        | FileType::Org
        | FileType::AsciiDoc
        | FileType::Tex
        | FileType::Html),
    ) = record.file_type
        && record.role == Role::Doc
    {
        let mut facts = prose::extract(file_type, &String::from_utf8_lossy(content));
        facts.lines = count_lines(content);
        return facts;
    }
    if let Some(file_type) = record
        .file_type
        .filter(|file_type| file_type.class() == crate::domain::filetype::Class::Structural)
    {
        let mut facts = structural::extract(file_type, &record.path, content);
        if file_type != FileType::DmResource {
            assets::literal_paths(&String::from_utf8_lossy(content), &mut facts);
        }
        return facts;
    }
    let document = match record.file_type {
        Some(FileType::Pdf) => Some(documents::pdf(content)),
        Some(FileType::Docx) => Some(documents::docx(content)),
        Some(FileType::Xlsx) => Some(documents::xlsx(content)),
        Some(FileType::Image | FileType::Svg | FileType::Video | FileType::Audio) => {
            let mut facts = assets::extract(&record.path, content);
            if record.file_type == Some(FileType::Svg) {
                facts.lines = count_lines(content);
            }
            Some(facts)
        }
        _ => None,
    };
    if let Some(facts) = document {
        return facts;
    }
    if record.file_type == Some(FileType::Markdown) {
        let mut facts = markdown::extract(&String::from_utf8_lossy(content));
        facts.lines = count_lines(content);
        return facts;
    }
    #[cfg(feature = "lang-groovy")]
    let terminated;
    #[cfg(feature = "lang-groovy")]
    let content = if record.file_type == Some(FileType::Groovy) {
        terminated = lang::groovy::terminate_statements(content);
        terminated.as_slice()
    } else {
        content
    };
    // Delphi and Lazarus forms are component trees, Lazarus packages XML: no Pascal source.
    #[cfg(feature = "lang-pascal")]
    if record.file_type == Some(FileType::Pascal) {
        let lower = record.path.to_ascii_lowercase();
        if lower.ends_with(".dfm") || lower.ends_with(".lfm") {
            let mut facts = lang::pascal::form(&record.path, &String::from_utf8_lossy(content));
            facts.lines = count_lines(content);
            return facts;
        }
        if lower.ends_with(".lpk") {
            return Facts {
                lines: count_lines(content),
                ..Facts::default()
            };
        }
    }
    #[cfg(any(feature = "lang-c", feature = "lang-cpp", feature = "lang-objc"))]
    let masked;
    #[cfg(any(feature = "lang-c", feature = "lang-cpp", feature = "lang-objc"))]
    let content = if matches!(
        record.file_type,
        Some(FileType::C | FileType::Cpp | FileType::ObjC)
    ) && let Some(rewritten) = lang::c::mask_platform_macros(content)
    {
        masked = rewritten;
        masked.as_slice()
    } else {
        content
    };
    // World of Warcraft `.toc` files list the addon's files; they are no Lua source.
    let toc = record.path.ends_with(".toc");
    let mut facts = record
        .file_type
        .filter(|_| !toc)
        .and_then(|file_type| lang::lookup(file_type, &record.path))
        .map(|(pack, language, query)| {
            engine::extract(&language, query, &pack.hooks, &record.path, content)
        })
        .unwrap_or_default();
    #[cfg(any(feature = "lang-c", feature = "lang-cpp", feature = "lang-objc"))]
    if matches!(
        record.file_type,
        Some(FileType::C | FileType::Cpp | FileType::ObjC)
    ) {
        lang::c::macro_references(content, &mut facts);
    }
    #[cfg(feature = "lang-lua")]
    if toc {
        lang::lua::toc_imports(&String::from_utf8_lossy(content), &mut facts);
    }
    if record.file_type == Some(FileType::Terraform) {
        hcl::extract(&String::from_utf8_lossy(content), &mut facts);
    }
    // SQL objects of `.sql` files; SQL statements in string literals of code.
    if record.file_type == Some(FileType::Sql) {
        sql::objects(&String::from_utf8_lossy(content), &mut facts);
    } else if matches!(record.role, Role::Src | Role::Test | Role::Gen)
        && record
            .file_type
            .is_some_and(|file_type| file_type.is_code())
    {
        sql::embedded_queries(&String::from_utf8_lossy(content), &mut facts);
    }
    #[cfg(feature = "lang-vbnet")]
    if record.file_type == Some(FileType::VbNet) {
        lang::vbnet::scan_lines(content, &mut facts);
    }
    // Shell variables read without assignment count as environment only when written like
    // environment variables (`HOME`, `API_TOKEN`); `$1` and lowercase locals do not.
    if record.file_type == Some(FileType::Bash) {
        facts.envs.retain(|env| {
            env.name
                .starts_with(|ch: char| ch.is_ascii_uppercase() || ch == '_')
                && env
                    .name
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        });
    }
    #[cfg(feature = "lang-dm")]
    if record.file_type == Some(FileType::Dm) {
        lang::dm::type_paths(&mut facts);
    }
    // PowerShell `$env:NAME` reads are captured as the whole variable.
    if record.file_type == Some(FileType::PowerShell) {
        for env in &mut facts.envs {
            if let Some((_, name)) = env.name.split_once(':') {
                env.name = name.trim_matches(['{', '}']).to_owned();
            }
        }
    }
    if matches!(record.role, Role::Config | Role::Manifest)
        && let Some(file_type) = record.file_type
    {
        let text = String::from_utf8_lossy(content);
        if record.role == Role::Config {
            config::extract(&record.path, file_type, &text, &mut facts);
        }
        match file_type {
            FileType::Proto => contracts::proto(&text, &mut facts),
            FileType::GraphQl => contracts::graphql(&text, &mut facts),
            FileType::Yaml if contracts::is_openapi(&text) => {
                contracts::openapi_yaml(&text, &mut facts)
            }
            FileType::Json if contracts::is_openapi(&text) => {
                contracts::openapi_json(&text, &mut facts)
            }
            _ => {}
        }
        match file_type {
            FileType::Make => tasks::makefile(&text, &mut facts),
            FileType::Dockerfile => tasks::dockerfile(&text, &mut facts),
            FileType::Yaml => tasks::yaml(&record.path, &text, &mut facts),
            FileType::Json if role::file_name(&record.path) == "package.json" => {
                tasks::package_scripts(&text, &mut facts);
            }
            _ => {}
        }
    }
    if matches!(
        record.role,
        Role::Src | Role::Test | Role::Gen | Role::Config
    ) {
        assets::literal_paths(&String::from_utf8_lossy(content), &mut facts);
    }
    facts.lines = count_lines(content);
    if record.role == Role::Manifest {
        let name = role::file_name(&record.path);
        facts.manifest = manifest::parse(name, &String::from_utf8_lossy(content));
    }
    facts.entry |= content.starts_with(b"#!");
    facts
}

pub fn count_lines(content: &[u8]) -> u32 {
    let newlines = content.iter().filter(|&&byte| byte == b'\n').count();
    let trailing = usize::from(!content.is_empty() && content.last() != Some(&b'\n'));
    u32::try_from(newlines + trailing).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grammar exploration for query work: prints the syntax tree of the file in
    /// `XCALYGRAPH_TREE_FILE` (type from its name). Development only:
    /// `XCALYGRAPH_TREE_FILE=x.m cargo test --release dump_tree -- --ignored --nocapture`.
    #[test]
    #[ignore = "development tool; needs XCALYGRAPH_TREE_FILE"]
    fn dump_tree() -> Result<(), Box<dyn std::error::Error>> {
        let Some(path) = std::env::var_os("XCALYGRAPH_TREE_FILE") else {
            return Ok(());
        };
        let path = std::path::PathBuf::from(path);
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let source = std::fs::read(&path)?;
        let file_type = match crate::domain::filetype::detect(name) {
            crate::domain::filetype::Detection::Known(file_type) => Some(file_type),
            crate::domain::filetype::Detection::Ambiguous(ambiguous) => {
                crate::domain::filetype::resolve_ambiguous(ambiguous, &source, false)
            }
            _ => None,
        }
        .ok_or_else(|| format!("no file type for {name}"))?;
        let Some((_, language, _)) = lang::lookup(file_type, name) else {
            return Err(format!("no grammar for {name}").into());
        };
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language)?;
        let tree = parser.parse(&source, None).ok_or("parse failed")?;
        let mut cursor = tree.walk();
        let mut depth = 0usize;
        loop {
            let node = cursor.node();
            if node.is_named() {
                let field = cursor
                    .field_name()
                    .map_or(String::new(), |field| format!("{field}: "));
                let text: String = node
                    .utf8_text(&source)
                    .unwrap_or_default()
                    .chars()
                    .take(60)
                    .map(|ch| if ch == '\n' { ' ' } else { ch })
                    .collect();
                println!(
                    "{}{field}{} {} | {text}",
                    "  ".repeat(depth),
                    node.kind(),
                    node.start_position().row + 1
                );
            }
            if cursor.goto_first_child() {
                depth += 1;
                continue;
            }
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    return Ok(());
                }
                depth -= 1;
            }
        }
    }

    #[test]
    fn lines_count_a_missing_final_newline() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a"), 1);
        assert_eq!(count_lines(b"a\n"), 1);
        assert_eq!(count_lines(b"a\nb"), 2);
    }

    #[test]
    fn text_manifests_are_parsed_not_read_as_prose() {
        let record = |path: &str, role: Role| FileRecord {
            path: path.to_owned(),
            size: 0,
            mtime_ns: 0,
            hash: 0,
            role,
            file_type: Some(FileType::Text),
        };
        let cmake = extract(
            &record("CMakeLists.txt", Role::Manifest),
            b"include_directories(include)\n",
        );
        assert_eq!(
            cmake.manifest.map(|manifest| manifest.include_dirs),
            Some(vec!["include".to_owned()])
        );
        let remappings = extract(&record("remappings.txt", Role::Manifest), b"@oz/=lib/oz/\n");
        assert_eq!(
            remappings.manifest.map(|manifest| manifest.deps),
            Some(vec![("@oz/".to_owned(), "lib/oz/".to_owned())])
        );
        let notes = extract(&record("docs/notes.txt", Role::Doc), b"First line.\n");
        assert_eq!(notes.doc, "First line.");
    }
}
