//! Tasks and services (`docs/spec.md` sections 5 and 6): Makefile targets, `package.json`
//! scripts and GitHub Actions jobs as `task` symbols, compose services and Kubernetes workloads as
//! `svc` symbols, Dockerfile stages as `task` symbols, with `runs` references to the scripts,
//! entrypoints and tasks they run.

use crate::domain::name::Name;
use std::borrow::Cow;

use super::json::{self, Value};
use crate::domain::facts::{
    self, Coverage, Facts, NO_ARITY, Ref, RefKind, SymKind, Symbol, ValueKind,
};

/// Script extensions a command may name.
const SCRIPT_EXTENSIONS: [&str; 11] = [
    ".sh", ".bash", ".py", ".js", ".mjs", ".cjs", ".ts", ".rb", ".pl", ".ps1", ".go",
];
const SCRIPT_INTERPRETERS: [&str; 20] = [
    "sh",
    "bash",
    "dash",
    "zsh",
    "ksh",
    "python",
    "python2",
    "python3",
    "node",
    "deno",
    "bun",
    "tsx",
    "vite-node",
    "ts-node",
    "ruby",
    "perl",
    "pwsh",
    "powershell",
    "pwsh.exe",
    "powershell.exe",
];
const COMMAND_WRAPPERS: [&str; 9] = [
    "command",
    "cross-env",
    "env",
    "exec",
    "nohup",
    "npx",
    "sudo",
    "time",
    "bunx",
];
const NYC_OPTIONS_WITH_VALUE: [&str; 7] = [
    "--exclude",
    "--include",
    "--extension",
    "--reporter",
    "--reporter-options",
    "--temp-dir",
    "--cache-dir",
];

fn push_symbol(
    facts: &mut Facts,
    kind: SymKind,
    name: &str,
    signature: String,
    line: u32,
    end_line: u32,
) -> u32 {
    facts.symbols.push(Symbol {
        kind: Some(kind),
        name: name.into(),
        line,
        end_line: end_line.max(line),
        signature: facts::clip_space(&signature, 160).into(),
        exported: true,
        flags: facts::FLAG_ENTRY,
        ..Symbol::default()
    });
    (facts.symbols.len() - 1) as u32
}

fn push_run(facts: &mut Facts, scope: Option<u32>, name: &str, qualifier: Option<&str>, line: u32) {
    let name = name.trim().trim_matches(|ch| ch == '"' || ch == '\'');
    if name.is_empty() || name.contains('$') {
        return;
    }
    let duplicate = facts.refs.iter().any(|reference| {
        reference.kind == RefKind::Run
            && reference.scope == scope
            && reference.name == name
            && reference.qualifier.as_deref() == qualifier
    });
    if !duplicate {
        facts.refs.push(Ref {
            kind: RefKind::Run,
            name: name.into(),
            qualifier: qualifier.map(Name::from),
            scope,
            line,
            arity: NO_ARITY,
            byte: crate::domain::facts::NO_BYTE,
        });
    }
}

fn is_script_path(path: &str) -> bool {
    let path = path.trim_matches(|ch| ch == '"' || ch == '\'');
    !path.starts_with('-')
        && !path.contains("://")
        && SCRIPT_EXTENSIONS
            .iter()
            .any(|extension| path.ends_with(extension))
}

fn is_shell_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CommandSyntax {
    Shell,
    Process,
}

fn command_index(words: &[&str], syntax: CommandSyntax) -> usize {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if syntax == CommandSyntax::Process && matches!(*word, "command" | "exec") {
            return words.len();
        }
        if (index > 0 && is_shell_assignment(word)) || COMMAND_WRAPPERS.contains(word) {
            index += 1;
        } else if *word == "nyc" {
            index += 1;
            while let Some(argument) = words.get(index) {
                if is_shell_assignment(argument) {
                    index += 1;
                    continue;
                }
                if argument.starts_with('-') {
                    let consumes_value =
                        NYC_OPTIONS_WITH_VALUE.contains(argument) && !argument.contains('=');
                    index += 1;
                    if consumes_value && index < words.len() {
                        index += 1;
                    }
                    continue;
                }
                break;
            }
        } else {
            break;
        }
    }
    index
}

fn command_program(command: &str) -> &str {
    command
        .trim_matches(|ch| ch == '"' || ch == '\'')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
}

