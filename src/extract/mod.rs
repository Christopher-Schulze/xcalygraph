//! Fact extraction: content bytes to per-file facts (`docs/spec.md` section 5). Pure; no IO.

pub mod engine;
#[cfg(test)]
mod golden;
pub mod lang;
pub mod manifest;

use crate::domain::corpus::FileRecord;
use crate::domain::facts::Facts;
use crate::domain::role::{self, Role};

/// Bumped whenever extraction output changes, so existing indexes rebuild.
pub const EXTRACTION_VERSION: u32 = 3;

/// Extracts the facts of one file of an extracted role.
pub fn extract(record: &FileRecord, content: &[u8]) -> Facts {
    let mut facts = record
        .file_type
        .and_then(|file_type| lang::lookup(file_type, &record.path))
        .map(|(pack, language, query)| {
            engine::extract(&language, query, &pack.hooks, &record.path, content)
        })
        .unwrap_or_default();
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
