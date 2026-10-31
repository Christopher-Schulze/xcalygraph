//! Python: leading-underscore privacy, `test_*` and `Test*` tests, `__main__` entry, import
//! statements expanded per name.

use super::Pack;
use crate::domain::facts::{FLAG_TEST, Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Python,
    language: |_| tree_sitter_python::LANGUAGE.into(),
    query: include_str!("../queries/python.scm"),
    hooks: Hooks {
        exported: |definition| !definition.name.starts_with('_'),
        signature_prefix: |_, _| "",
        flags,
        expand_import,
        builtin,
    },
};

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    // A registering decorator (`@app.route`, `@pytest.fixture`, `@click.command`) means the
    // framework calls the function: an entrypoint, never dead.
    let registered = definition
        .decorations
        .split('@')
        .map(str::trim)
        .filter(|decorator| !decorator.is_empty())
        .any(|decorator| {
            let name = decorator.split(['(', ' ']).next().unwrap_or_default();
            !matches!(
                name,
                "property" | "staticmethod" | "classmethod" | "abstractmethod" | "override"
            ) && !name.ends_with(".setter")
                && !name.ends_with(".getter")
        });
    let entry = if registered {
        crate::domain::facts::FLAG_ENTRY
    } else {
        0
    };
    let test_file = crate::domain::role::is_test_path(FileType::Python, context.path);
    let test_name = (matches!(definition.kind, SymKind::Fn | SymKind::Method)
        && definition.name.starts_with("test"))
        || (definition.kind == SymKind::Class && definition.name.starts_with("Test"));
    if test_name && (test_file || matches!(definition.parent, Some((SymKind::Class, _)))) {
        FLAG_TEST | entry
    } else {
        entry
    }
}

/// `import a.b as c, d` and `from .x import y as z, w` into one import per name: `spec` is the
/// module path (leading dots kept for relative imports), `names` the imported name (empty for
/// whole-module imports), `alias` the `as` name.
fn expand_import(statement: &str, line: u32, _reexport: bool) -> Vec<Import> {
    let text: String = statement
        .replace(['(', ')', '\\'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let split_alias = |part: &str| -> (String, Option<String>) {
        match part.trim().split_once(" as ") {
            Some((name, alias)) => (name.trim().to_owned(), Some(alias.trim().to_owned())),
            None => (part.trim().to_owned(), None),
        }
    };
    if let Some(rest) = text.strip_prefix("from ") {
        let Some((module, names)) = rest.split_once(" import ") else {
            return Vec::new();
        };
        return names
            .split(',')
            .filter(|part| !part.trim().is_empty())
            .map(|part| {
                let (name, alias) = split_alias(part);
                Import {
                    spec: module.trim().to_owned(),
                    alias,
                    names: vec![name],
                    line,
                    reexport: false,
                }
            })
            .collect();
    }
    let Some(rest) = text.strip_prefix("import ") else {
        return Vec::new();
    };
    rest.split(',')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let (module, alias) = split_alias(part);
            Import {
                spec: module,
                alias,
                names: Vec::new(),
                line,
                reexport: false,
            }
        })
        .collect()
}

/// Builtin functions and types.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "len"
            | "range"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "isinstance"
            | "issubclass"
            | "super"
            | "open"
            | "enumerate"
            | "zip"
            | "map"
            | "filter"
            | "sorted"
            | "reversed"
            | "min"
            | "max"
            | "sum"
            | "any"
            | "all"
            | "getattr"
            | "setattr"
            | "hasattr"
            | "type"
            | "object"
            | "Exception"
            | "ValueError"
            | "TypeError"
            | "KeyError"
            | "RuntimeError"
            | "repr"
            | "iter"
            | "next"
            | "abs"
            | "round"
            | "format"
            | "id"
            | "hash"
            | "bytes"
            | "callable"
            | "vars"
            | "dir"
            | "staticmethod"
            | "classmethod"
            | "property"
            | "NotImplementedError"
            | "None"
            | "Optional"
            | "List"
            | "Dict"
            | "Any"
            | "Union"
            | "Tuple"
            | "Set"
            | "Callable"
            | "Iterable"
            | "Iterator"
            | "Sequence"
            | "Mapping"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(statement: &str) -> Vec<(String, String, Option<String>)> {
        expand_import(statement, 1, false)
            .into_iter()
            .map(|import| (import.spec, import.names.join(","), import.alias))
            .collect()
    }

    #[test]
    fn import_statements_expand_per_name() {
        assert_eq!(
            rows("import os, a.b as c"),
            [
                ("os".to_owned(), String::new(), None),
                ("a.b".to_owned(), String::new(), Some("c".to_owned()))
            ]
        );
        assert_eq!(
            rows("from ..pkg.mod import (x as y,\n    z)"),
            [
                ("..pkg.mod".to_owned(), "x".to_owned(), Some("y".to_owned())),
                ("..pkg.mod".to_owned(), "z".to_owned(), None),
            ]
        );
        assert_eq!(
            rows("from . import util"),
            [(".".to_owned(), "util".to_owned(), None)]
        );
        assert_eq!(
            rows("from x import *"),
            [("x".to_owned(), "*".to_owned(), None)]
        );
    }
}
