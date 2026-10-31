//! Bash: functions, commands as calls, `source` and `.` imports with literal paths, reads of
//! variables the file never assigns as environment reads.

use super::Pack;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    file_type: FileType::Bash,
    suffixes: &[],
    language: |_| tree_sitter_bash::LANGUAGE.into(),
    query: include_str!("../queries/bash.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: |_: &Definition, _: &FileContext| 0,
        expand_import,
        builtin,
    },
};

/// Recovers functions the grammar lost to an error region (tree-sitter-bash gives up on some
/// `case` patterns such as `--*' '*)` and swallows what follows): column-0 headers `name() {`
/// and `function name {` without a parsed symbol become functions up to the next column-0 `}`,
/// and the words in command position inside them become calls.
pub fn recover_functions(source: &str, facts: &mut crate::domain::facts::Facts) {
    use crate::domain::facts::{NO_ARITY, Ref, RefKind, SymKind, Symbol};
    let lines: Vec<&str> = source.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let Some(name) = function_header(line) else {
            index += 1;
            continue;
        };
        let has_body = line.contains('{')
            || lines[index + 1..]
                .iter()
                .map(|line| line.trim())
                .find(|line| !line.is_empty() && !line.starts_with('#'))
                .is_some_and(|line| line.starts_with('{'));
        if !has_body {
            index += 1;
            continue;
        }
        let number = index as u32 + 1;
        let end = lines[index + 1..]
            .iter()
            .position(|candidate| candidate.trim_end() == "}")
            .map_or(index, |offset| index + 1 + offset);
        let parsed = facts
            .symbols
            .iter()
            .any(|symbol| symbol.name == name && symbol.line == number);
        if !parsed {
            // The error region read the header as a command: that is the definition, no call.
            facts
                .refs
                .retain(|reference| !(reference.line == number && reference.name == name));
            let scope = facts.symbols.len() as u32;
            facts.symbols.push(Symbol {
                kind: Some(SymKind::Fn),
                name: name.to_owned(),
                parent: None,
                owner: String::new(),
                line: number,
                end_line: end as u32 + 1,
                signature: line.trim().trim_end_matches('{').trim_end().to_owned(),
                doc: String::new(),
                terms: String::new(),
                exported: true,
                flags: 0,
            });
            for (offset, body) in lines
                .get(index + 1..end)
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let body_line = number + 1 + offset as u32;
                for command in command_words(body) {
                    let known = facts
                        .refs
                        .iter()
                        .any(|reference| reference.line == body_line && reference.name == command);
                    if !known && !builtin(command) {
                        facts.refs.push(Ref {
                            kind: RefKind::Call,
                            name: command.to_owned(),
                            qualifier: None,
                            scope: Some(scope),
                            line: body_line,
                            arity: NO_ARITY,
                        });
                    }
                }
            }
        }
        index = end.max(index) + 1;
    }
}

/// Name of a column-0 function header: `name() {`, `name ()`, `function name {`.
fn function_header(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = line.strip_prefix("function ").map_or(line, str::trim_start);
    let end = rest
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.')))
        .unwrap_or(rest.len());
    let (name, after) = rest.split_at(end);
    let after = after.trim_start();
    let header = after.starts_with("()")
        || (line.starts_with("function ") && (after.is_empty() || after.starts_with('{')));
    (header && !name.is_empty() && !name.starts_with(|ch: char| ch.is_ascii_digit()))
        .then_some(name)
}

