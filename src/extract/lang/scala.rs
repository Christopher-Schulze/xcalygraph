//! Scala: public unless `private`, JUnit and ScalaTest suites (`*Spec`, `*Suite`), `main` and
//! `@main` entrypoints; imports and annotations as in Java.

use super::Pack;
use super::java::{header, jvm_flags, jvm_import};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Scala,
    language: |_| tree_sitter_scala::LANGUAGE.into(),
    query: include_str!("../queries/scala.scm"),
    hooks: Hooks {
        exported,
        signature_prefix: |_, _| "",
        flags,
        expand_import: jvm_import,
        builtin,
    },
};

fn exported(definition: &Definition) -> bool {
    !header(definition)
        .split_whitespace()
        .any(|word| word == "private" || word.starts_with("private["))
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    jvm_flags(
        definition,
        context,
        FileType::Scala,
        &["Spec", "Suite"],
        false,
    )
}

/// Scala standard library types and functions visible without import.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Int"
            | "Long"
            | "Short"
            | "Byte"
            | "Double"
            | "Float"
            | "Boolean"
            | "Char"
            | "Unit"
            | "Any"
            | "AnyRef"
            | "AnyVal"
            | "Nothing"
            | "Null"
            | "Option"
            | "Some"
            | "None"
            | "Either"
            | "Left"
            | "Right"
            | "List"
            | "Nil"
            | "Seq"
            | "Vector"
            | "Map"
            | "Set"
            | "Array"
            | "Iterator"
            | "Future"
            | "Try"
            | "Success"
            | "Failure"
            | "Throwable"
            | "Exception"
            | "RuntimeException"
            | "IllegalArgumentException"
            | "IllegalStateException"
            | "println"
            | "print"
            | "assert"
            | "override"
            | "deprecated"
    )
}
