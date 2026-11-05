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

/// Callee whose result is assigned by `with`, rather than the manager expression itself.
pub(in crate::extract) fn context_manager_callee(
    item: Node,
    callee: Node,
    source: &[u8],
) -> Option<String> {
    let statement = item.parent()?.parent()?;
    if item.kind() != "with_item" || statement.kind() != "with_statement" {
        return None;
    }
    let mut cursor = statement.walk();
    let asynchronous = statement
        .children(&mut cursor)
        .any(|node| node.kind() == "async");
    let method = if asynchronous {
        "__aenter__"
    } else {
        "__enter__"
    };
    let call = callee
        .parent()
        .is_some_and(|parent| parent.kind() == "call");
    let callee = callee.utf8_text(source).ok()?;
    Some(format!("{callee}{}.{method}", if call { "()" } else { "" }))
}

/// Keep protocol return annotations intact so unions cannot become their first branch.
pub(in crate::extract) fn context_manager_result(node: Node, source: &[u8]) -> Option<String> {
    let function = node.parent()?;
    if function.kind() != "function_definition"
        || !matches!(
            function
                .child_by_field_name("name")?
                .utf8_text(source)
                .ok()?,
            "__enter__" | "__aenter__"
        )
    {
        return None;
    }
    if returned_receiver(function, source).is_some() {
        return Some("Self".to_owned());
    }
    let text = node.utf8_text(source).ok()?;
    let unquoted = text
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .or_else(|| {
            text.strip_prefix('\'')
                .and_then(|text| text.strip_suffix('\''))
        })
        .filter(|text| !text.contains('\\'))
        .unwrap_or(text);
    Some(crate::domain::facts::normalize_space(unquoted))
}

/// A successful exit returns the original receiver: no alternative return or local rebinding.
fn returned_receiver<'source>(function: Node, source: &'source [u8]) -> Option<&'source str> {
    if function.has_error() || function.parent()?.parent()?.kind() != "class_definition" {
        return None;
    }
    let parameter = function.child_by_field_name("parameters")?.named_child(0)?;
    let parameter = match parameter.kind() {
        "identifier" => parameter,
        "typed_parameter" => parameter.named_child(0)?,
        _ => return None,
    };
    if parameter.kind() != "identifier" {
        return None;
    }
    let receiver = parameter.utf8_text(source).ok()?;
    let body = function.child_by_field_name("body")?;
    let mut cursor = body.walk();
    if body
        .named_children(&mut cursor)
        .filter(|node| node.kind() != "comment")
        .last()?
        .kind()
        != "return_statement"
    {
        return None;
    }
    let mut cursor = body.walk();
    loop {
        let node = cursor.node();
        match node.kind() {
            "return_statement" => {
                let value = node.named_child(0)?;
                if node.named_child_count() != 1
                    || value.kind() != "identifier"
                    || value.utf8_text(source).ok()? != receiver
                {
                    return None;
                }
            }
            "function_definition"
            | "class_definition"
            | "lambda"
            | "yield"
            | "import_statement"
            | "import_from_statement"
            | "global_statement"
            | "nonlocal_statement"
            | "delete_statement"
            | "match_statement"
            | "except_clause" => return None,
            _ => {}
        }
        let target = match node.kind() {
            "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => {
                node.child_by_field_name("left")
            }
            "named_expression" => node.child_by_field_name("name"),
            "as_pattern_target" => Some(node),
            _ => None,
        };
        if target.is_some_and(|target| target_changes_receiver(target, receiver, source)) {
            return None;
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return Some(receiver);
            }
        }
    }
}