fn interpreter_script<'a>(program: &str, args: &'a [&'a str]) -> Option<&'a str> {
    let mut index = 0;
    while let Some(option) = args.get(index).copied() {
        if !option.starts_with('-') {
            break;
        }
        if matches!(
            (program, option),
            ("sh" | "bash" | "dash" | "zsh" | "ksh", "-c" | "--command")
                | ("python" | "python2" | "python3", "-c" | "-m")
                | ("node", "-e" | "--eval" | "-p" | "--print")
                | ("ruby" | "perl", "-e")
                | (
                    "pwsh" | "pwsh.exe" | "powershell" | "powershell.exe",
                    "-Command" | "-c"
                )
        ) {
            return None;
        }
        if matches!(
            (program, option),
            (
                "node",
                "--require"
                    | "-r"
                    | "--import"
                    | "--loader"
                    | "--experimental-loader"
                    | "--test-reporter"
                    | "--test-name-pattern"
                    | "--conditions"
                    | "--input-type"
                    | "--env-file"
            ) | ("python" | "python2" | "python3", "-W" | "-X")
                | (
                    "sh" | "bash" | "dash" | "zsh" | "ksh",
                    "-o" | "-O" | "--rcfile" | "--init-file"
                )
                | ("tsx" | "ts-node" | "vite-node", "--tsconfig" | "--config")
        ) {
            index += 2;
            continue;
        }
        let boolean_option = match program {
            "sh" | "bash" | "dash" | "zsh" | "ksh" => {
                option.strip_prefix('-').is_some_and(|flags| {
                    !flags.is_empty()
                        && flags.chars().all(|flag| {
                            matches!(
                                flag,
                                'a' | 'b'
                                    | 'e'
                                    | 'f'
                                    | 'h'
                                    | 'k'
                                    | 'm'
                                    | 'n'
                                    | 'p'
                                    | 't'
                                    | 'u'
                                    | 'v'
                                    | 'x'
                                    | 'B'
                                    | 'C'
                                    | 'E'
                                    | 'H'
                                    | 'P'
                                    | 'T'
                            )
                        })
                })
            }
            "python" | "python2" | "python3" => matches!(
                option,
                "-B" | "-d"
                    | "-E"
                    | "-I"
                    | "-O"
                    | "-OO"
                    | "-P"
                    | "-q"
                    | "-s"
                    | "-S"
                    | "-u"
                    | "-v"
                    | "-V"
                    | "-VV"
                    | "-i"
            ),
            "node" => matches!(
                option,
                "--test"
                    | "--watch"
                    | "--no-warnings"
                    | "--trace-warnings"
                    | "--experimental-strip-types"
            ),
            _ => false,
        };
        if boolean_option || option.contains('=') {
            index += 1;
            continue;
        }
        return None;
    }
    args.get(index).copied().filter(|path| is_script_path(path))
}

fn invoked_script<'a>(program: &str, command: &'a str, args: &'a [&'a str]) -> Option<&'a str> {
    match program {
        "." | "source" => args.first().copied().filter(|path| is_script_path(path)),
        "bun" if args.first() == Some(&"test") => {
            args.get(1).copied().filter(|path| is_script_path(path))
        }
        "npm" if args.first() == Some(&"test") && args.get(1) == Some(&"--") => {
            args.get(2).copied().filter(|path| is_script_path(path))
        }
        "deno" if args.first() == Some(&"run") => interpreter_script(program, &args[1..]),
        _ if is_script_path(command) => Some(command),
        _ if SCRIPT_INTERPRETERS.contains(&program) => interpreter_script(program, args),
        _ => None,
    }
}

fn task_runner_refs(facts: &mut Facts, scope: Option<u32>, words: &[&str], line: u32) {
    let Some(program) = words.first().copied() else {
        return;
    };
    let args = &words[1..];
    let next = args.first().copied().unwrap_or_default();
    match program {
        "make" | "gmake" => {
            for target in args
                .iter()
                .take_while(|word| !word.starts_with('-') && !word.contains('='))
            {
                push_run(facts, scope, target, Some("make"), line);
            }
        }
        "npm" | "pnpm" | "bun" if matches!(next, "run" | "run-script") => {
            if let Some(script) = args.get(1) {
                if program == "bun" && is_script_path(script) {
                    push_run(facts, scope, script, None, line);
                } else {
                    push_run(facts, scope, script, Some("npm"), line);
                }
            }
        }
        "npm" if next == "test" => {
            push_run(facts, scope, "test", Some("npm"), line);
            if args.get(1) == Some(&"--")
                && let Some(script) = args.get(2).filter(|path| is_script_path(path))
            {
                push_run(facts, scope, script, None, line);
            }
        }
        "bun" if next == "test" => {
            if let Some(script) = args.get(1).filter(|path| is_script_path(path)) {
                push_run(facts, scope, script, None, line);
            }
        }
        "yarn"
            if !next.is_empty()
                && !next.starts_with('-')
                && !matches!(next, "install" | "add" | "remove") =>
        {
            let script = if next == "run" {
                args.get(1).copied().unwrap_or_default()
            } else {
                next
            };
            push_run(facts, scope, script, Some("npm"), line);
        }
        "go" if next == "run" => {
            if let Some(target) = args[1..].iter().find(|word| !word.starts_with('-')) {
                push_run(facts, scope, target, None, line);
            }
        }
        _ => {}
    }
}

