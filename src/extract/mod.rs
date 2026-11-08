//! Fact extraction: content bytes to per-file facts (`docs/spec.md` section 5). Pure; no IO.

mod assets;
mod bindings;
mod build_manifests;
#[cfg(feature = "lang-javascript")]
mod commonjs;
mod config;
mod contracts;
mod documents;
pub mod engine;
#[cfg(feature = "lang-go")]
mod go_embed;
#[cfg(test)]
mod golden;
mod hcl;
mod html;
pub mod json;
pub mod lang;
pub mod manifest;
pub mod markdown;
mod markup;
mod prose;
#[cfg(test)]
mod query_cost;
mod routes;
mod sections;
pub mod sfc;
mod sql;
mod structural;
mod tasks;
mod terms;
mod yaml;

use crate::domain::corpus::FileRecord;
use crate::domain::facts::{Coverage, Facts};
use crate::domain::filetype::{Class, FileType};
use crate::domain::role::{self, Role};

/// Bumped whenever extraction code changes output, so existing indexes rebuild.
/// Rows of a CSV or TSV file that reach search.
const CSV_INDEXED_ROWS: usize = 2000;
const EXTRACTION_VERSION: u32 = 161;

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
    extract_inner(record, content, false).facts
}

/// Extracts the usual facts plus transient, complete declaration identities for diff analysis.
pub fn extract_with_identities(record: &FileRecord, content: &[u8]) -> engine::Parsed {
    extract_inner(record, content, true)
}

