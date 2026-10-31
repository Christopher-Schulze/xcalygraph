//! C: non-`static` definitions exported, prototypes flagged as declarations, `main` entry,
//! `#include` paths as imports. The hooks serve C++ and Objective-C too.

use super::Pack;
use crate::domain::facts::{
    FLAG_DECLARATION, FLAG_ENTRY, FLAG_TEST, Facts, Import, NO_ARITY, Ref, RefKind, SymKind,
};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks};

#[cfg(feature = "lang-c")]
pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::C,
    language: |_| tree_sitter_c::LANGUAGE.into(),
    query: include_str!("../queries/c.scm"),
    hooks: Hooks {
        exported,
        signature_prefix,
        flags,
        expand_import: include_import,
        builtin,
    },
};

/// Visible beyond the file unless declared `static`.
pub(super) fn exported(definition: &Definition) -> bool {
    let head = definition
        .text
        .split(['(', '{', '='])
        .next()
        .unwrap_or_default();
    !head.split_whitespace().any(|word| word == "static")
}

/// Macros keep `#define` in their signature.
pub(super) fn signature_prefix(kind: SymKind, text: &str) -> &'static str {
    if kind == SymKind::Macro && !text.starts_with('#') {
        "#define "
    } else {
        ""
    }
}

/// Declarations without body, `main`, and test macros (GoogleTest `TEST`, `TEST_F`, `TEST_P`,
/// `TYPED_TEST`; Catch2 `TEST_CASE`, `SCENARIO`).
pub(super) fn flags(definition: &Definition, _context: &FileContext) -> u8 {
    let callable = matches!(definition.kind, SymKind::Fn | SymKind::Method);
    let body = definition.text.contains('{');
    let mut flags = 0;
    if callable && !body {
        flags |= FLAG_DECLARATION;
    }
    if definition.kind == SymKind::Fn && definition.name == "main" && body {
        flags |= FLAG_ENTRY;
    }
    if callable
        && matches!(
            definition.name,
            "TEST"
                | "TEST_F"
                | "TEST_P"
                | "TYPED_TEST"
                | "TYPED_TEST_P"
                | "TEST_CASE"
                | "TEST_CASE_METHOD"
                | "SCENARIO"
        )
    {
        flags |= FLAG_TEST;
    }
    flags
}

/// `#include "a/b.h"` -> spec `a/b.h`; `#include <a/b.h>` -> spec `sys:a/b.h` (searched in include
/// directories only); Objective-C `#import` alike.
pub(super) fn include_import(text: &str, line: u32, _reexport: bool) -> Vec<Import> {
    let text = text.trim();
    let spec = if let Some(system) = text.strip_prefix('<') {
        format!("sys:{}", system.trim_end_matches('>'))
    } else {
        text.trim_matches('"').to_owned()
    };
    if spec.is_empty() || spec == "sys:" {
        return Vec::new();
    }
    vec![Import {
        spec,
        line,
        ..Import::default()
    }]
}

/// C standard library functions and macros: references to them carry no project edge.
pub(super) fn builtin(name: &str) -> bool {
    matches!(
        name,
        "printf"
            | "fprintf"
            | "sprintf"
            | "snprintf"
            | "puts"
            | "malloc"
            | "calloc"
            | "realloc"
            | "free"
            | "memcpy"
            | "memset"
            | "memmove"
            | "memcmp"
            | "strlen"
            | "strcmp"
            | "strncmp"
            | "strcpy"
            | "strncpy"
            | "strcat"
            | "strdup"
            | "assert"
            | "exit"
            | "abort"
            | "fopen"
            | "fclose"
            | "fread"
            | "fwrite"
            | "NULL"
            | "size_t"
            | "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "int8_t"
            | "int16_t"
            | "int32_t"
            | "int64_t"
            | "FILE"
            | "bool"
            | "true"
            | "false"
    )
}

/// Identifiers in `#define` bodies as value references of their macro: macro bodies are
/// unparsed text, yet they call functions (`#define getaddr(S,n) getaddr_(S, n)`). References
/// that name nothing in the root resolve to nothing.
pub fn macro_references(source: &[u8], facts: &mut Facts) {
    let text = String::from_utf8_lossy(source);
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let start = index;
        let mut body = String::new();
        let trimmed = lines[index].trim_start();
        let is_define = trimmed
            .strip_prefix('#')
            .is_some_and(|rest| rest.trim_start().starts_with("define"));
        loop {
            body.push_str(lines[index].trim_end_matches('\\'));
            body.push(' ');
            let continued = lines[index].trim_end().ends_with('\\');
            index += 1;
            if !continued || index >= lines.len() {
                break;
            }
        }
        if !is_define {
            continue;
        }
        let number = start as u32 + 1;
        let Some(scope) = facts
            .symbols
            .iter()
            .position(|symbol| symbol.kind() == SymKind::Macro && symbol.line == number)
        else {
            continue;
        };
        let name = facts.symbols[scope].name.clone();
        // Skip `#define NAME(params)`: identifiers after the parameter list are the body.
        let after_name = body.split_once(name.as_str()).map_or("", |(_, rest)| rest);
        let body = match after_name.strip_prefix('(') {
            Some(rest) => rest.split_once(')').map_or("", |(_, rest)| rest),
            None => after_name,
        };
        let mut seen: Vec<&str> = Vec::new();
        for word in body.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')) {
            let identifier = word
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphabetic() || ch == '_');
            if identifier
                && word != name
                && !seen.contains(&word)
                && !builtin(word)
                && !is_c_keyword(word)
            {
                seen.push(word);
                facts.refs.push(Ref {
                    kind: RefKind::Value,
                    name: word.to_owned(),
                    qualifier: None,
                    scope: u32::try_from(scope).ok(),
                    line: number,
                    arity: NO_ARITY,
                });
            }
        }
    }
}

fn is_c_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "else"
            | "while"
            | "for"
            | "do"
            | "return"
            | "sizeof"
            | "int"
            | "char"
            | "long"
            | "short"
            | "unsigned"
            | "signed"
            | "void"
            | "const"
            | "static"
            | "struct"
            | "union"
            | "enum"
            | "typedef"
            | "double"
            | "float"
            | "break"
            | "continue"
            | "switch"
            | "case"
            | "default"
            | "goto"
            | "volatile"
            | "extern"
            | "inline"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_distinguish_quoted_and_system_paths() {
        let spec = |text: &str| {
            include_import(text, 1, false)
                .into_iter()
                .map(|import| import.spec)
                .collect::<Vec<_>>()
        };
        assert_eq!(spec("\"store/db.h\""), ["store/db.h"]);
        assert_eq!(spec("<stdio.h>"), ["sys:stdio.h"]);
        assert!(
            spec("MACRO_PATH")
                .first()
                .is_some_and(|value| value == "MACRO_PATH")
        );
    }
}