fn bundle_pipeline_script<'a>(pipeline: &[Vec<&'a str>]) -> Option<&'a str> {
    let consumer_words = pipeline.last()?;
    let consumer_index = command_index(consumer_words, CommandSyntax::Shell);
    let consumer_program = command_program(consumer_words.get(consumer_index)?);
    if !matches!(consumer_program, "node" | "bun")
        || consumer_words[consumer_index + 1..]
            .iter()
            .any(|word| is_script_path(word))
    {
        return None;
    }
    let producer_words = pipeline.first()?;
    let producer_index = command_index(producer_words, CommandSyntax::Shell);
    if command_program(producer_words.get(producer_index)?) != "esbuild"
        || !producer_words[producer_index + 1..].contains(&"--bundle")
    {
        return None;
    }
    producer_words[producer_index + 1..]
        .iter()
        .copied()
        .find(|word| is_script_path(word))
}

/// Scripts, directories run with `go run`, Makefile targets (`make t`) and package scripts
/// (`npm run t`, `yarn t`, `pnpm t`) named in a shell command.
pub(super) fn command_runs(facts: &mut Facts, scope: Option<u32>, command: &str, line: u32) {
    use crate::domain::filetype::FileType;

    let Some(pack) = super::lang::all_packs()
        .iter()
        .find(|pack| pack.file_type == FileType::Bash)
    else {
        facts.coverage = facts.coverage.max(Coverage::NoGrammar);
        return;
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&(pack.language)("")).is_err() {
        facts.coverage = facts.coverage.max(Coverage::Failed);
        return;
    }
    let Some(tree) = parser.parse(command, None) else {
        facts.coverage = facts.coverage.max(Coverage::Failed);
        return;
    };
    let root = tree.root_node();
    if root.has_error() {
        facts.coverage = facts.coverage.max(Coverage::ParseErrors);
    }
    let source = command.as_bytes();
    let mut pending = vec![root];
    #[cfg(feature = "lang-bash")]
    let mut literals = Facts::default();
    while let Some(node) = pending.pop() {
        for index in (0..node.child_count()).rev() {
            if let Some(child) = node.child(index) {
                pending.push(child);
            }
        }
        if node.kind() == "command" {
            let words = shell_command_words(node, source);
            command_words_runs(facts, scope, &words, line, CommandSyntax::Shell);
        } else if node.kind() == "pipeline" {
            let mut cursor = node.walk();
            let pipeline: Vec<Vec<&str>> = node
                .named_children(&mut cursor)
                .filter(|child| child.kind() == "command")
                .map(|child| shell_command_words(child, source))
                .collect();
            if let Some(script) = bundle_pipeline_script(&pipeline) {
                push_run(facts, scope, script, None, line);
            }
        } else if matches!(node.kind(), "raw_string" | "string") {
            #[cfg(feature = "lang-bash")]
            if let Some(value) = super::lang::bash::literal_value(node, source) {
                super::assets::literal_paths(value, &mut literals);
                for mut reference in literals.refs.drain(..) {
                    if !facts.refs.iter().any(|prior| {
                        prior.scope == scope
                            && prior.name == reference.name
                            && matches!(prior.kind, RefKind::Run | RefKind::Path)
                    }) {
                        reference.scope = scope;
                        reference.line = line;
                        facts.refs.push(reference);
                    }
                }
            }
        }
    }
}

fn command_words_runs(
    facts: &mut Facts,
    scope: Option<u32>,
    words: &[&str],
    line: u32,
    syntax: CommandSyntax,
) {
    let index = command_index(words, syntax);
    let Some(command) = words.get(index).copied() else {
        return;
    };
    let program = command_program(command);
    if syntax == CommandSyntax::Process {
        if matches!(program, "." | "source") {
            return;
        }
        if matches!(program, "sh" | "bash" | "dash" | "zsh" | "ksh")
            && words
                .get(index + 1)
                .is_some_and(|option| matches!(*option, "-c" | "--command"))
        {
            if let Some(source) = words.get(index + 2) {
                command_runs(facts, scope, source, line);
            }
            return;
        }
    }
    task_runner_refs(facts, scope, &words[index..], line);
    if let Some(script) = invoked_script(program, command, &words[index + 1..]) {
        push_run(facts, scope, script, None, line);
    }
}

fn shell_command_words<'source>(
    node: tree_sitter::Node<'_>,
    source: &'source [u8],
) -> Vec<&'source str> {
    let literal = |node: tree_sitter::Node<'_>| {
        #[cfg(feature = "lang-bash")]
        {
            super::lang::bash::literal_value(node, source)
        }
        #[cfg(not(feature = "lang-bash"))]
        {
            let _ = (node, source);
            None
        }
    };
    let Some(name) = node.child_by_field_name("name") else {
        return Vec::new();
    };
    let name = name.named_child(0).and_then(literal);
    let Some(name) = name else { return Vec::new() };
    let mut words = vec![name];
    let mut cursor = node.walk();
    // An unknown argument keeps its slot, so option values cannot become executable positions.
    words.extend(
        node.children_by_field_name("argument", &mut cursor)
            .map(|argument| literal(argument).unwrap_or_default()),
    );
    words
}

