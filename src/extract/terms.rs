//! Body words of symbols for search (`docs/spec.md` section 6), stored as normalized search
//! tokens so the derived index only splits them: the tokens of the distinct identifier-like
//! words of each symbol's lines outside nested symbols (identifiers, string contents, comments,
//! markup text), so a question in behavior words ("store the session cookie") finds the handler
//! whose body calls `cookies.set('jwt', ...)`. Each line counts for its innermost symbol only;
//! a symbol's lines start at the doc comment above it; lines outside every symbol (top-level
//! statements) are the file's words. URLs are skipped:
//! their host words (`api.realworld.show`) say nothing about behavior.

use std::collections::HashSet;

use crate::domain::facts::{BODY_TERMS, Facts, SymKind};
use crate::domain::search;

/// Keywords and literals common to most languages, and markup filler (`className`, `div`,
/// `span`): in nearly every body, never distinguishing.
const KEYWORDS: [&str; 51] = [
    "and",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "classname",
    "const",
    "continue",
    "def",
    "default",
    "div",
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
    "span",
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
/// [`Facts::terms`] for top-level code from the file content. `spans` holds the syntax-tree byte
/// range of the leading symbols (the engine's); later symbols span their whole lines. Ownership
/// is by bytes, so one-line and minified symbols keep their own words and never take a
/// neighbor's.
pub fn add(content: &[u8], facts: &mut Facts, spans: &[(usize, usize)]) {
    let starts = line_starts(content);
    let line_start = |line: u32| starts.get(line.saturating_sub(1) as usize).copied();
    let line_end = |line: u32| {
        starts
            .get(line as usize)
            .map_or(content.len(), |next| next - 1)
    };
    let owned: Vec<(usize, usize)> = facts
        .symbols
        .iter()
        .enumerate()
        .map(|(index, symbol)| {
            let (start, end) = spans.get(index).copied().unwrap_or_else(|| {
                (
                    line_start(symbol.line).unwrap_or(0),
                    line_end(symbol.end_line.max(symbol.line)),
                )
            });
            // The doc comment directly above belongs to the symbol.
            let first = leading_comment_start(content, &starts, symbol.line);
            let start = if first < symbol.line {
                line_start(first).map_or(start, |comment| comment.min(start))
            } else {
                start
            };
            (start.min(content.len()), end.min(content.len()).max(start))
        })
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    facts.terms = collect(content, (0, content.len()), &owned, &mut seen);
    for (index, symbol) in facts.symbols.iter_mut().enumerate() {
        if matches!(
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
            symbol.terms = collect(content, owned[index], &owned, &mut seen);
        }
    }
}

/// Tokens of the distinct words of the bytes of `range` outside the ranges of `others` strictly
/// inside it (the range itself and identical ranges are not inside), line by line.
fn collect(
    content: &[u8],
    (first, last): (usize, usize),
    others: &[(usize, usize)],
    seen: &mut HashSet<String>,
) -> String {
    let mut text = content[first..last].to_vec();
    for &(start, end) in others {
        if start >= first && end <= last && (start, end) != (first, last) {
            for byte in &mut text[start - first..end - first] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
        }
    }
    seen.clear();
    let mut terms = String::new();
    let mut count = 0;
    let mut line_words = String::new();
    let mut buffer = String::new();
    for line in text.split(|byte| *byte == b'\n') {
        if count >= BODY_TERMS {
            break;
        }
        // Search tokens of the line's new words (phrases such as `sign in` stay adjacent); a
        // token shared by several words (`delete` of `deleteUser`, `deleteComment`) repeats, so
        // its frequency still counts.
        line_words.clear();
        let line = &without_class_lists(line);
        for word in words(line) {
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

/// The line with quoted CSS class lists blanked (`"flex items-center px-4"`): utility class
/// names would fill a component's token budget with layout noise.
fn without_class_lists(text: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let mut out: Option<Vec<u8>> = None;
    let mut index = 0;
    while index < text.len() {
        let quote = text[index];
        if !matches!(quote, b'"' | b'\'' | b'`') {
            index += 1;
            continue;
        }
        let Some(length) = text[index + 1..].iter().position(|&byte| byte == quote) else {
            break;
        };
        let inner = &text[index + 1..index + 1 + length];
        if is_class_list(inner) {
            out.get_or_insert_with(|| text.to_vec())[index + 1..index + 1 + length].fill(b' ');
        }
        index += length + 2;
    }
    out.map_or(std::borrow::Cow::Borrowed(text), std::borrow::Cow::Owned)
}

/// At least two space-separated class-like items (lowercase, digits, `-:/[].%!`), one with a
/// dash or colon: `"flex items-center"`, `"md:px-4 text-sm"`.
fn is_class_list(inner: &[u8]) -> bool {
    let items: Vec<&[u8]> = inner
        .split(|byte| *byte == b' ')
        .filter(|item| !item.is_empty())
        .collect();
    items.len() >= 2
        && items.iter().all(|item| {
            item.iter().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b':' | b'/' | b'[' | b']' | b'.' | b'%' | b'!')
            })
        })
        && items
            .iter()
            .any(|item| item.contains(&b'-') || item.contains(&b':'))
}

/// First line of the comment and annotation block directly above `line` (`///`, `/** */`,
/// `#`, `--`, `@Decorator`), else `line`: the full doc comment counts for the symbol.
fn leading_comment_start(content: &[u8], starts: &[usize], line: u32) -> u32 {
    let mut first = line;
    while first > 1 {
        let Some(text) = line_text(content, starts, first - 1) else {
            break;
        };
        let trimmed = text.trim_ascii_start();
        let comment = [&b"//"[..], b"/*", b"*", b"#", b"--", b"@"]
            .iter()
            .any(|marker| trimmed.starts_with(marker));
        if !comment {
            break;
        }
        first -= 1;
    }
    first
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
        add(content, &mut facts, &[]);
        assert_eq!(facts.symbols[0].terms, "session ttl");
        assert_eq!(facts.symbols[1].terms, "save cooky set jwt token");
        assert_eq!(facts.terms, "");
    }

    #[test]
    fn css_class_lists_are_no_words() {
        let line = b"<div className=\"flex items-center gap-2\">Sign out</div>";
        let kept = without_class_lists(line);
        let words: Vec<&str> = words(&kept).collect();
        assert_eq!(words, ["div", "className", "Sign", "out", "div"]);
        assert!(KEYWORDS.contains(&"classname"));
        assert!(!is_class_list(b"sign-in"));
        assert!(!is_class_list(b"Hello world"));
    }

    #[test]
    fn doc_comments_above_a_symbol_are_its_words() {
        let content = b"setup(app)\n/**\n * Compose middleware into one handler.\n */\nfunction compose(a) {\n  return a\n}\n";
        let mut facts = Facts {
            symbols: vec![Symbol {
                kind: Some(SymKind::Fn),
                name: "compose".to_owned(),
                line: 5,
                end_line: 7,
                ..Symbol::default()
            }],
            ..Facts::default()
        };
        add(content, &mut facts, &[]);
        assert_eq!(facts.symbols[0].terms, "compos middlewar into one handler");
        assert_eq!(facts.terms, "setup app");
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
        add(content, &mut facts, &[]);
        // A one-line symbol owns its line; URLs stay skipped.
        assert_eq!(facts.symbols[0].terms, "base");
        assert_eq!(facts.terms, "api interceptor use notify");
    }
}