fn target_changes_receiver(node: Node, name: &str, source: &[u8]) -> bool {
    match node.kind() {
        "identifier" => node.utf8_text(source).ok() == Some(name),
        "attribute" => {
            node.child_by_field_name("attribute")
                .and_then(|attribute| attribute.utf8_text(source).ok())
                == Some("__class__")
        }
        "subscript" => false,
        _ => node
            .named_children(&mut node.walk())
            .any(|child| target_changes_receiver(child, name, source)),
    }
}

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

    #[test]
    fn context_bindings_use_the_protocol_result_instead_of_the_manager() -> std::io::Result<()> {
        use crate::domain::graph::{Confidence, EdgeKind};
        use crate::infra::testdir::TestDir;

        let root = TestDir::new("python-context-values")?;
        root.write("contexts.py", concat!(
            "from typing import Self, TypeVar\n",
            "R = TypeVar('R', bound='GenericManager')\nS = TypeVar('S', bound='GenericAsync')\n",
            "W = TypeVar('W', bound='ReboundManager')\nM = TypeVar('M', bound='MutatingManager')\n",
            "class Resource:\n    def consume(self): return 1\n",
            "    @property\n    def closed(self) -> bool: return False\n",
            "    @property\n    def auth(self) -> str: return self._auth\n",
            "    @auth.setter\n    def auth(self, value: str): self._auth = value\n",
            "    def unknown_only(self): return 1\n    def union_only(self): return 1\n",
            "    def rebound_only(self): return 1\n    def mutated_only(self): return 1\n",
            "class Manager:\n",
            "    def __enter__(self) -> Resource: return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 2\n",
            "class Own:\n",
            "    def __enter__(self) -> Self: return self\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 3\n",
            "    def union_only(self): return 3\n",
            "class AsyncManager:\n",
            "    async def __aenter__(self) -> Resource: return Resource()\n",
            "    async def __aexit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 4\n",
            "class UnknownManager:\n",
            "    def __enter__(self): return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 5\n",
            "    def unknown_only(self): return 5\n",
            "class QuotedManager:\n",
            "    def __enter__(self) -> 'Resource': return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 7\n",
            "class AmbiguousManager:\n",
            "    def __enter__(self) -> Resource | Own: return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 6\n",
            "    def union_only(self): return 6\n",
            "class GenericManager:\n",
            "    def __enter__(this: R) -> R:\n",
            "        this.state = 1\n        for piece in (1,): this.state += piece\n",
            "        if this.state < 0: raise ValueError('state')\n        return this\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 8\n",
            "class Derived(GenericManager):\n    def consume(self): return 9\n",
            "class GenericAsync:\n",
            "    async def __aenter__(self: S) -> S:\n        self.state = 1\n        return self\n",
            "    async def __aexit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 10\n",
            "class NominalManager(Resource):\n",
            "    def __enter__(self: Resource) -> Resource: return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 11\n",
            "class ReboundManager:\n",
            "    def __enter__(self: W) -> W:\n        self = Resource()\n        return self\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def rebound_only(self): return 12\n",
            "class MutatingManager:\n",
            "    def __enter__(self: M) -> M:\n        self.__class__ = Resource\n        return self\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def mutated_only(self): return 13\n",
            "class MixedManager:\n",
            "    def __init__(self): self.alternative = True\n",
            "    def __enter__(self) -> 'Resource | MixedManager':\n",
            "        if self.alternative: return Resource()\n        return self\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 14\n",
            "def acquire() -> Manager: return Manager()\n",
            "def direct():\n    with Manager() as value: return value.consume()\n",
            "def factory():\n    with acquire() as value: return value.consume()\n",
            "def existing(manager: Manager):\n    with manager as value: return value.consume()\n",
            "def own():\n    with Own() as value: return value.consume()\n",
            "def quoted():\n    with QuotedManager() as value: return value.consume()\n",
            "async def asynchronous():\n    async with AsyncManager() as value: return value.consume()\n",
            "def unknown():\n    with UnknownManager() as value: return value.unknown_only()\n",
            "def ambiguous():\n    with AmbiguousManager() as value: return value.union_only()\n",
            "def generic():\n    with GenericManager() as value: return value.consume()\n",
            "def derived():\n    with Derived() as value: return value.consume()\n",
            "async def generic_async():\n    async with GenericAsync() as value: return value.consume()\n",
            "def nominal():\n    with NominalManager() as value: return value.consume()\n",
            "def rebound():\n    with ReboundManager() as value: return value.rebound_only()\n",
            "def mutated():\n    with MutatingManager() as value: return value.mutated_only()\n",
            "def mixed():\n    with MixedManager() as value: return value.consume()\n",
            "def property_inside():\n    with Manager() as value: return value.closed\n",
            "def property_after():\n    with Manager() as value: value.consume()\n    return value.closed\n",
            "def ambiguous_property():\n    with Manager() as value:\n        value.auth = 'updated'\n        return value.auth\n",
        ))?;
        root.write("consumer.py", "import contexts as api\ndef qualified():\n    with api.Derived() as value: return value.consume()\n")?;
        let graph = crate::resolve::tests::graph_of(root.path())?;
        let mut calls: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls)
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                if !matches!(
                    target.name.as_str(),
                    "consume" | "rebound_only" | "mutated_only"
                ) {
                    return None;
                }
                let owner = graph.symbols.get(target.parent as usize)?;
                Some((source.name.as_str(), owner.name.as_str(), edge.confidence))
            })
            .collect();
        calls.sort_unstable();
        for name in [
            "unknown_only",
            "union_only",
            "rebound_only",
            "mutated_only",
            "auth",
        ] {
            assert!(
                graph
                    .ambiguous_names
                    .iter()
                    .any(|candidate| candidate == name),
                "{name}: unresolved context members must remain ambiguous"
            );
        }
        assert_eq!(
            calls,
            [
                ("asynchronous", "Resource", Confidence::Inferred),
                ("derived", "Derived", Confidence::Inferred),
                ("direct", "Resource", Confidence::Inferred),
                ("existing", "Resource", Confidence::Inferred),
                ("factory", "Resource", Confidence::Inferred),
                ("generic", "GenericManager", Confidence::Inferred),
                ("generic_async", "GenericAsync", Confidence::Inferred),
                ("nominal", "Resource", Confidence::Inferred),
                ("own", "Own", Confidence::Inferred),
                ("property_after", "Resource", Confidence::Inferred),
                ("qualified", "Derived", Confidence::Inferred),
                ("quoted", "Resource", Confidence::Inferred),
            ]
        );
        let mut properties: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Uses)
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                (target.name == "closed").then_some((source.name.as_str(), edge.confidence))
            })
            .collect();
        properties.sort_unstable();
        assert_eq!(
            properties,
            [
                ("property_after", Confidence::Inferred),
                ("property_inside", Confidence::Inferred),
            ]
        );
        Ok(())
    }

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
