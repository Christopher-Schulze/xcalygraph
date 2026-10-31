//! Fact extraction: content bytes to per-file facts (`docs/spec.md` section 5). Pure; no IO.

mod config;
mod contracts;
pub mod engine;
#[cfg(test)]
mod golden;
mod hcl;
pub mod json;
pub mod lang;
pub mod manifest;
pub mod markdown;
pub mod sfc;
mod sql;
mod tasks;

use crate::domain::corpus::FileRecord;
use crate::domain::facts::Facts;
use crate::domain::filetype::FileType;
use crate::domain::role::{self, Role};

/// Bumped whenever extraction code changes output, so existing indexes rebuild.
const EXTRACTION_VERSION: u32 = 20;

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
    let mut facts = record
        .file_type
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

    #[test]
    fn lines_count_a_missing_final_newline() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a"), 1);
        assert_eq!(count_lines(b"a\n"), 1);
        assert_eq!(count_lines(b"a\nb"), 2);
    }
}
