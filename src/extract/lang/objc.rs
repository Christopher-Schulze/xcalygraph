//! Objective-C: C hooks for functions, methods always visible, `XCTestCase` subclasses and their
//! `test*` methods as tests.

use super::Pack;
use super::c::{include_import, signature_prefix};
use crate::domain::facts::{FLAG_TEST, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::ObjC,
    language: |_| tree_sitter_objc::LANGUAGE.into(),
    query: include_str!("../queries/objc.scm"),
    hooks: Hooks {
        exported,
        signature_prefix,
        flags,
        expand_import: ImportExpansion::Text(include_import),
        builtin,
    },
};

fn exported(definition: &Definition) -> bool {
    definition.kind == SymKind::Method || super::c::exported(definition)
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    let mut flags = super::c::flags(definition, context);
    let header = definition.text.split('\n').next().unwrap_or_default();
    if definition.kind == SymKind::Class && header.contains("XCTestCase") {
        flags |= FLAG_TEST;
    }
    let test_file = crate::domain::role::is_test_path(FileType::ObjC, context.path);
    if definition.kind == SymKind::Method
        && definition.name.starts_with("test")
        && (definition.inherited & FLAG_TEST != 0 || test_file)
    {
        flags |= FLAG_TEST;
    }
    flags
}

/// C library names and Apple framework names (`NS`, `CF`, `CG`, `UI` prefixes, reserved for
/// the platform): references to them carry no project edge.
fn builtin(name: &str) -> bool {
    let apple = ["NS", "CF", "CG", "UI"].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .and_then(|rest| rest.chars().next())
            .is_some_and(char::is_uppercase)
    });
    apple || name.starts_with("dispatch_") || super::c::builtin(name)
}
