//! TypeScript and JavaScript: `export` visibility, ES module and CommonJS imports, JSX
//! component usage; `.tsx` uses the TSX grammar.

use super::Pack;
use crate::domain::facts::{FLAG_DECLARATION, Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, unquote};

pub const HOOKS: Hooks = Hooks {
    exported,
    signature_prefix,
    flags,
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

/// Direct literal keys that need neither escape decoding nor ambiguous field-list rendering.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn simple_property_key(
    node: tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    let key = match node.kind() {
        "string" => {
            let quote = text.chars().next()?;
            if !matches!(quote, '\'' | '"') {
                return None;
            }
            text.strip_prefix(quote)?.strip_suffix(quote)?
        }
        "number" => {
            if !text.bytes().all(|byte| byte.is_ascii_digit())
                || (text.len() > 1 && text.starts_with('0'))
                || text.parse::<u64>().ok()? > 9_007_199_254_740_991
            {
                return None;
            }
            text
        }
        _ => return None,
    };
    (!key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '$' | '-')))
    .then(|| key.to_owned())
}

/// `export`ed declarations, plus members of exported classes and interfaces that are not
/// `private`, `protected` or `#private`.
/// Overload signatures (`function scale(x: number): number;`) declare the implementation that
/// follows them.
fn flags(definition: &Definition, _: &FileContext) -> u8 {
    if definition.node_kind == "function_signature" {
        FLAG_DECLARATION
    } else {
        0
    }
}

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

    #[test]
    fn interface_properties_keep_their_owner_and_declared_types() {
        use crate::domain::corpus::FileRecord;
        use crate::domain::role::Role;

        let source = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/typescript-interface-properties.ts"
        ));
        let record = FileRecord {
            path: "interface-properties.ts".to_owned(),
            size: source.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::TypeScript),
        };
        let facts = crate::extract::extract(&record, source);
        let fields = |name: &str| {
            facts
                .fields
                .iter()
                .find(|fields| facts.symbols[fields.owner as usize].name == name)
        };
        let config = fields("Config");
        assert!(config.is_some(), "Config properties were not captured");
        if let Some(config) = config {
            assert_eq!(
                config.names,
                [
                    "width",
                    "height",
                    "nested",
                    "choice",
                    "display-name",
                    "quoted-key",
                    "ümlaut",
                    "123",
                    "9007199254740991"
                ]
            );
            assert_eq!(config.types[0..2], ["number", "number"]);
            assert_eq!(config.types[3], "");
            assert_eq!(
                config.types[4..8],
                ["string", "string", "string", "boolean"]
            );
            assert_eq!(config.types[8], "number");
            assert!(!config.names.iter().any(|name| name == "inner"
                || name == "run"
                || name == "a,b"
                || name == "escaped\\u0061"
                || name == "escapeda"
                || name == "0x10"
                || name == "16"));
        }
        let other = fields("Other");
        assert!(other.is_some(), "Other properties were not captured");
        if let Some(other) = other {
            assert_eq!(other.names, ["enabled"]);
            assert_eq!(other.types, ["boolean"]);
        }
        assert!(fields("Unrelated").is_some());
        let malformed = crate::extract::extract(&record, b"interface Broken { -1: string; }");
        assert!(malformed.fields.is_empty(), "malformed key became a field");
        let malformed = crate::extract::extract(&record, b"interface Broken { -foo: string; }");
        assert!(
            malformed.fields.is_empty(),
            "malformed identifier became a field"
        );
        let mixed = crate::extract::extract(
            &record,
            b"interface Broken { -foo: string; }\ninterface Fine { value: number; 7: boolean; }",
        );
        assert_eq!(mixed.fields.len(), 1);
        assert_eq!(mixed.fields[0].names, ["value", "7"]);
    }

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