/// Make expands before shell quoting: unknown values stay dynamic, never literal targets.
fn make_shell_source(command: &str) -> Option<Cow<'_, str>> {
    if !command.contains('$') {
        return Some(Cow::Borrowed(command));
    }
    let mut shell = String::with_capacity(command.len());
    let mut characters = command.char_indices().peekable();
    while let Some((_, character)) = characters.next() {
        if character != '$' {
            shell.push(character);
            continue;
        }
        match characters.peek().map(|(_, character)| *character) {
            Some('$') => {
                characters.next();
                shell.push('$');
            }
            Some(open @ ('(' | '{')) => {
                let (start, _) = characters.next()?;
                let close = if open == '(' { ')' } else { '}' };
                let mut depth = 1;
                let mut end = None;
                for (index, character) in characters.by_ref() {
                    if character == open {
                        depth += 1;
                    } else if character == close {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(index);
                            break;
                        }
                    }
                }
                let name = &command[start + 1..end?];
                shell.push_str(if name == "MAKE" { "make" } else { "${_}" });
            }
            _ => shell.push('$'),
        }
    }
    Some(Cow::Owned(shell))
}

/// Makefile targets with prerequisites and the commands of their recipe.
pub fn makefile(text: &str, facts: &mut Facts) {
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let raw = lines[index];
        let line = index as u32 + 1;
        index += 1;
        if raw.starts_with('\t') || raw.starts_with('#') || raw.trim().is_empty() {
            continue;
        }
        let Some((head, prerequisites)) = raw.split_once(':') else {
            continue;
        };
        if prerequisites.starts_with('=')
            || head.contains('=')
            || head.trim().starts_with('.')
            || head.contains('$')
        {
            continue;
        }
        let targets: Vec<&str> = head.split_whitespace().collect();
        let mut end = index;
        while end < lines.len() && (lines[end].starts_with('\t') || lines[end].trim().is_empty()) {
            end += 1;
        }
        let recipe: Vec<(u32, &str)> = (index..end)
            .filter(|&recipe| lines[recipe].starts_with('\t'))
            .map(|recipe| (recipe as u32 + 1, lines[recipe].trim()))
            .collect();
        let end_line = recipe.last().map_or(line, |(number, _)| *number);
        for target in targets {
            let signature = format!(
                "{target}:{}",
                prerequisites.split('#').next().unwrap_or_default()
            );
            let scope = push_symbol(facts, SymKind::Task, target, signature, line, end_line);
            for prerequisite in prerequisites
                .split('#')
                .next()
                .unwrap_or_default()
                .split_whitespace()
            {
                if !prerequisite.contains('$') && prerequisite != "|" {
                    push_run(facts, Some(scope), prerequisite, None, line);
                }
            }
            for (number, command) in &recipe {
                let Some(command) = make_shell_source(command.trim_start_matches(['@', '-', '+']))
                else {
                    facts.coverage = facts.coverage.max(Coverage::ParseErrors);
                    continue;
                };
                command_runs(facts, Some(scope), &command, *number);
            }
        }
        index = end;
    }
}

/// `package.json` scripts: one task per script, signature `name: command`.
pub fn package_scripts(text: &str, facts: &mut Facts) {
    let Some(value) = json::parse(text) else {
        return;
    };
    for (name, line, command) in value.get("scripts").map(Value::entries).unwrap_or_default() {
        let Some(command) = command.as_str() else {
            continue;
        };
        let scope = push_symbol(
            facts,
            SymKind::Task,
            name,
            format!("{name}: {command}"),
            *line,
            *line,
        );
        command_runs(facts, Some(scope), command, *line);
    }
}

