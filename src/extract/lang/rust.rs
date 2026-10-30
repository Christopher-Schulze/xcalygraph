//! Rust: `pub` visibility, `#[test]` and `#[cfg(test)]` tests, `main` of bin targets, use trees.

use super::Pack;
use crate::domain::facts::{FLAG_ENTRY, FLAG_TEST, Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    file_type: FileType::Rust,
    language: |_| tree_sitter_rust::LANGUAGE.into(),
    query: include_str!("../queries/rust.scm"),
    hooks: Hooks {
        exported,
        signature_prefix: |_, _| "",
        flags,
        expand_import,
        builtin,
    },
};

/// Primitive types, prelude items and standard macros.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "Ok" | "Err"
            | "Some"
            | "None"
            | "Box"
            | "Vec"
            | "String"
            | "Option"
            | "Result"
            | "Self"
            | "println"
            | "print"
            | "eprintln"
            | "eprint"
            | "format"
            | "vec"
            | "assert"
            | "assert_eq"
            | "assert_ne"
            | "debug_assert"
            | "debug_assert_eq"
            | "debug_assert_ne"
            | "panic"
            | "todo"
            | "unimplemented"
            | "unreachable"
            | "write"
            | "writeln"
            | "matches"
            | "dbg"
            | "include_str"
            | "include_bytes"
            | "concat"
            | "env"
            | "option_env"
            | "cfg"
            | "format_args"
            | "drop"
            | "usize"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
            | "Send"
            | "Sync"
            | "Sized"
            | "Copy"
            | "Clone"
            | "Default"
            | "Debug"
            | "PartialEq"
            | "Eq"
            | "Hash"
            | "Ord"
            | "PartialOrd"
            | "From"
            | "Into"
            | "Iterator"
    )
}

/// `pub` items, plus methods of public traits and of trait implementations, which are as
/// visible as the trait.
fn exported(definition: &Definition) -> bool {
    definition.text.starts_with("pub ")
        || definition.text.starts_with("pub\n")
        || definition.in_trait_impl
        || matches!(definition.parent, Some((SymKind::Trait, true)))
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    let mut flags = 0;
    let attributes = definition.decorations;
    if attributes.contains("#[test]")
        || attributes.contains("::test]")
        || attributes.contains("#[rstest")
        || attributes.contains("#[cfg(test)]")
    {
        flags |= FLAG_TEST;
    }
    let bin = context.path.ends_with("src/main.rs")
        || context.path == "main.rs"
        || context.path.contains("src/bin/")
        || context.path.ends_with("build.rs");
    if definition.kind == SymKind::Fn
        && definition.name == "main"
        && bin
        && definition.inherited == 0
    {
        flags |= FLAG_ENTRY;
    }
    flags
}

/// Expands a use tree (`crate::a::{b::C, d as E, f::*}`) into one import per leaf: `spec` is
/// the path before the leaf, `names` the leaf (`*` for globs), `alias` the `as` name.
fn expand_import(tree: &str, line: u32, reexport: bool) -> Vec<Import> {
    let compact: String = tree.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut imports = Vec::new();
    expand(&compact, &[], line, reexport, &mut imports);
    imports
}

fn expand(tree: &str, prefix: &[String], line: u32, reexport: bool, out: &mut Vec<Import>) {
    let tree = tree.trim().trim_start_matches("::");
    if let Some(open) = tree.find('{') {
        let head = tree[..open].trim_end_matches("::");
        let inner = tree[open + 1..].trim_end().trim_end_matches('}');
        let mut path = prefix.to_vec();
        path.extend(segments(head));
        for item in split_top_level(inner) {
            expand(item, &path, line, reexport, out);
        }
        return;
    }
    let (path_text, alias) = match tree.split_once(" as ") {
        Some((path, alias)) => (path, Some(alias.trim().to_owned())),
        None => (tree, None),
    };
    let mut path = prefix.to_vec();
    path.extend(segments(path_text));
    if path.last().is_some_and(|last| last == "self") {
        path.pop();
    }
    let Some(name) = path.pop() else { return };
    out.push(Import {
        spec: path.join("::"),
        alias,
        names: vec![name],
        line,
        reexport,
    });
}

fn segments(path: &str) -> impl Iterator<Item = String> + '_ {
    path.split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
}

fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imports(tree: &str) -> Vec<(String, String, Option<String>)> {
        expand_import(tree, 1, false)
            .into_iter()
            .map(|import| (import.spec, import.names.join(","), import.alias))
            .collect()
    }

    fn row(spec: &str, name: &str, alias: Option<&str>) -> (String, String, Option<String>) {
        (spec.to_owned(), name.to_owned(), alias.map(str::to_owned))
    }

    #[test]
    fn use_trees_expand_to_leaves() {
        assert_eq!(imports("std::io"), [row("std", "io", None)]);
        assert_eq!(
            imports("crate::a::{b::C, d as E, f::*, self}"),
            [
                row("crate::a::b", "C", None),
                row("crate::a", "d", Some("E")),
                row("crate::a::f", "*", None),
                row("crate", "a", None),
            ]
        );
        assert_eq!(
            imports("super::{x::{y, z}, w}"),
            [
                row("super::x", "y", None),
                row("super::x", "z", None),
                row("super", "w", None)
            ]
        );
        assert_eq!(imports("serde"), [row("", "serde", None)]);
    }
}
