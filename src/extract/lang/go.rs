//! Go: package-level visibility by capitalization, `_test.go` test functions, `main` entry.

use super::Pack;
use crate::domain::facts::{FLAG_ENTRY, FLAG_TEST, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    file_type: FileType::Go,
    language: |_| tree_sitter_go::LANGUAGE.into(),
    query: include_str!("../queries/go.scm"),
    hooks: Hooks {
        exported,
        signature_prefix,
        flags,
        expand_import: engine::single_import,
        builtin,
    },
};

/// Predeclared Go functions and types.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "append"
            | "cap"
            | "clear"
            | "close"
            | "complex"
            | "copy"
            | "delete"
            | "imag"
            | "len"
            | "make"
            | "max"
            | "min"
            | "new"
            | "panic"
            | "print"
            | "println"
            | "real"
            | "recover"
            | "bool"
            | "byte"
            | "complex64"
            | "complex128"
            | "error"
            | "float32"
            | "float64"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "rune"
            | "string"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
            | "any"
            | "comparable"
    )
}

fn exported(definition: &Definition) -> bool {
    definition.name.starts_with(|ch: char| ch.is_uppercase())
}

fn signature_prefix(kind: SymKind, text: &str) -> &'static str {
    match kind {
        SymKind::Struct | SymKind::Iface | SymKind::Alias if !text.starts_with("type") => "type ",
        SymKind::Const if !text.starts_with("const") => "const ",
        SymKind::Var if !text.starts_with("var") => "var ",
        _ => "",
    }
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    let mut flags = 0;
    let test_prefix = ["Test", "Benchmark", "Fuzz", "Example"]
        .iter()
        .any(|prefix| definition.name.starts_with(prefix));
    if definition.kind == SymKind::Fn && context.path.ends_with("_test.go") && test_prefix {
        flags |= FLAG_TEST;
    }
    if definition.kind == SymKind::Fn
        && definition.name == "main"
        && context.package == Some("main")
    {
        flags |= FLAG_ENTRY;
    }
    flags
}