/// Dockerfile stages as tasks (`FROM image [AS name]`, signature with `EXPOSE` ports) running the
/// scripts of their `RUN`, `CMD` and `ENTRYPOINT` lines; exec-form arrays included.
pub fn dockerfile(text: &str, facts: &mut Facts) {
    let lines: Vec<&str> = text.lines().collect();
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.trim_start().to_ascii_uppercase().starts_with("FROM "))
        .map(|(index, _)| index)
        .collect();
    for (position, &start) in starts.iter().enumerate() {
        let end = starts.get(position + 1).map_or(lines.len(), |&next| next);
        let words: Vec<&str> = lines[start].split_whitespace().collect();
        let image = words
            .iter()
            .skip(1)
            .find(|word| !word.starts_with("--"))
            .copied()
            .unwrap_or_default();
        let alias = words
            .iter()
            .position(|word| word.eq_ignore_ascii_case("as"))
            .and_then(|index| words.get(index + 1))
            .copied();
        let exposed: Vec<&str> = lines[start..end]
            .iter()
            .filter(|line| {
                line.trim_start()
                    .to_ascii_uppercase()
                    .starts_with("EXPOSE ")
            })
            .flat_map(|line| line.split_whitespace().skip(1))
            .collect();
        let name = alias.unwrap_or_else(|| {
            image
                .rsplit('/')
                .next()
                .unwrap_or(image)
                .split([':', '@'])
                .next()
                .unwrap_or(image)
        });
        let mut signature = format!("FROM {image}");
        if let Some(alias) = alias {
            signature.push_str(&format!(" AS {alias}"));
        }
        if !exposed.is_empty() {
            signature.push_str(&format!(" EXPOSE {}", exposed.join(" ")));
        }
        let scope = push_symbol(
            facts,
            SymKind::Task,
            name,
            signature,
            start as u32 + 1,
            end as u32,
        );
        for (index, raw) in lines.iter().enumerate().take(end).skip(start + 1) {
            let trimmed = raw.trim();
            let instruction = trimmed
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            if matches!(instruction.as_str(), "RUN" | "CMD" | "ENTRYPOINT") {
                let command = trimmed[instruction.len()..].replace(['[', ']', ','], " ");
                command_runs(facts, Some(scope), &command, index as u32 + 1);
            }
        }
    }
}

/// GitHub Actions jobs (`needs`, step `run` commands), compose services (`depends_on`, `build`,
/// `command`, `entrypoint`) and Kubernetes workloads.
pub fn yaml(path: &str, text: &str, facts: &mut Facts) {
    let name = path.rsplit('/').next().unwrap_or(path);
    let workflow = path.contains(".github/workflows/");
    let compose = name.starts_with("docker-compose") || name.starts_with("compose.");
    let kubernetes =
        !workflow && !compose && text.contains("apiVersion:") && text.contains("kind:");
    if !(workflow || compose || kubernetes) {
        return;
    }
    let (entries, coverage) = super::yaml::parse(text);
    facts.coverage = facts.coverage.max(coverage);
    let mut current: Option<u32> = None;
    let mut kind: Option<&str> = None;
    let mut document = 0;
    let source_lines: Vec<&str> = text.lines().collect();
    for (position, entry) in entries.iter().enumerate() {
        if entry.document != document {
            current = None;
            kind = None;
            document = entry.document;
        }
        let depth = entry.path.len();
        let under = |root: &str| depth >= 1 && entry.path[0] == root;
        if entry.kind == Some(ValueKind::Object)
            && ((workflow && depth == 1 && entry.path[0] == "jobs")
                || (compose && depth == 1 && entry.path[0] == "services"))
        {
            let symbol_kind = if workflow {
                SymKind::Task
            } else {
                SymKind::Svc
            };
            let label = if workflow { "job" } else { "service" };
            current = Some(push_symbol(
                facts,
                symbol_kind,
                &entry.key,
                format!("{label} {}", entry.key),
                entry.line,
                entry.end,
            ));
            continue;
        }
        if kubernetes {
            if depth == 0 && entry.key == "kind" {
                kind = Some(entry.value.as_str());
            }
            if matches!(
                kind,
                Some("Deployment" | "StatefulSet" | "DaemonSet" | "Job" | "CronJob" | "Pod")
            ) && depth == 1
                && entry.path[0] == "metadata"
                && entry.key == "name"
                && entry.kind == Some(ValueKind::String)
            {
                let label = kind.unwrap_or_default().to_ascii_lowercase();
                push_symbol(
                    facts,
                    SymKind::Svc,
                    &entry.value,
                    format!("{label} {}", entry.value),
                    entry.line,
                    entry.line,
                );
            }
            continue;
        }
        let Some(scope) = current else { continue };
        let before = facts.refs.len();
        if workflow && under("jobs") {
            match entry.key.as_str() {
                "needs" => push_run(facts, Some(scope), &entry.value, None, entry.line),
                "" if entry.path.last().is_some_and(|key| key == "[]")
                    && entry
                        .path
                        .iter()
                        .rev()
                        .nth(1)
                        .is_some_and(|key| key == "needs") =>
                {
                    push_run(facts, Some(scope), &entry.value, None, entry.line);
                }
                "run" if entry.kind == Some(ValueKind::String) => {
                    command_runs(facts, Some(scope), &entry.value, entry.line);
                }
                _ => {}
            }
        }
        if compose && under("services") {
            match entry.key.as_str() {
                "command" | "entrypoint" => {
                    compose_command_runs(facts, Some(scope), entry, &entries[position + 1..]);
                }
                "build" if entry.kind == Some(ValueKind::String) && !entry.value.is_empty() => {
                    push_run(
                        facts,
                        Some(scope),
                        &format!("{}/Dockerfile", entry.value.trim_end_matches('/')),
                        None,
                        entry.line,
                    );
                }
                "dockerfile" => push_run(facts, Some(scope), &entry.value, None, entry.line),
                "" if entry.path.last().is_some_and(|key| key == "[]")
                    && entry
                        .path
                        .iter()
                        .rev()
                        .nth(1)
                        .is_some_and(|key| key == "depends_on") =>
                {
                    push_run(facts, Some(scope), &entry.value, None, entry.line);
                }
                key if depth >= 3
                    && !key.is_empty()
                    && entry.path.last().is_some_and(|key| key == "depends_on") =>
                {
                    push_run(facts, Some(scope), key, None, entry.line);
                }
                _ => {}
            }
        }
        for reference in &mut facts.refs[before..] {
            if reference.kind != RefKind::Path {
                continue;
            }
            let start = entry.line.saturating_sub(1) as usize;
            let end = (entry.end as usize).min(source_lines.len());
            if let Some(relative) = source_lines.get(start..end).and_then(|lines| {
                lines
                    .iter()
                    .position(|line| line.contains(reference.name.as_str()))
            }) {
                reference.line = (start + relative + 1) as u32;
            }
        }
    }
}

