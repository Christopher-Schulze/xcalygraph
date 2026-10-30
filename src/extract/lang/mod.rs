//! Language packs: grammar, query and hooks per deep language (`docs/decisions.md` D16).

#[cfg(feature = "lang-go")]
pub mod go;
#[cfg(feature = "lang-rust")]
pub mod rust;

use std::sync::OnceLock;

use tree_sitter::{Language, Query};

use super::engine::Hooks;
use crate::domain::filetype::FileType;

pub struct Pack {
    pub file_type: FileType,
    /// Grammar for a file path (some languages switch dialects by suffix).
    pub language: fn(&str) -> Language,
    pub query: &'static str,
    pub hooks: Hooks,
}

const PACKS: &[&Pack] = &[
    #[cfg(feature = "lang-go")]
    &go::PACK,
    #[cfg(feature = "lang-rust")]
    &rust::PACK,
];

static QUERIES: OnceLock<Vec<Option<Query>>> = OnceLock::new();

/// Pack and compiled query for a file type; `None` when the grammar is not compiled in or its
/// query fails to compile (covered by tests).
pub fn lookup(
    file_type: FileType,
    path: &str,
) -> Option<(&'static Pack, Language, &'static Query)> {
    let index = PACKS.iter().position(|pack| pack.file_type == file_type)?;
    let queries = QUERIES.get_or_init(|| {
        PACKS
            .iter()
            .map(|pack| Query::new(&(pack.language)(""), pack.query).ok())
            .collect()
    });
    let query = queries.get(index)?.as_ref()?;
    let pack = PACKS[index];
    Some((pack, (pack.language)(path), query))
}

#[cfg(test)]
pub fn packs() -> &'static [&'static Pack] {
    PACKS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_query_compiles() {
        for pack in packs() {
            let compiled = Query::new(&(pack.language)(""), pack.query);
            assert!(
                compiled.is_ok(),
                "{}: {:?}",
                pack.file_type.id(),
                compiled.err()
            );
        }
    }
}
