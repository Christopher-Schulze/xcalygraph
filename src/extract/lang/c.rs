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

/// Apple platform macros the grammars cannot parse, rewritten in place with the same byte length
/// so every line and column stays valid: region markers (`NS_ASSUME_NONNULL_BEGIN`,
/// `CF_EXTERN_C_BEGIN`) become blanks, and `typedef NS_ENUM(Base, Name) {...}` becomes
/// `enum Name {...}` so its enumerators are definitions. `None` when nothing matched.
pub fn mask_platform_macros(source: &[u8]) -> Option<Vec<u8>> {
    const REGIONS: [&[u8]; 8] = [
        b"NS_ASSUME_NONNULL_BEGIN",
        b"NS_ASSUME_NONNULL_END",
        b"CF_ASSUME_NONNULL_BEGIN",
        b"CF_ASSUME_NONNULL_END",
        b"CF_EXTERN_C_BEGIN",
        b"CF_EXTERN_C_END",
        b"CF_IMPLICIT_BRIDGING_ENABLED",
        b"CF_IMPLICIT_BRIDGING_DISABLED",
    ];
    const ENUMS: [&[u8]; 7] = [
        b"NS_ENUM",
        b"NS_OPTIONS",
        b"NS_CLOSED_ENUM",
        b"NS_ERROR_ENUM",
        b"CF_ENUM",
        b"CF_OPTIONS",
        b"CF_CLOSED_ENUM",
    ];
    if !REGIONS
        .iter()
        .chain(ENUMS.iter())
        .any(|word| memchr_word(source, word, 0).is_some())
    {
        return None;
    }
    let mut out = source.to_vec();
    for word in REGIONS {
        let mut from = 0;
        while let Some(start) = memchr_word(&out, word, from) {
            out[start..start + word.len()].fill(b' ');
            from = start + word.len();
        }
    }
    for word in ENUMS {
        let mut from = 0;
        while let Some(start) = memchr_word(&out, word, from) {
            from = start + word.len();
            let Some(open) = out[from..]
                .iter()
                .position(|&b| !b.is_ascii_whitespace())
                .map(|offset| from + offset)
                .filter(|&open| out[open] == b'(')
            else {
                continue;
            };
            let Some(close) = out[open..]
                .iter()
                .position(|&b| b == b')' || b == b'\n')
                .map(|offset| open + offset)
                .filter(|&close| out[close] == b')')
            else {
                continue;
            };
            // The name is the last argument: `NS_ENUM(NSUInteger, Mode)`.
            let comma = out[open..close].iter().rposition(|&b| b == b',');
            let name_start = comma.map_or(open + 1, |offset| open + offset + 1);
            out[start..name_start].fill(b' ');
            out[start..start + 4].copy_from_slice(b"enum");
            out[close] = b' ';
            // A preceding `typedef` would leave a declaration without declarator.
            let before = &out[..start];
            let trimmed = before.len()
                - before
                    .iter()
                    .rev()
                    .take_while(|b| b.is_ascii_whitespace())
                    .count();
            if out[..trimmed].ends_with(b"typedef")
                && (trimmed == 7 || !is_word_byte(out[trimmed - 8]))
            {
                out[trimmed - 7..trimmed].fill(b' ');
            }
        }
    }
    Some(out)
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// First whole-word occurrence of `word` at or after `from`.
fn memchr_word(haystack: &[u8], word: &[u8], from: usize) -> Option<usize> {
    let mut position = from;
    while position + word.len() <= haystack.len() {
        let offset = haystack[position..]
            .windows(word.len())
            .position(|window| window == word)?;
        let start = position + offset;
        let end = start + word.len();
        let before = start == 0 || !is_word_byte(haystack[start - 1]);
        let after = end == haystack.len() || !is_word_byte(haystack[end]);
        if before && after {
            return Some(start);
        }
        position = start + 1;
    }
    None
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

    #[test]
    fn platform_macros_are_masked_with_the_same_length() {
        let source = b"NS_ASSUME_NONNULL_BEGIN\ntypedef NS_ENUM(NSUInteger, Mode) {\n    ModeNone,\n};\n@interface Foo : NSObject\n@end\nNS_ASSUME_NONNULL_END\n";
        let masked = mask_platform_macros(source).unwrap_or_default();
        assert_eq!(masked.len(), source.len());
        let text = String::from_utf8(masked).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0].trim(), "");
        assert_eq!(
            lines[1].split_whitespace().collect::<Vec<_>>(),
            ["enum", "Mode", "{"]
        );
        assert_eq!(lines[4], "@interface Foo : NSObject");
        assert_eq!(lines[6].trim(), "");
        assert!(mask_platform_macros(b"@interface Foo : NSObject\n@end\n").is_none());
        // Whole words only: a longer identifier stays untouched.
        assert!(mask_platform_macros(b"int MY_NS_ENUM_COUNT = 1;\n").is_none());
    }
}