/// Words in command position: line start and after `;`, `&&`, `||`, `|`, `$(`, `(`, `!` and the
/// keywords that open a command list; assignments and keywords are skipped.
fn command_words(line: &str) -> Vec<&str> {
    const KEYWORDS: [&str; 18] = [
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
        "in", "function", "select", "time", "{", "}",
    ];
    let code = line.trim_start();
    if code.starts_with('#') {
        return Vec::new();
    }
    let mut words = Vec::new();
    let separators: &[&str] = &["&&", "||", "$(", ";", "|", "(", "!", "`"];
    let mut segments: Vec<&str> = vec![code];
    for separator in separators {
        segments = segments
            .iter()
            .flat_map(|segment| segment.split(separator))
            .collect();
    }
    for segment in segments {
        let mut tokens = segment.split_whitespace().peekable();
        while let Some(token) = tokens.peek() {
            if KEYWORDS.contains(token) || token.contains('=') {
                tokens.next();
            } else {
                break;
            }
        }
        if let Some(token) = tokens
            .next()
            .map(|token| token.trim_end_matches([')', '"', '\'', '}']))
            && token.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
            && token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        {
            words.push(token);
        }
    }
    words
}

/// `source` and `.` with literal paths only (section 6); expansions are not resolvable.
fn expand_import(path: &str, line: u32, reexport: bool) -> Vec<crate::domain::facts::Import> {
    if path.contains('$') || path.contains('`') {
        return Vec::new();
    }
    engine::single_import(path, line, reexport)
}

/// Shell builtins and ubiquitous utilities: calls to them are no graph edges.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "echo"
            | ":"
            | "printf"
            | "cd"
            | "exit"
            | "return"
            | "set"
            | "unset"
            | "export"
            | "local"
            | "read"
            | "shift"
            | "test"
            | "["
            | "[["
            | "true"
            | "false"
            | "eval"
            | "exec"
            | "trap"
            | "source"
            | "\\."
            | "."
            | "command"
            | "type"
            | "cat"
            | "grep"
            | "sed"
            | "awk"
            | "cut"
            | "tr"
            | "sort"
            | "uniq"
            | "head"
            | "tail"
            | "wc"
            | "find"
            | "xargs"
            | "mkdir"
            | "rm"
            | "cp"
            | "mv"
            | "ln"
            | "chmod"
            | "ls"
            | "pwd"
            | "dirname"
            | "basename"
            | "date"
            | "sleep"
            | "env"
            | "wait"
            | "kill"
            | "readonly"
            | "declare"
            | "printenv"
            | "tee"
            | "touch"
            | "mktemp"
            | "realpath"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_and_command_words() {
        assert_eq!(function_header("nvm_ls() {"), Some("nvm_ls"));
        assert_eq!(function_header("function nvm_use {"), Some("nvm_use"));
        assert_eq!(function_header("  inner() {"), None);
        assert_eq!(function_header("echo hi"), None);
        assert_eq!(
            command_words("  if nvm_is_zsh && VERSION=\"$(nvm_version x)\"; then"),
            ["nvm_is_zsh", "nvm_version"]
        );
        assert_eq!(command_words("  # nvm_comment"), Vec::<&str>::new());
    }

    #[test]
    fn lost_functions_are_recovered_with_their_calls() {
        let source = "a() {\n  b\n}\nb() {\n  echo x | c\n}\n";
        let mut facts = crate::domain::facts::Facts::default();
        recover_functions(source, &mut facts);
        let names: Vec<&str> = facts
            .symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert_eq!(names, ["a", "b"]);
        let calls: Vec<(&str, u32)> = facts
            .refs
            .iter()
            .map(|reference| (reference.name.as_str(), reference.line))
            .collect();
        assert_eq!(calls, [("b", 2), ("c", 5)]);
    }

    #[test]
    fn incomplete_and_embedded_headers_do_not_panic_or_invent_functions() {
        let cases: [(&str, &[&str]); 4] = [
            ("root.mkdir()\n", &[]),
            ("function missing\n", &[]),
            ("unfinished() {\n", &["unfinished"]),
            ("multiline()\n{\n  actual\n}\n", &["multiline"]),
        ];
        for (source, expected) in cases {
            let mut facts = crate::domain::facts::Facts::default();
            recover_functions(source, &mut facts);
            let names: Vec<&str> = facts.symbols.iter().map(|s| s.name.as_str()).collect();
            assert_eq!(names, expected, "{source}");
        }
    }
}
