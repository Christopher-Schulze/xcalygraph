//! Body words of symbols for search (`docs/spec.md` section 6), stored as normalized search
//! tokens so the derived index only splits them: the tokens of the distinct identifier-like
//! words of each symbol's lines outside nested symbols (identifiers, string contents, comments,
//! markup text), so a question in behavior words ("store the session cookie") finds the handler
//! whose body calls `cookies.set('jwt', ...)`. Each line counts for its innermost symbol only;
//! lines outside every symbol (top-level statements) are the file's words. URLs are skipped:
//! their host words (`api.realworld.show`) say nothing about behavior.

use std::collections::HashSet;

use crate::domain::facts::{BODY_TERMS, Facts, SymKind};
use crate::domain::search;

/// Keywords and literals common to most languages: in nearly every body, never distinguishing.
const KEYWORDS: [&str; 48] = [
    "and",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "def",
    "default",
    "elif",
    "else",
    "end",
    "enum",
    "export",
    "extends",
    "false",
    "final",
    "for",
    "from",
    "func",
    "function",
    "impl",
    "import",
    "let",
    "mut",
    "new",
    "nil",
    "none",
    "not",
    "null",
    "pub",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "switch",
    "the",
    "then",
    "this",
    "throw",
    "true",
    "try",
    "undefined",
    "var",
    "while",
];

/// Fills [`crate::domain::facts::Symbol::terms`] for code symbols and
/// [`Facts::terms`] for top-level code from the file content.
pub fn add(content: &[u8], facts: &mut Facts) {
    let starts = line_starts(content);
    let ranges: Vec<(u32, u32)> = facts
        .symbols
        .iter()
        .map(|symbol| (symbol.line, symbol.end_line.max(symbol.line)))
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    let lines = starts.len() as u32;
    facts.terms = collect(content, &starts, (1, lines), &ranges, &mut seen);
    for (index, symbol) in facts.symbols.iter_mut().enumerate() {
        if !matches!(
            symbol.kind(),
            SymKind::Fn
                | SymKind::Method
                | SymKind::Class
                | SymKind::Struct
                | SymKind::Enum
                | SymKind::Iface
                | SymKind::Trait
                | SymKind::Const
                | SymKind::Var
                | SymKind::Macro
                | SymKind::Table
                | SymKind::View
        ) {
            continue;
        }
        let range = ranges[index];
        // A one-line symbol's words are its signature.
        if range.0 == range.1 {
            continue;
        }
        symbol.terms = collect(content, &starts, range, &ranges, &mut seen);
    }
}

/// Tokens of the distinct words of the lines of `range` outside the ranges of `others` strictly
/// inside it (the symbol itself and symbols on the same lines are not inside).
fn collect(
    content: &[u8],
    starts: &[usize],
    (first, last): (u32, u32),
    others: &[(u32, u32)],
    seen: &mut HashSet<String>,
) -> String {
    let nested: Vec<(u32, u32)> = others
        .iter()
        .copied()
        .filter(|&(line, end)| line >= first && end <= last && (line, end) != (first, last))
        .collect();
    seen.clear();
    let mut terms = String::new();
    let mut count = 0;
    let mut line_words = String::new();
    let mut buffer = String::new();
    for line in first..=last {
        if count >= BODY_TERMS {
            break;
        }
        if nested
            .iter()
            .any(|&(start, end)| (start..=end).contains(&line))
        {
            continue;
        }
        let Some(text) = line_text(content, starts, line) else {
            break;
        };
        // Search tokens of the line's new words (phrases such as `sign in` stay adjacent); a
        // token shared by several words (`delete` of `deleteUser`, `deleteComment`) repeats, so
        // its frequency still counts.
        line_words.clear();
        for word in words(text) {
            let lower = word.to_ascii_lowercase();
            if !KEYWORDS.contains(&lower.as_str()) && seen.insert(lower) {
                line_words.push_str(word);
                line_words.push(' ');
            }
        }
        search::for_each_token(&line_words, &mut buffer, |token| {
            if count >= BODY_TERMS || token.len() < 3 {
                return;
            }
            if !terms.is_empty() {
                terms.push(' ');
            }
            terms.push_str(token);
            count += 1;
        });
    }
    terms
}

/// Byte offset of every line start.
fn line_starts(content: &[u8]) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            content
                .iter()
                .enumerate()
                .filter(|&(_, &byte)| byte == b'\n')
                .map(|(index, _)| index + 1),
        )
        .collect()
}

/// Bytes of the 1-based line, without its newline.
fn line_text<'a>(content: &'a [u8], starts: &[usize], line: u32) -> Option<&'a [u8]> {
    let start = *starts.get(line.checked_sub(1)? as usize)?;
    let end = starts
        .get(line as usize)
        .map_or(content.len(), |next| next - 1);
    content.get(start..end.max(start))
}

/// ASCII identifier-like words of at least 2 characters outside URLs (`in` of `sign in`
/// counts for the phrase), digits-only words excluded.
fn words(text: &[u8]) -> impl Iterator<Item = &str> {
    text.split(|byte| {
        byte.is_ascii_whitespace() || matches!(byte, b'\'' | b'"' | b'`' | b'(' | b')')
    })
    .filter(|chunk| !chunk.windows(3).any(|window| window == b"://"))
    .flat_map(|chunk| chunk.split(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_')))
    .filter(|word| word.len() >= 2 && word.iter().any(u8::is_ascii_alphabetic))
    .filter_map(|word| std::str::from_utf8(word).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::facts::Symbol;

    #[test]
    fn body_words_skip_nested_symbols_and_keywords() {
        let content =
            b"class Session {\n  ttl = 30\n  save() {\n    cookies.set('jwt', token)\n  }\n}\n";
        let mut facts = Facts {
            symbols: vec![
                Symbol {
                    kind: Some(SymKind::Class),
                    name: "Session".to_owned(),
                    line: 1,
                    end_line: 6,
                    ..Symbol::default()
                },
                Symbol {
                    kind: Some(SymKind::Method),
                    name: "save".to_owned(),
                    parent: Some(0),
                    line: 3,
                    end_line: 5,
                    ..Symbol::default()
                },
            ],
            ..Facts::default()
        };
        add(content, &mut facts);
        assert_eq!(facts.symbols[0].terms, "session ttl");
        assert_eq!(facts.symbols[1].terms, "save cooky set jwt token");
        assert_eq!(facts.terms, "");
    }

    #[test]
    fn top_level_code_belongs_to_the_file_and_urls_are_skipped() {
        let content = b"const base = 'https://api.example.show/v1'\napi.interceptors.use(notify)\n";
        let mut facts = Facts {
            symbols: vec![Symbol {
                kind: Some(SymKind::Const),
                name: "base".to_owned(),
                line: 1,
                end_line: 1,
                ..Symbol::default()
            }],
            ..Facts::default()
        };
        add(content, &mut facts);
        // A one-line symbol's words are its signature.
        assert_eq!(facts.symbols[0].terms, "");
        assert_eq!(facts.terms, "api interceptor use notify");
    }
}
