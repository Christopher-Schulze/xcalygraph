//! Swift: `public` and `open` exported, `XCTestCase` subclasses with their `test*` methods and
//! `@Test` functions as tests, `@main` types as entrypoints.

use super::Pack;
use crate::domain::facts::{FLAG_ENTRY, FLAG_TEST, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Swift,
    language: |_| tree_sitter_swift::LANGUAGE.into(),
    query: include_str!("../queries/swift.scm"),
    hooks: Hooks {
        exported,
        signature_prefix: |_, _| "",
        flags,
        expand_import: engine::single_import,
        builtin,
    },
};

fn exported(definition: &Definition) -> bool {
    let head = super::java::header(definition);
    head.split_whitespace()
        .any(|word| matches!(word, "public" | "open"))
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    let head = super::java::header(definition);
    let first_line = definition.text.split('{').next().unwrap_or_default();
    let mut flags = 0;
    if definition.kind == SymKind::Class && first_line.contains("XCTestCase") {
        flags |= FLAG_TEST;
    }
    let test_file = crate::domain::role::is_test_path(FileType::Swift, context.path);
    let function = matches!(definition.kind, SymKind::Fn | SymKind::Method);
    if function
        && (head.contains("@Test")
            || (definition.name.starts_with("test")
                && (definition.inherited & FLAG_TEST != 0 || test_file)))
    {
        flags |= FLAG_TEST;
    }
    if head.contains("@main") {
        flags |= FLAG_ENTRY;
    }
    flags
}

/// Swift standard library names visible everywhere.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "debugPrint"
            | "fatalError"
            | "precondition"
            | "preconditionFailure"
            | "assert"
            | "assertionFailure"
            | "min"
            | "max"
            | "zip"
            | "stride"
            | "String"
            | "Int"
            | "Double"
            | "Float"
            | "Bool"
            | "Character"
            | "Array"
            | "Dictionary"
            | "Set"
            | "Optional"
            | "Result"
            | "Error"
            | "Void"
            | "Any"
            | "AnyObject"
            | "Self"
    )
}
