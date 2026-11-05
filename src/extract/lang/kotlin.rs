//! Kotlin: public by default, JUnit and Kotest tests, top-level `main` entry, framework
//! annotations as entrypoints; imports and annotations as in Java.

use super::Pack;
use super::java::{header, jvm_flags, jvm_import};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Kotlin,
    language: |_| tree_sitter_kotlin_ng::LANGUAGE.into(),
    query: include_str!("../queries/kotlin.scm"),
    hooks: Hooks {
        exported,
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(jvm_import),
        builtin,
    },
};

fn exported(definition: &Definition) -> bool {
    !header(definition)
        .split_whitespace()
        .any(|word| word == "private")
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    jvm_flags(definition, context, FileType::Kotlin, &[], false)
}

/// Kotlin standard library types and collection builders visible without import; function
/// names a class may shadow (`check`, `require`, `run`) stay references.
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
            | "Nothing"
            | "Number"
            | "Array"
            | "List"
            | "MutableList"
            | "Map"
            | "MutableMap"
            | "Set"
            | "MutableSet"
            | "Collection"
            | "Iterable"
            | "Sequence"
            | "Pair"
            | "Triple"
            | "Result"
            | "Throwable"
            | "Exception"
            | "RuntimeException"
            | "IllegalArgumentException"
            | "IllegalStateException"
            | "println"
            | "print"
            | "listOf"
            | "listOfNotNull"
            | "mutableListOf"
            | "emptyList"
            | "mapOf"
            | "mutableMapOf"
            | "emptyMap"
            | "setOf"
            | "mutableSetOf"
            | "emptySet"
            | "arrayOf"
            | "sequenceOf"
            | "buildList"
            | "buildMap"
            | "buildString"
            | "Override"
            | "Deprecated"
            | "Suppress"
            | "JvmStatic"
            | "JvmOverloads"
            | "Throws"
            | "true"
            | "false"
            | "null"
            | "it"
            | "this"
    )
}
