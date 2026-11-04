//! Bash: functions, commands as calls, `source` and `.` imports with literal paths, reads of
//! variables the file never assigns as environment reads.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

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

/// Adds Bash facts that need the parse tree: literal trap actions and `getopts` case branches.
pub fn post_parse(
    source: &[u8],
    root: Node<'_>,
    parsed: &mut engine::Parsed,
    capture_identities: bool,
) {
    recover_functions(&String::from_utf8_lossy(source), &mut parsed.facts);
    if capture_identities {
        parsed.identities.resize(parsed.facts.symbols.len(), None);
    }
    let identities = capture_identities.then_some(&mut parsed.identities);
    extract_constructs(source, root, &mut parsed.facts, identities);
}

#[derive(Clone, Copy)]
struct OptionSpec {
    character: char,
    requires_argument: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaseHandler {
    Explicit(u32),
    Fallback(u32),
    None,
    Ambiguous,
}

fn extract_constructs(
    source: &[u8],
    root: Node<'_>,
    facts: &mut crate::domain::facts::Facts,
    mut identities: Option<&mut Vec<Option<String>>>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "command" && command_named(node, "trap", source) {
            if let Some(reference) = trap_reference(node, source, facts) {
                facts.refs.push(reference);
            }
        } else if node.kind() == "while_statement" {
            extract_getopts(node, source, facts, &mut identities);
        }
        for index in (0..node.child_count()).rev() {
            if let Some(child) = node.child(index) {
                stack.push(child);
            }
        }
    }
}

fn extract_getopts(
    statement: Node<'_>,
    source: &[u8],
    facts: &mut crate::domain::facts::Facts,
    identities: &mut Option<&mut Vec<Option<String>>>,
) {
    let Some(condition) = statement.child_by_field_name("condition") else {
        return;
    };
    let commands = condition_getopts_commands(condition, source);
    let [command] = commands.as_slice() else {
        return;
    };
    let arguments = field_nodes(*command, "argument");
    let (Some(specification), Some(variable)) = (arguments.first(), arguments.get(1)) else {
        return;
    };
    let (Some(specification), Some(variable)) = (
        literal_value(*specification, source),
        literal_value(*variable, source),
    ) else {
        return;
    };
    if !shell_variable(variable) {
        return;
    }
    let Some(options) = parse_option_spec(specification) else {
        return;
    };
    let Some(body) = statement.child_by_field_name("body") else {
        return;
    };
    let handlers = case_handlers(body, variable, &options, source);
    let specification_line = source_line(*command);
    for option in options {
        let (case, handler_kind) = match handlers
            .get(&option.character)
            .copied()
            .unwrap_or(CaseHandler::None)
        {
            CaseHandler::Explicit(line) => (line.to_string(), "explicit"),
            CaseHandler::Fallback(line) => (format!("*:{line}"), "fallback"),
            CaseHandler::None => ("none".to_owned(), "none"),
            CaseHandler::Ambiguous => ("ambiguous".to_owned(), "ambiguous"),
        };
        let argument = if option.requires_argument {
            "required"
        } else {
            "none"
        };
        let signature = format!("arg={argument};case={case}");
        facts.symbols.push(crate::domain::facts::Symbol {
            kind: Some(crate::domain::facts::SymKind::Option),
            name: format!("-{}", option.character),
            parent: None,
            owner: String::new(),
            line: specification_line,
            end_line: specification_line,
            signature,
            doc: String::new(),
            terms: String::new(),
            exported: false,
            flags: 0,
        });
        if let Some(identities) = identities.as_deref_mut() {
            identities.push(Some(format!("arg={argument};handler={handler_kind}")));
        }
    }
}

fn trap_reference(
    command: Node<'_>,
    source: &[u8],
    facts: &crate::domain::facts::Facts,
) -> Option<crate::domain::facts::Ref> {
    use crate::domain::facts::{NO_ARITY, Ref, RefKind, SymKind};

    let action = *field_nodes(command, "argument").first()?;
    let name = literal_value(action, source)?;
    if name.starts_with('-') || !function_name(name) || builtin(name) {
        return None;
    }
    let line = source_line(action);
    let scope = facts
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| {
            symbol.kind() == SymKind::Fn && symbol.line <= line && line <= symbol.end_line
        })
        .min_by_key(|(_, symbol)| symbol.end_line.saturating_sub(symbol.line))
        .and_then(|(index, _)| u32::try_from(index).ok());
    Some(Ref {
        kind: RefKind::Call,
        name: name.to_owned(),
        qualifier: None,
        scope,
        line,
        arity: NO_ARITY,
    })
}

