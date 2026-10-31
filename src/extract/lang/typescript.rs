//! TypeScript and JavaScript: `export` visibility, ES module and CommonJS imports, JSX
//! component usage; `.tsx` uses the TSX grammar.

use super::Pack;
use crate::domain::facts::{Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, unquote};

pub const HOOKS: Hooks = Hooks {
    exported,
    signature_prefix,
    flags: |_: &Definition, _: &FileContext| 0,
    expand_import,
    builtin,
};

#[cfg(feature = "lang-typescript")]
pub static TYPESCRIPT: Pack = Pack {
    file_type: FileType::TypeScript,
    suffixes: &[],
    language: |_| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    query: include_str!("../queries/typescript.scm"),
    hooks: HOOKS,
};

#[cfg(feature = "lang-typescript")]
pub static TSX: Pack = Pack {
    file_type: FileType::TypeScript,
    suffixes: &[".tsx"],
    language: |_| tree_sitter_typescript::LANGUAGE_TSX.into(),
    query: concat!(
        include_str!("../queries/typescript.scm"),
        include_str!("../queries/jsx.scm")
    ),
    hooks: HOOKS,
};

#[cfg(feature = "lang-javascript")]
pub static JAVASCRIPT: Pack = Pack {
    file_type: FileType::JavaScript,
    suffixes: &[],
    language: |_| tree_sitter_javascript::LANGUAGE.into(),
    query: concat!(
        include_str!("../queries/javascript.scm"),
        include_str!("../queries/jsx.scm")
    ),
    hooks: HOOKS,
};

/// `export`ed declarations, plus members of exported classes and interfaces that are not
/// `private`, `protected` or `#private`.
fn exported(definition: &Definition) -> bool {
    // `exports.f = function`, `app.f = function`: reachable through the object.
    if definition.export_wrapper || definition.node_kind == "assignment_expression" {
        return true;
    }
    let member_of_exported = matches!(
        definition.parent,
        Some((SymKind::Class | SymKind::Iface, true))
    );
    member_of_exported
        && !definition.name.starts_with('#')
        && !definition.text.starts_with("private")
        && !definition.text.starts_with("protected")
}

fn signature_prefix(kind: SymKind, text: &str) -> &'static str {
    match kind {
        // `exports.f = function` reads as written.
        SymKind::Const | SymKind::Fn
            if !text.starts_with("function")
                && !text.starts_with("async")
                && !text
                    .split(" = ")
                    .next()
                    .is_some_and(|left| left.contains('.')) =>
        {
            "const "
        }
        _ => "",
    }
}

/// One import per bound name: `import D, {a as b} from "m"`, `import * as ns from "m"`,
/// `export {x} from "m"`, `export * from "m"`. `names` holds the imported name (`default`
/// for default imports, `*` for namespaces and star re-exports), `alias` the local name.
fn expand_import(statement: &str, line: u32, _reexport: bool) -> Vec<Import> {
    let text: String = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    let reexport = text.starts_with("export");
    let Some(spec) = text
        .rsplit(" from ")
        .next()
        .filter(|_| text.contains(" from "))
        .or_else(|| text.strip_prefix("import "))
    else {
        return Vec::new();
    };
    let spec = unquote(spec.trim_end_matches(';').trim()).to_owned();
    let import = |name: &str, alias: Option<&str>| Import {
        spec: spec.clone(),
        alias: alias.map(str::to_owned),
        names: vec![name.to_owned()],
        line,
        reexport,
    };
    let Some((head, _)) = text.split_once(" from ") else {
        // Side-effect import: `import "./polyfill"`.
        return vec![Import {
            spec,
            line,
            ..Import::default()
        }];
    };
    let clause = head
        .trim_start_matches("export")
        .trim_start()
        .trim_start_matches("import")
        .trim_start()
        .trim_start_matches("type ")
        .trim();
    let mut imports = Vec::new();
    let (outside, braces) = match (clause.find('{'), clause.rfind('}')) {
        (Some(open), Some(close)) if open < close => (
            format!("{}{}", &clause[..open], &clause[close + 1..]),
            Some(&clause[open + 1..close]),
        ),
        _ => (clause.to_owned(), None),
    };
    for part in outside
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some(alias) = part.strip_prefix("* as ") {
            imports.push(import("*", Some(alias.trim())));
        } else if part == "*" {
            imports.push(import("*", None));
        } else {
            imports.push(import("default", Some(part)));
        }
    }
    for part in braces
        .into_iter()
        .flat_map(|inner| inner.split(','))
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let part = part.trim_start_matches("type ").trim();
        match part.split_once(" as ") {
            Some((name, alias)) => imports.push(import(name.trim(), Some(alias.trim()))),
            None => imports.push(import(part, None)),
        }
    }
    imports
}

fn builtin(name: &str) -> bool {
    matches!(
        name,
        "console"
            | "require"
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "Promise"
            | "Object"
            | "Array"
            | "String"
            | "Number"
            | "Boolean"
            | "JSON"
            | "Math"
            | "Date"
            | "Error"
            | "TypeError"
            | "Map"
            | "Set"
            | "Symbol"
            | "parseInt"
            | "parseFloat"
            | "fetch"
            | "Record"
            | "Partial"
            | "Readonly"
            | "Pick"
            | "Omit"
            | "ReturnType"
            | "Awaited"
            | "string"
            | "number"
            | "boolean"
            | "any"
            | "unknown"
            | "void"
            | "never"
            | "undefined"
            | "null"
            | "describe"
            | "it"
            | "test"
            | "expect"
            | "beforeEach"
            | "afterEach"
            | "beforeAll"
            | "afterAll"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(statement: &str) -> Vec<(String, String, Option<String>, bool)> {
        expand_import(statement, 1, false)
            .into_iter()
            .map(|import| {
                (
                    import.spec,
                    import.names.join(","),
                    import.alias,
                    import.reexport,
                )
            })
            .collect()
    }

    fn row(
        spec: &str,
        name: &str,
        alias: Option<&str>,
        reexport: bool,
    ) -> (String, String, Option<String>, bool) {
        (
            spec.to_owned(),
            name.to_owned(),
            alias.map(str::to_owned),
            reexport,
        )
    }

    #[test]
    fn import_and_export_clauses_expand_per_binding() {
        assert_eq!(
            rows("import Store, { open as openStore, type Row } from \"./store\";"),
            [
                row("./store", "default", Some("Store"), false),
                row("./store", "open", Some("openStore"), false),
                row("./store", "Row", None, false)
            ]
        );
        assert_eq!(
            rows("import * as db from '../db'"),
            [row("../db", "*", Some("db"), false)]
        );
        assert_eq!(
            rows("export { a, b as c } from './x'"),
            [
                row("./x", "a", None, true),
                row("./x", "b", Some("c"), true)
            ]
        );
        assert_eq!(rows("export * from \"./y\""), [row("./y", "*", None, true)]);
        assert_eq!(
            rows("import \"./polyfill\";"),
            [row("./polyfill", "", None, false)]
        );
    }
}