fn compose_command_runs(
    facts: &mut Facts,
    scope: Option<u32>,
    entry: &super::yaml::Entry,
    following: &[super::yaml::Entry],
) {
    match entry.kind {
        Some(ValueKind::String) => {
            let Some(arguments) = shlex::split(&entry.value) else {
                facts.coverage = facts.coverage.max(Coverage::ParseErrors);
                return;
            };
            let words: Vec<&str> = arguments.iter().map(String::as_str).collect();
            command_words_runs(facts, scope, &words, entry.line, CommandSyntax::Process);
        }
        Some(ValueKind::Array) => {
            let mut path = entry.path.clone();
            path.extend([entry.key.clone(), "[]".to_owned()]);
            let words: Vec<&str> = following
                .iter()
                .take_while(|item| item.document == entry.document && item.path.starts_with(&path))
                .filter(|item| item.path == path && item.key.is_empty())
                .map(|item| {
                    if item.kind == Some(ValueKind::String) {
                        item.value.as_str()
                    } else {
                        ""
                    }
                })
                .collect();
            command_words_runs(facts, scope, &words, entry.line, CommandSyntax::Process);
        }
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn summary(facts: &Facts) -> (Vec<String>, Vec<String>) {
        let symbols = facts
            .symbols
            .iter()
            .map(|symbol| {
                format!(
                    "{}-{} {:?} {}",
                    symbol.line,
                    symbol.end_line,
                    symbol.kind(),
                    symbol.name
                )
            })
            .collect();
        let runs = facts
            .refs
            .iter()
            .filter(|reference| reference.kind == RefKind::Run)
            .map(|reference| {
                let scope = reference.scope.map_or("-".to_owned(), |scope| {
                    facts.symbols[scope as usize].name.to_string()
                });
                format!(
                    "{} {scope} -> {}{}",
                    reference.line,
                    reference
                        .qualifier
                        .as_deref()
                        .map_or(String::new(), |q| format!("{q}:")),
                    reference.name
                )
            })
            .collect();
        (symbols, runs)
    }

    #[test]
    fn shell_command_nodes_preserve_literals_and_real_nested_invocations() {
        let mut facts = Facts::default();
        command_runs(
            &mut facts,
            None,
            concat!(
                "echo 'note; ./mentioned.sh'\n",
                "printf '%s' 'note | ./also-mentioned.sh'\n",
                "\"if\" ./keyword-argument.sh\n",
                "\"X=command.sh\" ./assignment-argument.sh\n",
                "./actual.sh\n",
                "node 'path with spaces.js'\n",
                "echo \"$(./substitution.sh)\"\n",
                "$(./select-command.sh) ignored\n",
                "esbuild --bundle 'source entry.ts' | node\n",
                "# ./comment.sh\n",
                "cat <<'EOF'\n./heredoc.sh\nEOF\n",
            ),
            7,
        );
        let (_, mut runs) = summary(&facts);
        runs.sort();
        assert_eq!(
            runs,
            [
                "7 - -> ./actual.sh",
                "7 - -> ./select-command.sh",
                "7 - -> ./substitution.sh",
                "7 - -> X=command.sh",
                "7 - -> path with spaces.js",
                "7 - -> source entry.ts",
            ]
        );
    }

    #[test]
    fn yaml_scalars_and_make_expansions_preserve_language_boundaries() {
        let mut workflow = Facts::default();
        yaml(
            ".github/workflows/ci.yml",
            concat!(
                "jobs:\n  ci:\n    steps:\n",
                "      - run: '. ./nvm.sh ; npm run lint'\n",
                "      - run: \"node \\\"scripts/build file.js\\\"\"\n",
                "      - run: 'echo ''note; ./quoted.sh'''\n",
            ),
            &mut workflow,
        );
        let (_, mut runs) = summary(&workflow);
        runs.sort();
        assert_eq!(
            runs,
            [
                "4 ci -> ./nvm.sh",
                "4 ci -> npm:lint",
                "5 ci -> scripts/build file.js",
            ]
        );

        let mut make = Facts::default();
        makefile(
            concat!(
                "help:\n\t$(error Please use `make list` or `npm test`)\n",
                "live:\n\t$(MAKE) list\n\techo \"$$(./real.sh)\"\n",
                "list:\n\techo list\n",
            ),
            &mut make,
        );
        let (_, runs) = summary(&make);
        assert_eq!(runs, ["4 live -> make:list", "5 live -> ./real.sh"]);
    }

    #[test]
    fn yaml_folded_literal_and_quoted_scalars_preserve_shell_commands() {
        let mut facts = Facts::default();
        yaml(
            ".github/workflows/ci.yml",
            concat!(
                "jobs:\n",
                "  folded:\n    steps:\n      - run: >\n",
                "          echo harmless\n          ./mentioned.sh\n",
                "  paragraph:\n    steps:\n      - run: >\n",
                "          echo harmless\n\n          ./paragraph.sh\n",
                "  literal:\n    steps:\n      - run: |2-\n          ./literal.sh\n",
                "  quoted:\n    steps:\n      - run: 'echo \" # text\"; ./quoted.sh'\n",
            ),
            &mut facts,
        );
        let mut runs: Vec<&str> = facts
            .refs
            .iter()
            .map(|reference| reference.name.as_str())
            .collect();
        runs.sort_unstable();
        assert_eq!(runs, ["./literal.sh", "./paragraph.sh", "./quoted.sh"]);
        assert_eq!(facts.coverage, Coverage::Complete);
    }

    #[test]
    fn compose_process_arguments_do_not_execute_shell_operators() {
        let mut facts = Facts::default();
        yaml(
            "compose.yml",
            concat!(
                "services:\n",
                "  inline:\n    command: ./actual.sh && ./mentioned.sh\n",
                "  vector:\n    entrypoint: [echo, 'note; ./mentioned.sh']\n",
                "  direct:\n    command: ['./literal, path.sh', ./mentioned.sh]\n",
                "  shell:\n    command: [sh, '-c', './actual.sh && ./other.sh']\n",
                "  builtin:\n    command: [source, ./mentioned.sh]\n",
            ),
            &mut facts,
        );
        let (_, runs) = summary(&facts);
        assert_eq!(
            runs,
            [
                "3 inline -> ./actual.sh",
                "7 direct -> ./literal, path.sh",
                "9 shell -> ./actual.sh",
                "9 shell -> ./other.sh",
            ]
        );
        assert_eq!(facts.coverage, Coverage::Complete);

        let mut aliases = Facts::default();
        yaml(
            "compose.yml",
            "base: &base ./unknown.sh\nservices:\n  app:\n    command: [*base, ./mentioned.sh]\n",
            &mut aliases,
        );
        assert!(aliases.refs.is_empty());
        assert_eq!(aliases.coverage, Coverage::NoGrammar);
    }

    #[test]
    fn dockerfile_stages_with_exposed_ports_and_run_scripts() {
        let mut facts = Facts::default();
        dockerfile(
            "FROM --platform=$B golang:1.22 AS build\nRUN go build ./cmd/server\nFROM gcr.io/distroless/base\nEXPOSE 8080\nENTRYPOINT [\"/app/start.sh\"]\n",
            &mut facts,
        );
        let (symbols, runs) = summary(&facts);
        assert_eq!(symbols, ["1-2 Task build", "3-5 Task base"]);
        assert_eq!(
            facts.symbols[1].signature,
            "FROM gcr.io/distroless/base EXPOSE 8080"
        );
        assert_eq!(runs, ["5 base -> /app/start.sh"]);
    }

    #[test]
    fn makefile_targets_prerequisites_and_recipe_commands() {
        let mut facts = Facts::default();
        makefile(
            ".PHONY: test\nVERSION := 1\n\nbuild: gen ## compile\n\tgo build ./...\n\t./scripts/stamp.sh $(VERSION)\n\ntest: build\n\t$(MAKE) lint\n\tgo run ./cmd/check -v\n",
            &mut facts,
        );
        let (symbols, runs) = summary(&facts);
        assert_eq!(symbols, ["4-6 Task build", "8-10 Task test"]);
        assert_eq!(
            runs,
            [
                "4 build -> gen",
                "6 build -> ./scripts/stamp.sh",
                "8 test -> build",
                "9 test -> make:lint",
                "10 test -> ./cmd/check"
            ]
        );
    }

    #[test]
    fn package_scripts_workflows_and_compose_services() {
        let mut facts = Facts::default();
        package_scripts(
            "{\n  \"scripts\": {\n    \"build\": \"tsc && node scripts/bundle.mjs\",\n    \"test\": \"npm run build && vitest\"\n  }\n}",
            &mut facts,
        );
        let (symbols, runs) = summary(&facts);
        assert_eq!(symbols, ["3-3 Task build", "4-4 Task test"]);
        assert_eq!(
            runs,
            ["3 build -> scripts/bundle.mjs", "4 test -> npm:build"]
        );

        let mut workflow = Facts::default();
        yaml(
            ".github/workflows/ci.yml",
            "on: push\njobs:\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: make lint\n  test:\n    needs: [lint]\n    steps:\n      - uses: actions/checkout@v4\n      - run: |\n          ./scripts/test.sh --all\n          npm run e2e\n",
            &mut workflow,
        );
        let (symbols, runs) = summary(&workflow);
        assert_eq!(symbols, ["3-6 Task lint", "7-13 Task test"]);
        assert_eq!(
            runs,
            [
                "6 lint -> make:lint",
                "8 test -> lint",
                "11 test -> ./scripts/test.sh",
                "11 test -> npm:e2e"
            ]
        );

        let mut compose = Facts::default();
        yaml(
            "docker-compose.yml",
            "services:\n  api:\n    build: ./api\n    command: [\"./bin/serve.sh\", \"--port\", \"80\"]\n    depends_on:\n      - db\n  db:\n    image: postgres\n",
            &mut compose,
        );
        let (symbols, runs) = summary(&compose);
        assert_eq!(symbols, ["2-6 Svc api", "7-8 Svc db"]);
        assert_eq!(
            runs,
            [
                "3 api -> ./api/Dockerfile",
                "4 api -> ./bin/serve.sh",
                "6 api -> db"
            ]
        );
    }

    #[test]
    fn yarn_scripts_and_go_run_name_their_targets() {
        let mut facts = Facts::default();
        yaml(
            ".github/workflows/ci.yml",
            concat!(
                "jobs:\n",
                "  ci:\n",
                "    steps:\n",
                "      - run: |\n",
                "          yarn build\n",
                "          yarn run lint\n",
                "          yarn install\n",
                "          yarn add left-pad\n",
                "          yarn --frozen-lockfile\n",
                "          go run ./cmd/tool\n",
            ),
            &mut facts,
        );
        let (_, mut runs) = summary(&facts);
        runs.sort();
        assert_eq!(
            runs,
            [
                "4 ci -> ./cmd/tool",
                "4 ci -> npm:build",
                "4 ci -> npm:lint"
            ]
        );
    }

    #[test]
    fn workflow_copying_a_script_does_not_emit_a_run() {
        let mut facts = Facts::default();
        yaml(
            ".github/workflows/release.yml",
            concat!(
                "jobs:\n",
                "  release:\n",
                "    steps:\n",
                "      - run: |\n",
                "          cp ../scripts/utils/install.sh install.sh\n",
                "          echo nvm.sh sourced\n",
                "          echo npm test\n",
                "          echo make lint\n",
                "          echo go run ./cmd/main.go\n",
                "          bash -e scripts/tests/test.sh\n",
                "          source scripts/utils/environment.sh\n",
                "          . ./nvm.sh\n",
                "          sudo ./install.sh /usr/ lib64\n",
                "          nohup npm run server >/dev/null 2>&1 &\n",
                "          if ! ./.github/workflows/check_pr_label.sh 1 no; then echo no; fi\n",
                "          GRAPHIFY_TEST_CLI=graphify bun test tests/pi.test.ts\n",
                "          bun run src/benchmark.ts\n",
                "          node --test test/browser.ts\n",
                "          npm test -- test/browser.ts\n",
                "          tsx ./mock-server.ts\n",
                "          vite-node ./server.ts\n",
                "          node --require ./preload.js scripts/app.js\n",
                "          vitest run --config vitest.config.js --project module\n",
                "          nyc --reporter=text npm test\n",
                "          esbuild --bundle src/main.ts | node\n",
                "          VERSION=$(node -p \"require('./package.json').version\")\n",
            ),
            &mut facts,
        );

        let (_, mut runs) = summary(&facts);
        runs.sort();
        let mut expected = [
            "4 release -> scripts/tests/test.sh",
            "4 release -> scripts/utils/environment.sh",
            "4 release -> ./nvm.sh",
            "4 release -> ./install.sh",
            "4 release -> npm:server",
            "4 release -> ./.github/workflows/check_pr_label.sh",
            "4 release -> tests/pi.test.ts",
            "4 release -> src/benchmark.ts",
            "4 release -> test/browser.ts",
            "4 release -> npm:test",
            "4 release -> ./mock-server.ts",
            "4 release -> ./server.ts",
            "4 release -> scripts/app.js",
            "4 release -> src/main.ts",
        ]
        .map(str::to_owned)
        .to_vec();
        expected.sort();
        assert_eq!(runs, expected);
        assert!(facts.refs.iter().any(|reference| {
            reference.kind == RefKind::Path
                && reference.name == "./package.json"
                && reference.line == 26
                && reference.scope == Some(0)
        }));
    }
}
