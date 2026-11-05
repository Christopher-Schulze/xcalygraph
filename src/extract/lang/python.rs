//! Python: leading-underscore privacy, `test_*` and `Test*` tests, `__main__` entry, import
//! statements expanded per name.

use super::Pack;
use crate::domain::facts::{FLAG_TEST, Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, ImportExpansion};
use tree_sitter::Node;

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Python,
    language: |_| tree_sitter_python::LANGUAGE.into(),
    query: include_str!("../queries/python.scm"),
    hooks: Hooks {
        exported: |definition| !definition.name.starts_with('_'),
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(expand_import),
        builtin,
    },
};

/// Evidence for conservative structural protocol matching; decorated or dynamic class bodies are
/// intentionally excluded because their runtime members cannot be established from method names.
pub(in crate::extract) fn structural_identity(node: Node, source: &[u8]) -> Option<String> {
    if node.has_error() || node.is_missing() {
        return None;
    }
    if node.kind() == "class_definition" && node.parent()?.kind() == "module" {
        let body = node.child_by_field_name("body")?;
        let mut cursor = body.walk();
        let simple = body.named_children(&mut cursor).all(|child| {
            matches!(
                child.kind(),
                "function_definition" | "comment" | "pass_statement"
            ) || (child.kind() == "expression_statement"
                && child.named_child_count() == 1
                && child
                    .named_child(0)
                    .is_some_and(|item| item.kind() == "string"))
        });
        if !simple {
            return None;
        }
        if node.child_by_field_name("superclasses").is_none() {
            return Some("python:plain-class:".to_owned());
        }
        let module = node.parent()?;
        let mut cursor = module.walk();
        let lines = module
            .named_children(&mut cursor)
            .take_while(|child| child.start_byte() < node.start_byte())
            .filter(|child| child.kind() == "import_from_statement")
            .filter_map(|child| {
                let name = child
                    .child_by_field_name("module_name")?
                    .utf8_text(source)
                    .ok()?;
                matches!(name, "typing" | "typing_extensions")
                    .then(|| (child.start_position().row + 1).to_string())
            })
            .collect::<Vec<_>>()
            .join(",");
        return Some(format!("python:plain-class:{lines}"));
    }
    if node.kind() != "function_definition" {
        return None;
    }
    let block = node.parent()?;
    let class = block.parent()?;
    if block.kind() != "block"
        || class.kind() != "class_definition"
        || class.parent()?.kind() != "module"
    {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    let header = source.get(node.start_byte()..body.start_byte())?;
    if header.len() > 1024 {
        return None;
    }
    let identity = crate::extract::engine::signature_identity(node, source)?;
    Some(format!("python:method:{identity}"))
}

/// Detects a Python contract method whose complete executable body raises `NotImplementedError`.
pub(in crate::extract) fn is_abstract_stub(node: Node, source: &[u8]) -> bool {
    if node.kind() != "function_definition" || node.has_error() || node.is_missing() {
        return false;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    let mut found_raise = false;
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "comment" => {}
            "expression_statement" if !found_raise && is_docstring(child) => {}
            "raise_statement" if !found_raise && raises_not_implemented(child, source) => {
                found_raise = true;
            }
            _ => return false,
        }
    }
    found_raise
}

fn is_docstring(statement: Node) -> bool {
    statement.kind() == "expression_statement"
        && statement.named_child_count() == 1
        && statement
            .named_child(0)
            .is_some_and(|child| matches!(child.kind(), "string" | "concatenated_string"))
}

fn raises_not_implemented(statement: Node, source: &[u8]) -> bool {
    let mut cursor = statement.walk();
    let mut expressions = statement.named_children(&mut cursor);
    let Some(exception) = expressions.next() else {
        return false;
    };
    if expressions.next().is_some() {
        return false;
    }
    let exception_name = match exception.kind() {
        "identifier" => Some(exception),
        "call" => exception.child_by_field_name("function"),
        _ => None,
    };
    exception_name.is_some_and(|name| {
        name.kind() == "identifier"
            && name
                .utf8_text(source)
                .is_ok_and(|text| text == "NotImplementedError")
    })
}

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
                    type_only: false,
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
                type_only: false,
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