fn extract_inner(record: &FileRecord, content: &[u8], capture_identities: bool) -> engine::Parsed {
    if matches!(
        record.file_type,
        Some(FileType::Vue | FileType::Svelte | FileType::Astro)
    ) {
        let mut parsed = sfc::extract(record, content, capture_identities);
        if record.role == Role::Src {
            terms::add(content, &mut parsed.facts, &parsed.spans);
        }
        parsed.facts.lines = count_lines(content);
        return parsed;
    }
    if record.file_type == Some(FileType::Html) && record.role == Role::Doc {
        return html::extract(record, content, capture_identities);
    }
    if let Some(
        file_type @ (FileType::Text
        | FileType::Rst
        | FileType::Org
        | FileType::AsciiDoc
        | FileType::Csv
        | FileType::Tex),
    ) = record.file_type
        && matches!(record.role, Role::Doc | Role::Config)
    {
        let text = String::from_utf8_lossy(content);
        let mut facts = prose::extract(file_type, &text);
        facts.lines = count_lines(content);
        if record.role == Role::Config {
            config::extract(&record.path, file_type, &text, &mut facts);
            config::retain_in_root_paths(&record.path, file_type, &mut facts);
        }
        if file_type == FileType::Csv {
            // A data table: its first rows say what it holds, the rest would only add sections.
            let head: String = text
                .lines()
                .take(CSV_INDEXED_ROWS)
                .collect::<Vec<_>>()
                .join("\n");
            sections::index_lines(&mut facts, &head);
            if text.lines().nth(CSV_INDEXED_ROWS).is_some() {
                facts.coverage = facts.coverage.max(Coverage::MatchLimit);
            }
        } else {
            sections::index_lines(&mut facts, &text);
        }
        return without_identities(facts, capture_identities);
    }
    if let Some(file_type) = record
        .file_type
        .filter(|file_type| file_type.class() == crate::domain::filetype::Class::Structural)
    {
        let mut facts = structural::extract(file_type, &record.path, content);
        assets::literal_paths(&String::from_utf8_lossy(content), &mut facts);
        return without_identities(facts, capture_identities);
    }
    let document = match record.file_type {
        Some(FileType::Pdf) => Some(documents::pdf(content)),
        Some(FileType::Docx) => Some(documents::docx(content)),
        Some(FileType::Xlsx) => Some(documents::xlsx(content)),
        Some(FileType::Pptx) => Some(documents::pptx(content)),
        Some(FileType::OpenDocument) => Some(documents::opendocument(content)),
        Some(FileType::Rtf) => Some(documents::rtf(content)),
        Some(FileType::LegacyOffice) => Some(documents::legacy_office(&record.path, content)),
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
        return without_identities(facts, capture_identities);
    }
    if record.file_type == Some(FileType::Markdown) {
        let text = String::from_utf8_lossy(content);
        let mut facts = markdown::extract(&text);
        facts.lines = count_lines(content);
        sections::index_lines(&mut facts, &text);
        return without_identities(facts, capture_identities);
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
            return without_identities(facts, capture_identities);
        }
        if lower.ends_with(".lpk") {
            return without_identities(
                Facts {
                    lines: count_lines(content),
                    ..Facts::default()
                },
                capture_identities,
            );
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
    let capture_sql = matches!(record.role, Role::Src | Role::Test | Role::Gen)
        && record.file_type.is_some_and(FileType::is_code);
    let grammar = record
        .file_type
        .filter(|_| !toc)
        .and_then(|file_type| lang::lookup(file_type, &record.path));
    let parsed_with_grammar = grammar.is_some();
    let parsed = grammar
        .map(|(pack, language, query)| {
            let post_parse: Option<engine::PostParse> =
                capture_sql.then_some(sql::post_parse as engine::PostParse);
            #[cfg(feature = "lang-javascript")]
            let post_parse =
                if pack.file_type == FileType::JavaScript && !record.path.ends_with(".mjs") {
                    let hook: engine::PostParse = if capture_sql {
                        |source, root, parsed, capture_identities| {
                            commonjs::post_parse(source, root, parsed, capture_identities);
                            sql::post_parse(source, root, parsed, capture_identities);
                        }
                    } else {
                        commonjs::post_parse
                    };
                    Some(hook)
                } else {
                    post_parse
                };
            #[cfg(feature = "lang-bash")]
            let post_parse = if pack.file_type == FileType::Bash {
                let hook: engine::PostParse = if capture_sql {
                    |source, root, parsed, capture_identities| {
                        lang::bash::post_parse(source, root, parsed, capture_identities);
                        sql::post_parse(source, root, parsed, capture_identities);
                    }
                } else {
                    lang::bash::post_parse
                };
                Some(hook)
            } else {
                post_parse
            };
            engine::extract_with_post_parse(
                &language,
                query,
                &pack.hooks,
                &record.path,
                content,
                capture_identities,
                post_parse,
            )
        })
        .unwrap_or_else(|| {
            let mut parsed = engine::Parsed::default();
            if !toc
                && record.file_type.is_some_and(|file_type| {
                    // SQL and Terraform have their own extractors below, no grammar.
                    file_type.class() == Class::Deep
                        && !matches!(file_type, FileType::Sql | FileType::Terraform)
                })
            {
                parsed.facts.coverage = Coverage::NoGrammar;
            }
            parsed
        });
    let engine::Parsed {
        mut facts,
        spans,
        mut identities,
        incomplete,
    } = parsed;
    #[cfg(any(feature = "lang-c", feature = "lang-cpp", feature = "lang-objc"))]
    if matches!(
        record.file_type,
        Some(FileType::C | FileType::Cpp | FileType::ObjC)
    ) {
        lang::c::macro_references(content, &mut facts);
    }
    if matches!(
        record.file_type,
        Some(FileType::TypeScript | FileType::JavaScript)
    ) && matches!(record.role, Role::Src | Role::Gen)
    {
        routes::add(
            &record.path,
            &String::from_utf8_lossy(content),
            None,
            &mut facts,
        );
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
    } else if capture_sql && !parsed_with_grammar {
        sql::embedded_queries(&String::from_utf8_lossy(content), None, &mut facts);
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
    // PowerShell `$env:NAME` reads are captured as the whole variable.
    if record.file_type == Some(FileType::PowerShell) {
        for env in &mut facts.envs {
            if let Some((_, name)) = env.name.split_once(':') {
                env.name = name.trim_matches(['{', '}']).to_owned();
            }
        }
    }
    let mut typescript_candidate = None;
    if matches!(record.role, Role::Config | Role::Manifest)
        && let Some(file_type) = record.file_type
    {
        let text = String::from_utf8_lossy(content);
        if record.role == Role::Config {
            typescript_candidate = config::extract(&record.path, file_type, &text, &mut facts);
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
    ) && !(record.role == Role::Config
        && matches!(record.file_type, Some(FileType::Json | FileType::Yaml)))
    {
        assets::literal_paths(&String::from_utf8_lossy(content), &mut facts);
    }
    if record.role == Role::Config
        && let Some(file_type) = record.file_type
    {
        config::retain_in_root_paths(&record.path, file_type, &mut facts);
    }
    // Test and generated bodies are left out: they rank below sources and would double the
    // index share.
    if record.role == Role::Src
        && record
            .file_type
            .is_some_and(|file_type| file_type.is_code())
    {
        terms::add(content, &mut facts, &spans);
    }
    facts.lines = count_lines(content);
    if record.role == Role::Manifest
        || (record.role == Role::Config
            && (manifest::is_project_config(role::file_name(&record.path))
                || manifest::is_deno_config(role::file_name(&record.path))))
    {
        let name = role::file_name(&record.path);
        let text = String::from_utf8_lossy(content);
        match manifest::parse_with_project_scope(name, &text) {
            Some((manifest, scope_identity)) => {
                facts.manifest = Some(manifest);
                if let Some(identity) = scope_identity {
                    push_identity(&mut facts, identity);
                }
            }
            None if matches!(name, "tsconfig.json" | "package.json") => push_identity(
                &mut facts,
                crate::domain::facts::SourceIdentity {
                    kind: if name == "tsconfig.json" {
                        crate::domain::facts::SourceIdentityKind::TypeScriptProject
                    } else {
                        crate::domain::facts::SourceIdentityKind::TypeScriptPackageType
                    },
                    index: 0,
                    identity: "unproven".to_owned(),
                },
            ),
            None => {}
        }
    } else if let Some(has_options) = typescript_candidate {
        facts.manifest = if has_options {
            manifest::parse_typescript_candidate(&String::from_utf8_lossy(content))
        } else {
            Some(crate::domain::facts::Manifest::default())
        };
    }
    facts.entry |= content.starts_with(b"#!");
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

fn push_identity(facts: &mut Facts, identity: crate::domain::facts::SourceIdentity) {
    let mut identities = facts
        .source_identities
        .take()
        .map_or_else(Vec::new, |items| items.into_vec());
    identities.push(identity);
    identities.sort_unstable_by_key(|identity| (identity.kind, identity.index));
    facts.source_identities = Some(identities.into_boxed_slice());
}

fn without_identities(facts: Facts, capture_identities: bool) -> engine::Parsed {
    let identities = if capture_identities {
        facts
            .symbols
            .iter()
            .map(|symbol| {
                (symbol.kind() == crate::domain::facts::SymKind::Heading)
                    .then(|| symbol.name.to_string())
            })
            .collect()
    } else {
        Vec::new()
    };
    engine::Parsed {
        facts,
        identities,
        ..engine::Parsed::default()
    }
}

pub fn count_lines(content: &[u8]) -> u32 {
    let newlines = content.iter().filter(|&&byte| byte == b'\n').count();
    let trailing = usize::from(!content.is_empty() && content.last() != Some(&b'\n'));
    u32::try_from(newlines + trailing).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage_of(path: &str, file_type: FileType, content: &str) -> Coverage {
        let record = FileRecord {
            path: path.to_owned(),
            size: content.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(file_type),
        };
        extract(&record, content.as_bytes()).coverage
    }

    #[test]
    fn coverage_records_parse_errors_and_spares_own_extractors() {
        assert_eq!(
            coverage_of("a.go", FileType::Go, "package a\nfunc A() {}\n"),
            Coverage::Complete
        );
        assert_eq!(
            coverage_of("a.go", FileType::Go, "package a\nfunc A( {\n"),
            Coverage::ParseErrors
        );
        // SQL and Terraform have their own extractors, so no grammar is missing.
        assert_eq!(
            coverage_of("a.sql", FileType::Sql, "create table t (id int);\n"),
            Coverage::Complete
        );
        assert_eq!(
            coverage_of("a.tf", FileType::Terraform, "resource \"a\" \"b\" {}\n"),
            Coverage::Complete
        );
    }

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
    fn embedded_sql_uses_host_comment_boundaries() {
        for (path, file_type, source) in [
            (
                "query.go",
                FileType::Go,
                "package query\n// \"SELECT id FROM decoy\"\n/* unmatched \" */\nfunc load() { db.Query(\"SELECT id FROM users\") }\n",
            ),
            (
                "query.rs",
                FileType::Rust,
                "// \"SELECT id FROM decoy\"\n/* unmatched \" */\nfn load() { let _query = r#\"SELECT id FROM users\"#; }\n",
            ),
            (
                "query.py",
                FileType::Python,
                "# \"SELECT id FROM decoy\"\n# unmatched \"\ndef load():\n    return \"SELECT id FROM users\"\n",
            ),
            (
                "query.ts",
                FileType::TypeScript,
                "// \"SELECT id FROM decoy\"\n/* unmatched \" */\nfunction load() { return `SELECT id FROM users`; }\n",
            ),
            (
                "query.js",
                FileType::JavaScript,
                "// \"SELECT id FROM decoy\"\n/* unmatched \" */\nfunction load() { return 'SELECT id FROM users'; }\n",
            ),
            (
                "query.sh",
                FileType::Bash,
                "# \"SELECT id FROM decoy\"\n# unmatched \"\nload() { echo \"SELECT id FROM users\"; }\n",
            ),
        ] {
            let record = FileRecord {
                path: path.to_owned(),
                size: source.len() as u64,
                mtime_ns: 0,
                identity: None,
                hash: 0,
                role: Role::Src,
                file_type: Some(file_type),
            };
            let facts = extract(&record, source.as_bytes());
            assert_eq!(facts.coverage, Coverage::Complete, "{path}");
            let queries: Vec<_> = facts
                .refs
                .iter()
                .filter(|reference| reference.kind == crate::domain::facts::RefKind::Query)
                .map(|reference| reference.name.as_str())
                .collect();
            assert_eq!(queries, ["users"], "{path}");
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
            identity: None,
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

    #[test]
    fn bounded_csv_search_does_not_claim_complete_coverage() {
        let content = format!("{}tailquokkaz\n", "firstrow\n".repeat(CSV_INDEXED_ROWS));
        let record = FileRecord {
            path: "rows.csv".to_owned(),
            size: content.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Doc,
            file_type: Some(FileType::Csv),
        };
        let facts = extract(&record, content.as_bytes());
        assert_eq!(facts.coverage, Coverage::MatchLimit);
        assert!(
            !facts
                .symbols
                .iter()
                .any(|symbol| symbol.terms.contains("tailquokkaz"))
        );
        let complete = extract(&record, b"firstrow\ntailquokkaz\n");
        assert_eq!(complete.coverage, Coverage::Complete);
        assert!(
            complete
                .symbols
                .iter()
                .any(|symbol| symbol.terms.contains("tailquokkaz"))
        );
    }

    #[test]
    fn typescript_project_scope_is_persisted_with_manifest_facts() {
        let record = FileRecord {
            path: "tsconfig.json".to_owned(),
            size: 0,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Manifest,
            file_type: Some(FileType::Json),
        };
        let facts = extract(
            &record,
            br#"{"files":["a.ts","b.ts"],"include":["src/**/*.ts"],"exclude":["generated"],"compilerOptions":{"outDir":"dist","moduleDetection":"legacy"}}"#,
        );
        let identity = facts.source_identity(
            crate::domain::facts::SourceIdentityKind::TypeScriptProject,
            0,
        );
        let config = identity.and_then(crate::domain::facts::typescript_project_config);
        assert_eq!(
            config,
            Some(crate::domain::facts::TypeScriptProjectConfig {
                files: Some(vec!["a.ts".to_owned(), "b.ts".to_owned()]),
                include: Some(vec!["src/**/*.ts".to_owned()]),
                exclude: Some(vec!["generated".to_owned()]),
                out_dir: Some("dist".to_owned()),
                module_detection: Some("legacy".to_owned()),
                module: None,
                jsx: None,
            })
        );

        let unsupported = extract(&record, br#"{"extends":"../base/tsconfig.json"}"#);
        assert_eq!(
            unsupported.source_identity(
                crate::domain::facts::SourceIdentityKind::TypeScriptProject,
                0
            ),
            Some("unproven")
        );
    }

    #[test]
    fn typescript_package_type_is_persisted_with_manifest_facts() {
        let record = |source: &str| {
            let record = FileRecord {
                path: "package.json".to_owned(),
                size: 0,
                mtime_ns: 0,
                identity: None,
                hash: 0,
                role: Role::Manifest,
                file_type: Some(FileType::Json),
            };
            extract(&record, source.as_bytes())
        };
        let module = record(r#"{"type":"module"}"#);
        assert_eq!(
            module.source_identity(
                crate::domain::facts::SourceIdentityKind::TypeScriptPackageType,
                0
            ),
            Some("typescript_package:v1:module")
        );

        let absent = record(r#"{"name":"package"}"#);
        assert_eq!(
            absent.source_identity(
                crate::domain::facts::SourceIdentityKind::TypeScriptPackageType,
                0
            ),
            Some("typescript_package:v1:absent")
        );

        let invalid = record(r#"{"type":true}"#);
        assert_eq!(
            invalid.source_identity(
                crate::domain::facts::SourceIdentityKind::TypeScriptPackageType,
                0
            ),
            Some("unproven")
        );
    }

    #[test]
    fn html_extracts_only_enabled_inline_javascript_at_original_lines() {
        let record = FileRecord {
            path: "docs/page.html".to_owned(),
            size: 0,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Doc,
            file_type: Some(FileType::Html),
        };
        let source = concat!(
            "<!doctype html>\n",
            "<h1>Guide</h1>\n",
            "<!--\n",
            "<script>function commentOnly() { return ghost(); }</script>\n",
            "-->\n",
            "<style>\n",
            "<script>function styleOnly() { return ghost(); }</script>\n",
            "</style>\n",
            "<textarea>\n",
            "<script>function textOnly() { return ghost(); }</script>\n",
            "</textarea>\n",
            "<template>\n",
            "<script>function templateOnly() { return ghost(); }</script>\n",
            "</template>\n",
            "<script type=\"module\">\n",
            "function hydrate() {\n",
            "  return render();\n",
            "}\n",
            "</script>\n",
            "<script type=\"application/json\">\n",
            "function dataOnly() { return fake(); }\n",
            "</script>\n",
            "<script src=\"external.js\">\n",
            "function externalOnly() { return remote(); }\n",
            "</script>\n",
            "<script nomodule>\n",
            "function legacyOnly() { return legacy(); }\n",
            "</script>\n",
            "<script type=\"text/javascript; charset=utf-8\">\n",
            "function classic() { return hydrate(); }\n",
            "</script>\n",
            "<script language=\"javascript\">\n",
            "function legacyLanguage() { return classic(); }\n",
            "</script>\n",
            "<!DOCTYPE html PUBLIC \"<script>function doctypeOnly() { return fake(); }</script>\" \"x\">\n",
            "<![CDATA[\n",
            "<script>function cdataOnly() { return fake(); }</script>\n",
            "]]>\n",
            "<noscript>\n",
            "<script>function noScriptOnly() { return fake(); }</script>\n",
            "</noscript>\n",
            "<script>\n",
            "function malformed() { return unfinished(); }\n"
        );
        let parsed = extract_with_identities(&record, source.as_bytes());
        let symbols: Vec<_> = parsed
            .facts
            .symbols
            .iter()
            .map(|symbol| (symbol.name.as_ref(), symbol.line))
            .collect();
        assert_eq!(
            symbols,
            [
                ("Guide", 2),
                ("hydrate", 16),
                ("classic", 30),
                ("legacyLanguage", 33),
            ]
        );
        let calls: Vec<_> = parsed
            .facts
            .refs
            .iter()
            .filter(|reference| reference.kind == crate::domain::facts::RefKind::Call)
            .map(|reference| (reference.name.as_str(), reference.line))
            .collect();
        assert_eq!(calls, [("render", 17), ("hydrate", 30), ("classic", 33)]);
        let function_lines: Vec<_> = parsed
            .facts
            .symbols
            .iter()
            .filter(|symbol| symbol.kind() == crate::domain::facts::SymKind::Fn)
            .map(|symbol| (symbol.name.as_ref(), symbol.line, symbol.end_line))
            .collect();
        assert_eq!(
            function_lines,
            [
                ("hydrate", 16, 18),
                ("classic", 30, 30),
                ("legacyLanguage", 33, 33),
            ]
        );
        let call_scopes: Vec<_> = parsed
            .facts
            .refs
            .iter()
            .filter(|reference| reference.kind == crate::domain::facts::RefKind::Call)
            .map(|reference| {
                (
                    reference.name.as_str(),
                    reference
                        .scope
                        .map(|index| parsed.facts.symbols[index as usize].name.as_ref()),
                )
            })
            .collect();
        assert_eq!(
            call_scopes,
            [
                ("render", Some("hydrate")),
                ("hydrate", Some("classic")),
                ("classic", Some("legacyLanguage")),
            ]
        );
        assert_eq!(parsed.identities.len(), parsed.facts.symbols.len());
        assert_eq!(parsed.identities[0].as_deref(), Some("Guide"));
        assert!(parsed.identities[1..].iter().all(Option::is_some));
        assert!(!parsed.incomplete);
        for (name, term) in [
            ("hydrate", "render"),
            ("classic", "hydrat"),
            ("legacyLanguage", "classic"),
        ] {
            assert!(
                parsed.facts.symbols.iter().any(|symbol| {
                    *symbol.name == *name
                        && symbol
                            .terms
                            .split_whitespace()
                            .any(|indexed| indexed == term)
                }),
                "{name} is missing body term {term}"
            );
        }
        assert!(!parsed.facts.terms.contains("render"));
        let facts = extract(&record, source.as_bytes());
        assert_eq!(facts.symbols, parsed.facts.symbols);
        assert_eq!(facts.refs, parsed.facts.refs);
    }

    #[cfg(feature = "lang-bash")]
    #[test]
    fn bash_getopts_options_have_stable_diff_identities_and_exact_trap_calls()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::domain::facts::{RefKind, SymKind};

        let source = include_bytes!("../../tests/fixtures/languages/bash/scripts/options.sh");
        let record = FileRecord {
            path: "src/options.sh".to_owned(),
            size: source.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::Bash),
        };
        let parsed = extract_with_identities(&record, source);
        assert!(!parsed.incomplete);
        assert_eq!(parsed.identities.len(), parsed.facts.symbols.len());

        let options: Vec<_> = parsed
            .facts
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| symbol.kind() == SymKind::Option)
            .map(|(index, symbol)| {
                let identity = parsed
                    .identities
                    .get(index)
                    .and_then(Option::as_deref)
                    .ok_or("Bash option has no diff identity")?;
                Ok((symbol.name.as_ref(), symbol.signature.as_ref(), identity))
            })
            .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
        assert_eq!(
            options,
            [
                ("-a", "arg=none;case=16", "arg=none;handler=explicit"),
                (
                    "-b",
                    "arg=required;case=17",
                    "arg=required;handler=explicit"
                ),
                ("-c", "arg=none;case=18", "arg=none;handler=explicit"),
                ("-d", "arg=none;case=*:21", "arg=none;handler=fallback"),
                ("-z", "arg=none;case=none", "arg=none;handler=none"),
                ("-x", "arg=none;case=*:36", "arg=none;handler=fallback"),
                ("-y", "arg=none;case=*:36", "arg=none;handler=fallback"),
                (
                    "-m",
                    "arg=none;case=ambiguous",
                    "arg=none;handler=ambiguous"
                ),
                (
                    "-n",
                    "arg=none;case=ambiguous",
                    "arg=none;handler=ambiguous"
                ),
            ]
        );
        let trap_calls: Vec<_> = parsed
            .facts
            .refs
            .iter()
            .filter(|reference| reference.kind == RefKind::Call && reference.name == "cleanup")
            .map(|reference| reference.line)
            .collect();
        assert_eq!(trap_calls, [8, 9, 10]);
        assert!(!parsed.facts.refs.iter().any(|reference| {
            reference.kind == RefKind::Call
                && ["EXIT", "INT", "TERM", "HUP"].contains(&reference.name.as_str())
        }));
        assert!(
            !parsed
                .facts
                .refs
                .iter()
                .any(|reference| reference.kind == RefKind::Call && reference.name == "getopts")
        );
        assert!(!parsed.facts.refs.iter().any(|reference| {
            reference.kind == RefKind::Call
                && reference.name == "on_verbose"
                && reference.line == 11
        }));
        Ok(())
    }
}
