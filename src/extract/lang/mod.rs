//! Language packs: grammar, query and hooks per deep language (`docs/decisions.md` D16).

#[cfg(feature = "lang-bash")]
pub mod bash;
#[cfg(any(feature = "lang-elixir", feature = "lang-erlang"))]
pub mod beam;
#[cfg(any(feature = "lang-c", feature = "lang-cpp", feature = "lang-objc"))]
pub mod c;
#[cfg(feature = "lang-cpp")]
pub mod cpp;
#[cfg(any(feature = "lang-csharp", feature = "lang-vbnet"))]
pub mod csharp;
#[cfg(feature = "lang-dart")]
pub mod dart;
#[cfg(feature = "lang-go")]
pub mod go;
#[cfg(feature = "lang-groovy")]
pub mod groovy;
#[cfg(any(
    feature = "lang-java",
    feature = "lang-kotlin",
    feature = "lang-scala",
    feature = "lang-groovy",
    feature = "lang-csharp",
    feature = "lang-vbnet",
    feature = "lang-swift",
    feature = "lang-php"
))]
pub mod java;
#[cfg(feature = "lang-kotlin")]
pub mod kotlin;
#[cfg(feature = "lang-lua")]
pub mod lua;
#[cfg(feature = "lang-objc")]
pub mod objc;
#[cfg(feature = "lang-php")]
pub mod php;
#[cfg(feature = "lang-python")]
pub mod python;
#[cfg(feature = "lang-ruby")]
pub mod ruby;
#[cfg(feature = "lang-rust")]
pub mod rust;
#[cfg(feature = "lang-scala")]
pub mod scala;
#[cfg(feature = "lang-swift")]
pub mod swift;
#[cfg(any(feature = "lang-typescript", feature = "lang-javascript"))]
pub mod typescript;
#[cfg(feature = "lang-vbnet")]
pub mod vbnet;
#[cfg(feature = "lang-zig")]
pub mod zig;

use std::sync::OnceLock;

use tree_sitter::{Language, Query};

use super::engine::Hooks;
use crate::domain::filetype::FileType;

pub struct Pack {
    pub file_type: FileType,
    /// Path suffixes this pack is restricted to (dialects such as `.tsx`); empty for all.
    pub suffixes: &'static [&'static str],
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
    #[cfg(feature = "lang-python")]
    &python::PACK,
    #[cfg(feature = "lang-typescript")]
    &typescript::TSX,
    #[cfg(feature = "lang-typescript")]
    &typescript::TYPESCRIPT,
    #[cfg(feature = "lang-javascript")]
    &typescript::JAVASCRIPT,
    #[cfg(feature = "lang-bash")]
    &bash::PACK,
    #[cfg(feature = "lang-java")]
    &java::PACK,
    #[cfg(feature = "lang-kotlin")]
    &kotlin::PACK,
    #[cfg(feature = "lang-scala")]
    &scala::PACK,
    #[cfg(feature = "lang-groovy")]
    &groovy::PACK,
    #[cfg(feature = "lang-csharp")]
    &csharp::PACK,
    #[cfg(feature = "lang-vbnet")]
    &vbnet::PACK,
    #[cfg(feature = "lang-c")]
    &c::PACK,
    #[cfg(feature = "lang-cpp")]
    &cpp::PACK,
    #[cfg(feature = "lang-objc")]
    &objc::PACK,
    #[cfg(feature = "lang-swift")]
    &swift::PACK,
    #[cfg(feature = "lang-ruby")]
    &ruby::PACK,
    #[cfg(feature = "lang-php")]
    &php::PACK,
    #[cfg(feature = "lang-dart")]
    &dart::PACK,
    #[cfg(feature = "lang-elixir")]
    &beam::ELIXIR,
    #[cfg(feature = "lang-erlang")]
    &beam::ERLANG,
    #[cfg(feature = "lang-lua")]
    &lua::PACK,
    #[cfg(feature = "lang-zig")]
    &zig::PACK,
];

/// One lazily compiled query per pack: a refresh compiles only the languages it extracts.
static QUERIES: [OnceLock<Option<Query>>; PACKS.len()] = [const { OnceLock::new() }; PACKS.len()];

/// Pack and compiled query for a file type; `None` when the grammar is not compiled in or its
/// query fails to compile (covered by tests).
pub fn lookup(
    file_type: FileType,
    path: &str,
) -> Option<(&'static Pack, Language, &'static Query)> {
    let index = PACKS.iter().position(|pack| {
        pack.file_type == file_type
            && (pack.suffixes.is_empty()
                || pack.suffixes.iter().any(|suffix| path.ends_with(suffix)))
    })?;
    let pack = PACKS[index];
    let query = QUERIES[index]
        .get_or_init(|| Query::new(&(pack.language)(""), pack.query).ok())
        .as_ref()?;
    Some((pack, (pack.language)(path), query))
}

pub fn all_packs() -> &'static [&'static Pack] {
    PACKS
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
