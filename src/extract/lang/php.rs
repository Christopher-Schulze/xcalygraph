//! PHP: public unless `private`/`protected`, PHPUnit `test*` methods and `#[Test]`, `use`
//! statements per imported name.

use super::Pack;
use crate::domain::facts::{FLAG_TEST, Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Php,
    language: |_| tree_sitter_php::LANGUAGE_PHP.into(),
    query: include_str!("../queries/php.scm"),
    hooks: Hooks {
        exported,
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(use_import),
        builtin,
    },
};

fn exported(definition: &Definition) -> bool {
    let head = super::java::header(definition);
    !head
        .split_whitespace()
        .any(|word| matches!(word, "private" | "protected"))
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    let head = super::java::header(definition);
    let method = definition.kind == SymKind::Method;
    let test_file = crate::domain::role::is_test_path(FileType::Php, context.path);
    if method && ((definition.name.starts_with("test") && test_file) || head.contains("#[Test]")) {
        FLAG_TEST
    } else {
        0
    }
}

/// `use A\B\C;`, `use A\B\C as D;`, `use A\B\{C, D as E};`, `use function A\f;` -> spec `A\B`,
/// name `C`, alias.
fn use_import(statement: &str, line: u32, _reexport: bool) -> Vec<Import> {
    let text = statement
        .trim()
        .trim_start_matches("use")
        .trim()
        .trim_end_matches(';')
        .trim();
    let text = text
        .trim_start_matches("function ")
        .trim_start_matches("const ")
        .trim();
    let one = |path: &str, alias: Option<&str>| -> Option<Import> {
        let path = path.trim().trim_start_matches('\\');
        let (spec, name) = path.rsplit_once('\\').unwrap_or(("", path));
        (!name.is_empty()).then(|| Import {
            spec: spec.to_owned(),
            alias: alias.map(|alias| alias.trim().to_owned()),
            names: vec![name.to_owned()],
            line,
            reexport: false,
        })
    };
    let split_alias = |part: &str| -> (String, Option<String>) {
        match part.split_once(" as ") {
            Some((path, alias)) => (path.trim().to_owned(), Some(alias.trim().to_owned())),
            None => (part.trim().to_owned(), None),
        }
    };
    if let Some((base, group)) = text.split_once('{') {
        let base = base.trim().trim_end_matches('\\');
        return group
            .trim_end_matches('}')
            .split(',')
            .filter_map(|part| {
                let (path, alias) = split_alias(part);
                one(&format!("{base}\\{path}"), alias.as_deref())
            })
            .collect();
    }
    text.split(',')
        .filter_map(|part| {
            let (path, alias) = split_alias(part);
            one(&path, alias.as_deref())
        })
        .collect()
}

/// PHP built-in functions and classes: references to them carry no project edge.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "count"
            | "strlen"
            | "array_map"
            | "array_filter"
            | "array_keys"
            | "array_values"
            | "in_array"
            | "isset"
            | "empty"
            | "unset"
            | "sprintf"
            | "printf"
            | "echo"
            | "print"
            | "json_encode"
            | "json_decode"
            | "implode"
            | "explode"
            | "str_replace"
            | "trim"
            | "is_array"
            | "is_string"
            | "Exception"
            | "InvalidArgumentException"
            | "RuntimeException"
            | "stdClass"
            | "self"
            | "static"
            | "parent"
            | "string"
            | "int"
            | "bool"
            | "array"
            | "void"
            | "mixed"
            | "null"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_statements_expand_per_name() {
        let specs = |text: &str| -> Vec<(String, String, Option<String>)> {
            use_import(text, 1, false)
                .into_iter()
                .map(|import| (import.spec, import.names.join(","), import.alias))
                .collect()
        };
        assert_eq!(
            specs("use App\\Store\\Repo;"),
            [("App\\Store".to_owned(), "Repo".to_owned(), None)]
        );
        assert_eq!(
            specs("use App\\Store\\{Repo, Row as R};"),
            [
                ("App\\Store".to_owned(), "Repo".to_owned(), None),
                (
                    "App\\Store".to_owned(),
                    "Row".to_owned(),
                    Some("R".to_owned())
                )
            ]
        );
        assert_eq!(
            specs("use function App\\helpers\\format;"),
            [("App\\helpers".to_owned(), "format".to_owned(), None)]
        );
    }
}
