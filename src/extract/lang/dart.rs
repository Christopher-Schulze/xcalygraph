//! Dart: names without a leading underscore are public, `main` entry, tests in `*_test.dart`
//! files run from their `main`; `import` URIs as imports.

use super::Pack;
use crate::domain::facts::{FLAG_ENTRY, FLAG_TEST, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Dart,
    language: |_| tree_sitter_dart::LANGUAGE.into(),
    query: include_str!("../queries/dart.scm"),
    hooks: Hooks {
        exported: |definition| !definition.name.starts_with('_'),
        signature_prefix: |_, _| "",
        flags,
        expand_import: engine::single_import,
        builtin,
    },
};

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    if definition.kind != SymKind::Fn || definition.name != "main" {
        return 0;
    }
    if crate::domain::role::is_test_path(FileType::Dart, context.path) {
        FLAG_TEST | FLAG_ENTRY
    } else {
        FLAG_ENTRY
    }
}

/// Dart core library names visible everywhere.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "String"
            | "int"
            | "double"
            | "num"
            | "bool"
            | "List"
            | "Map"
            | "Set"
            | "Future"
            | "Stream"
            | "Object"
            | "dynamic"
            | "void"
            | "Iterable"
            | "Duration"
            | "DateTime"
            | "Exception"
            | "Error"
            | "StateError"
            | "ArgumentError"
            | "identical"
            | "override"
            | "required"
    )
}