fn case_handlers(
    body: Node<'_>,
    variable: &str,
    options: &[OptionSpec],
    source: &[u8],
) -> HashMap<char, CaseHandler> {
    let mut handlers: HashMap<char, CaseHandler> = options
        .iter()
        .map(|option| (option.character, CaseHandler::None))
        .collect();
    let cases: Vec<Node<'_>> = loop_cases(body)
        .into_iter()
        .filter(|case| case_variable(*case, source).as_deref() == Some(variable))
        .collect();
    let [case] = cases.as_slice() else {
        if cases.len() > 1 {
            handlers
                .values_mut()
                .for_each(|handler| *handler = CaseHandler::Ambiguous);
        }
        return handlers;
    };
    for item in (0..case.child_count()).filter_map(|index| case.child(index)) {
        if item.kind() != "case_item" {
            continue;
        }
        let line = source_line(item);
        let mut explicit = HashSet::new();
        let mut fallback = false;
        let mut unsupported = false;
        for value in field_nodes(item, "value") {
            if wildcard(value, source) {
                fallback = true;
            } else if let Some(character) = case_option(value, source) {
                explicit.insert(character);
            } else {
                unsupported = true;
            }
        }
        for option in options {
            let Some(handler) = handlers.get_mut(&option.character) else {
                continue;
            };
            if *handler != CaseHandler::None {
                continue;
            }
            *handler = if explicit.contains(&option.character) {
                CaseHandler::Explicit(line)
            } else if fallback {
                CaseHandler::Fallback(line)
            } else if unsupported {
                CaseHandler::Ambiguous
            } else {
                CaseHandler::None
            };
        }
    }
    handlers
}

fn condition_getopts_commands<'tree>(condition: Node<'tree>, source: &[u8]) -> Vec<Node<'tree>> {
    let mut stack = vec![condition];
    let mut commands = Vec::new();
    while let Some(node) = stack.pop() {
        if node.kind() == "command" {
            if command_named(node, "getopts", source) {
                commands.push(node);
            }
            continue;
        }
        if matches!(
            node.kind(),
            "command_substitution" | "process_substitution" | "subshell" | "function_definition"
        ) {
            continue;
        }
        for index in (0..node.child_count()).rev() {
            if let Some(child) = node.child(index) {
                stack.push(child);
            }
        }
    }
    commands
}

fn loop_cases(body: Node<'_>) -> Vec<Node<'_>> {
    let mut stack = vec![body];
    let mut cases = Vec::new();
    while let Some(node) = stack.pop() {
        if node.kind() == "case_statement" {
            cases.push(node);
        }
        if matches!(node.kind(), "while_statement" | "function_definition") {
            continue;
        }
        for index in (0..node.child_count()).rev() {
            if let Some(child) = node.child(index) {
                stack.push(child);
            }
        }
    }
    cases
}

fn case_variable(case: Node<'_>, source: &[u8]) -> Option<String> {
    let value = case
        .child_by_field_name("value")?
        .utf8_text(source)
        .ok()?
        .trim();
    let value = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value);
    let name = value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'))?;
    shell_variable(name).then(|| name.to_owned())
}

fn case_option(value: Node<'_>, source: &[u8]) -> Option<char> {
    if value.kind() == "word"
        && let Some(text) = value.utf8_text(source).ok()?.strip_prefix('\\')
    {
        let mut characters = text.chars();
        let character = characters.next()?;
        if character.is_ascii_graphic() && characters.next().is_none() {
            return Some(character);
        }
    }
    let text = literal_value(value, source)?;
    if value.kind() == "word" && text.chars().any(|ch| matches!(ch, '*' | '?' | '[' | '\\')) {
        return None;
    }
    let mut characters = text.chars();
    let character = characters.next()?;
    characters.next().is_none().then_some(character)
}

fn wildcard(value: Node<'_>, source: &[u8]) -> bool {
    value.kind() == "extglob_pattern"
        && value.utf8_text(source).is_ok_and(|text| text.trim() == "*")
}

fn parse_option_spec(specification: &str) -> Option<Vec<OptionSpec>> {
    let mut characters = specification.chars().peekable();
    if characters.peek() == Some(&':') {
        characters.next();
    }
    let mut seen = HashSet::new();
    let mut options = Vec::new();
    while let Some(character) = characters.next() {
        if !character.is_ascii_graphic()
            || matches!(character, ':' | '?')
            || !seen.insert(character)
        {
            return None;
        }
        let requires_argument = characters.peek() == Some(&':');
        if requires_argument {
            characters.next();
            if characters.peek() == Some(&':') {
                return None;
            }
        }
        options.push(OptionSpec {
            character,
            requires_argument,
        });
    }
    (!options.is_empty()).then_some(options)
}

fn command_named(command: Node<'_>, name: &str, source: &[u8]) -> bool {
    command
        .child_by_field_name("name")
        .and_then(|node| node.utf8_text(source).ok())
        == Some(name)
}

fn field_nodes<'tree>(node: Node<'tree>, field: &str) -> Vec<Node<'tree>> {
    (0..node.child_count())
        .filter(|&index| node.field_name_for_child(index) == Some(field))
        .filter_map(|index| node.child(index))
        .collect()
}

pub(in crate::extract) fn literal_value<'source>(
    node: Node<'_>,
    source: &'source [u8],
) -> Option<&'source str> {
    let text = node.utf8_text(source).ok()?;
    match node.kind() {
        "word" => (!text
            .bytes()
            .any(|byte| matches!(byte, b'$' | b'`' | b'\\' | b'"' | b'\'')))
        .then_some(text),
        "raw_string" => text.strip_prefix('\'')?.strip_suffix('\''),
        "string" => {
            let value = text.strip_prefix('"')?.strip_suffix('"')?;
            (!value
                .bytes()
                .any(|byte| matches!(byte, b'$' | b'`' | b'\\')))
            .then_some(value)
        }
        _ => None,
    }
}

fn shell_variable(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn function_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | ':' | '.')
        })
}

fn source_line(node: Node<'_>) -> u32 {
    u32::try_from(node.start_position().row.saturating_add(1)).unwrap_or(u32::MAX)
}

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
            | "getopts"
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
