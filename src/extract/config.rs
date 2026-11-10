//! Config facts (`docs/spec.md` section 5): dotted key paths (max depth 6) with value kind and
//! line for JSON, YAML, TOML, XML, INI and properties files; env definitions (names only) from
//! `.env.example`, Dockerfiles, compose files, GitHub Actions workflows and Kubernetes manifests.
//! Values are never stored.

use std::collections::HashSet;

use super::json::{self, KeyVisitor, Kind};
use crate::domain::facts::{
    ConfigKey, Coverage, EnvDef, EnvRead, EnvSource, Facts, NO_ARITY, Ref, RefKind, ValueKind,
};
use crate::domain::filetype::{self, Detection, FileType};

const MAX_DEPTH: usize = 6;
/// Key paths kept per file: large data files would flood the search index.
const MAX_KEYS: usize = 400;

/// Fills `config_keys` and `env_defs`; a valid JSON object returns whether it has project options.
pub fn extract(path: &str, file_type: FileType, text: &str, facts: &mut Facts) -> Option<bool> {
    let candidate = if file_type == FileType::Json {
        json_keys(text, facts)
    } else {
        match file_type {
            FileType::Yaml => yaml(path, text, facts),
            FileType::Toml => toml_keys(text, facts),
            FileType::Ini | FileType::Properties => ini_keys(text, facts),
            FileType::Text => server_keys(text, facts),
            FileType::Xml => xml_keys(text, facts),
            FileType::EnvExample => env_example(text, facts),
            FileType::Dockerfile => dockerfile(text, facts),
            _ => {}
        }
        None
    };
    let mut seen = HashSet::with_capacity(facts.config_keys.len());
    let first: Vec<bool> = facts
        .config_keys
        .iter()
        .map(|key| seen.insert(key.path.as_str()))
        .collect();
    let mut first = first.into_iter();
    facts.config_keys.retain(|_| first.next().unwrap_or(false));
    facts.config_keys.truncate(MAX_KEYS);
    candidate
}

fn push_key(facts: &mut Facts, path: &[String], kind: ValueKind, line: u32) {
    if path.is_empty() || path.len() > MAX_DEPTH {
        return;
    }
    facts.config_keys.push(ConfigKey {
        path: path.join("."),
        kind,
        line,
    });
}

/// A complete scalar path, rather than an arbitrary word inside prose or a command.
fn path_value(facts: &mut Facts, value: &str, line: u32) {
    let value = value.trim().trim_end_matches(';').trim_end();
    let quoted = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        });
    let value = quoted.unwrap_or(value);
    if quoted.is_none() && value.chars().any(char::is_whitespace) {
        return;
    }
    path_literal(facts, value, line);
}

/// Already decoded scalar text, including quoted paths with spaces.
fn path_literal(facts: &mut Facts, value: &str, line: u32) {
    if value.is_empty()
        || value.contains(['$', '%', '*', '?', '#', '\\', '\n', '\r', '"', '\''])
        || value.contains("://")
        || value.chars().any(char::is_control)
        || (!value.contains('/')
            && !matches!(
                filetype::detect(value),
                Detection::Known(_) | Detection::Ambiguous(_)
            ))
        || !value.rsplit_once('.').is_some_and(|(stem, extension)| {
            !stem.is_empty()
                && !stem.ends_with('/')
                && !extension.is_empty()
                && extension
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
                && extension
                    .chars()
                    .any(|character| character.is_ascii_alphabetic())
        })
    {
        return;
    }
    facts.refs.push(Ref {
        kind: RefKind::Path,
        name: value.into(),
        qualifier: None,
        scope: None,
        line,
        arity: NO_ARITY,
        byte: crate::domain::facts::NO_BYTE,
    });
}

/// Config paths cannot escape the indexed root, even if a root-relative fallback exists.
pub(super) fn retain_in_root_paths(path: &str, file_type: FileType, facts: &mut Facts) {
    // Each literal once, at its first line; one set instead of a scan per value.
    let mut seen = std::collections::HashSet::new();
    let mut refs = std::mem::take(&mut facts.refs);
    refs.retain(|reference| {
        reference.kind != RefKind::Path || reference.scope.is_some() || seen.insert(reference.name)
    });
    facts.refs = refs;
    facts.refs.retain(|reference| {
        if reference.kind != RefKind::Path {
            return true;
        }
        if reference.name.starts_with('/') {
            return file_type != FileType::Text;
        }
        let mut depth = path.bytes().filter(|byte| *byte == b'/').count();
        for part in reference.name.split('/') {
            match part {
                "" | "." => {}
                ".." if depth == 0 => return false,
                ".." => depth -= 1,
                _ => depth += 1,
            }
        }
        true
    });
}

/// Server text configs retain INI sections or literal directive names; values stay out of keys.
fn server_keys(text: &str, facts: &mut Facts) {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with(['#', ';']))
        .unwrap_or_default();
    if !first.starts_with('[')
        && !first.contains('=')
        && (first.ends_with(';') || first.contains('{'))
    {
        server_directives(text, facts);
        return;
    }
    let mut section: Option<&str> = None;
    let mut continued = false;
    for (index, raw) in text.lines().enumerate() {
        let line = index as u32 + 1;
        let text = raw.trim();
        if text.is_empty() || text.starts_with(['#', ';']) {
            continue;
        }
        if continued {
            continued = text.ends_with('\\');
            continue;
        }
        continued = text.ends_with('\\');
        if let Some(name) = text
            .strip_prefix('[')
            .and_then(|text| text.strip_suffix(']'))
        {
            section = Some(name.trim());
            push_key(facts, &[name.trim().to_owned()], ValueKind::Object, line);
            continue;
        }
        if text == "}" || text == "};" {
            continue;
        }
        let Some(split) =
            text.find(|character: char| character == '=' || character.is_whitespace())
        else {
            facts.coverage = facts.coverage.max(Coverage::NoGrammar);
            continue;
        };
        let key = &text[..split];
        if key.is_empty()
            || !key.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            })
        {
            facts.coverage = facts.coverage.max(Coverage::NoGrammar);
            continue;
        }
        let value = text[split..]
            .trim_start()
            .trim_start_matches('=')
            .trim_start();
        let mut path: Vec<String> = section.into_iter().map(str::to_owned).collect();
        path.push(key.to_owned());
        push_key(
            facts,
            &path,
            if value == "{" {
                ValueKind::Object
            } else {
                scalar_kind(value.trim_end_matches(';'))
            },
            line,
        );
        if !continued {
            path_value(facts, value, line);
        }
        // Inline blocks and multiple directives need a grammar; do not invent nested keys.
        if value.contains(['{', '}']) && value != "{" {
            facts.coverage = facts.coverage.max(Coverage::NoGrammar);
        }
    }
}

/// Semicolon/block configuration syntax: newlines inside a directive are not new keys.
fn server_directives(text: &str, facts: &mut Facts) {
    let mut parents: Vec<String> = Vec::new();
    let mut statement = String::new();
    let (mut line, mut start) = (1, 1);
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut expansion = false;
    for character in text.chars() {
        if comment {
            if character == '\n' {
                comment = false;
                if !statement.is_empty() {
                    statement.push(' ');
                }
                line += 1;
            }
            continue;
        }
        if statement.is_empty() {
            if character.is_whitespace() {
                if character == '\n' {
                    line += 1;
                }
                continue;
            }
            start = line;
        }
        if escaped {
            statement.push(character);
            escaped = false;
        } else if character == '\\' {
            statement.push(character);
            escaped = true;
        } else if let Some(delimiter) = quote {
            statement.push(character);
            if character == delimiter {
                quote = None;
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
            statement.push(character);
        } else if expansion {
            statement.push(character);
            if character == '}' {
                expansion = false;
            }
        } else if character == '{' && statement.ends_with('$') {
            expansion = true;
            statement.push(character);
        } else if character == '#' {
            comment = true;
        } else if matches!(character, ';' | '{') {
            let body = statement.trim();
            let split = body.find(char::is_whitespace).unwrap_or(body.len());
            let key = &body[..split];
            if key.is_empty() || key.contains(['$', '\\', '"', '\'']) {
                facts.coverage = facts.coverage.max(Coverage::NoGrammar);
                return;
            } else {
                let value = body[split..].trim();
                parents.push(key.to_owned());
                push_key(
                    facts,
                    &parents,
                    if character == '{' {
                        ValueKind::Object
                    } else {
                        scalar_kind(value)
                    },
                    start,
                );
                if character == ';' {
                    parents.pop();
                    path_value(facts, value, start);
                }
            }
            statement.clear();
        } else if character == '}' {
            if !statement.trim().is_empty() || parents.pop().is_none() {
                facts.coverage = facts.coverage.max(Coverage::ParseErrors);
            }
            statement.clear();
        } else {
            statement.push(character);
        }
        if character == '\n' {
            line += 1;
        }
    }
    if quote.is_some()
        || escaped
        || expansion
        || !statement.trim().is_empty()
        || !parents.is_empty()
    {
        facts.coverage = facts.coverage.max(Coverage::ParseErrors);
    }
}

fn push_env(facts: &mut Facts, name: &str, source: EnvSource, line: u32) {
    let name = name.trim().trim_matches(|ch| ch == '"' || ch == '\'');
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && !name.starts_with(|ch: char| ch.is_ascii_digit());
    if valid
        && !facts
            .env_defs
            .iter()
            .any(|def| def.name == name && def.source == source)
    {
        facts.env_defs.push(EnvDef {
            name: name.to_owned(),
            source,
            line,
        });
    }
}

fn json_keys(text: &str, facts: &mut Facts) -> Option<bool> {
    let before = facts.config_keys.len();
    let before_refs = facts.refs.len();
    let mut walk = JsonWalk {
        path: String::new(),
        lengths: Vec::new(),
        seen: HashSet::new(),
        facts,
        project_candidate: false,
    };
    let kind = json::walk_keys_complete(text, &mut walk);
    let candidate = walk.project_candidate;
    if kind.is_none() {
        facts.config_keys.truncate(before);
        facts.refs.truncate(before_refs);
        facts.coverage = facts.coverage.max(Coverage::ParseErrors);
        return None;
    }
    (kind == Some(Kind::Object)).then_some(candidate)
}

fn json_kind(kind: Kind) -> ValueKind {
    match kind {
        Kind::Null => ValueKind::Null,
        Kind::Bool => ValueKind::Bool,
        Kind::Number => ValueKind::Number,
        Kind::Str => ValueKind::String,
        Kind::Array => ValueKind::Array,
        Kind::Object => ValueKind::Object,
    }
}

/// Keeps the first [`MAX_KEYS`] distinct paths down to [`MAX_DEPTH`] in document order; repeated
/// keys of a large data file cost no string.
struct JsonWalk<'a> {
    path: String,
    /// Path length before each open segment.
    lengths: Vec<usize>,
    seen: HashSet<u64>,
    facts: &'a mut Facts,
    project_candidate: bool,
}

impl JsonWalk<'_> {
    fn enter(&mut self, segment: &str) {
        self.lengths.push(self.path.len());
        if self.lengths.len() > MAX_DEPTH {
            return;
        }
        if self.lengths.len() > 1 {
            self.path.push('.');
        }
        self.path.push_str(segment);
    }

    fn leave(&mut self) {
        if let Some(length) = self.lengths.pop() {
            self.path.truncate(length);
        }
    }
}

impl KeyVisitor for JsonWalk<'_> {
    fn string_value(&mut self, value: &str, line: u32) {
        path_literal(self.facts, value, line);
    }

    fn key(&mut self, key: &str, line: u32, kind: Kind) {
        if self.lengths.is_empty()
            && matches!(
                key,
                "compilerOptions" | "extends" | "references" | "files" | "include" | "exclude"
            )
        {
            self.project_candidate = true;
        }
        path_literal(self.facts, key, line);
        self.enter(key);
        if self.lengths.len() <= MAX_DEPTH
            && self.seen.len() < MAX_KEYS
            && self
                .seen
                .insert(xxhash_rust::xxh3::xxh3_64(self.path.as_bytes()))
        {
            self.facts.config_keys.push(ConfigKey {
                path: self.path.clone(),
                kind: json_kind(kind),
                line,
            });
        }
    }

    fn end_key(&mut self) {
        self.leave();
    }

    // Objects in arrays share one path segment `[]`.
    fn array_object(&mut self) {
        self.enter("[]");
    }

    fn end_array_object(&mut self) {
        self.leave();
    }
}

/// Kind of a scalar written after a `:` or `=`; empty means a nested block.
fn scalar_kind(value: &str) -> ValueKind {
    let value = value.trim();
    match value {
        "" => ValueKind::Object,
        "true" | "false" | "yes" | "no" | "on" | "off" | "True" | "False" => ValueKind::Bool,
        "null" | "~" | "Null" | "NULL" => ValueKind::Null,
        _ if value.starts_with('[') => ValueKind::Array,
        _ if value.starts_with('{') => ValueKind::Object,
        _ if value.parse::<f64>().is_ok() => ValueKind::Number,
        _ => ValueKind::String,
    }
}

/// YAML block mappings by indentation; `- ` items open a `[]` segment. Compose `environment`,
/// GitHub Actions `env` and Kubernetes `env` lists yield env definitions.
fn yaml(path: &str, text: &str, facts: &mut Facts) {
    let workflow = path.contains(".github/workflows/");
    let kubernetes = text.contains("apiVersion:") && text.contains("kind:");
    let name = path.rsplit('/').next().unwrap_or(path);
    let compose = name.starts_with("docker-compose") || name.starts_with("compose.");
    let (entries, coverage) = super::yaml::parse(text);
    facts.coverage = facts.coverage.max(coverage);
    for entry in entries {
        if !entry.key.is_empty()
            && let Some(kind) = entry.kind
        {
            let mut segments = entry.path.clone();
            segments.push(entry.key.clone());
            push_key(facts, &segments, kind, entry.line);
        }
        let sequence = entry.path.last().is_some_and(|key| key == "[]");
        let parent = if sequence {
            entry.path.iter().rev().nth(1)
        } else {
            entry.path.last()
        };
        let in_env = parent.is_some_and(|key| matches!(key.as_str(), "environment" | "env"));
        let execution = (workflow && entry.key == "run")
            || (compose
                && (matches!(entry.key.as_str(), "command" | "entrypoint")
                    || parent.is_some_and(|key| matches!(key.as_str(), "command" | "entrypoint"))));
        if entry.kind == Some(ValueKind::String) && !execution {
            path_literal(facts, &entry.value, entry.line);
        }
        if in_env && (compose || workflow) {
            let variable = if sequence && entry.key.is_empty() {
                entry.value.split('=').next().unwrap_or_default()
            } else {
                entry.key.as_str()
            };
            push_env(
                facts,
                variable,
                if compose {
                    EnvSource::Compose
                } else {
                    EnvSource::Ci
                },
                entry.line,
            );
        }
        if kubernetes && sequence && in_env && entry.key == "name" {
            push_env(facts, &entry.value, EnvSource::Kubernetes, entry.line);
        }
    }
}

/// `[table]`, `[[array.of.tables]]` and `key = value` (dotted keys allowed).
fn toml_keys(text: &str, facts: &mut Facts) {
    let mut prefix: Vec<String> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index as u32 + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(header) = trimmed
            .strip_prefix("[[")
            .and_then(|rest| rest.split("]]").next())
        {
            prefix = split_dotted(header);
            push_key(facts, &prefix, ValueKind::Array, line);
            continue;
        }
        if let Some(header) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.split(']').next())
        {
            prefix = split_dotted(header);
            push_key(facts, &prefix, ValueKind::Object, line);
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let mut path = prefix.clone();
        path.extend(split_dotted(key));
        push_key(
            facts,
            &path,
            scalar_kind(value.split(" #").next().unwrap_or_default()),
            line,
        );
        path_value(facts, value.split(" #").next().unwrap_or_default(), line);
    }
}

fn split_dotted(text: &str) -> Vec<String> {
    text.split('.')
        .map(|part| {
            part.trim()
                .trim_matches(|ch| ch == '"' || ch == '\'')
                .to_owned()
        })
        .filter(|part| !part.is_empty())
        .collect()
}

/// `[section]` with `key = value` or `key: value`; properties without sections.
fn ini_keys(text: &str, facts: &mut Facts) {
    let mut section: Option<String> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index as u32 + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with(['#', ';', '!']) {
            continue;
        }
        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = Some(name.trim().to_owned());
            push_key(facts, &[name.trim().to_owned()], ValueKind::Object, line);
            continue;
        }
        let Some(split) = trimmed.find(['=', ':']) else {
            continue;
        };
        let key = trimmed[..split].trim();
        if key.is_empty() || key.contains(' ') {
            continue;
        }
        let mut path: Vec<String> = section.iter().cloned().collect();
        path.extend(split_dotted(key));
        push_key(facts, &path, scalar_kind(&trimmed[split + 1..]), line);
        path_value(facts, &trimmed[split + 1..], line);
    }
}

/// Element paths (`project.dependencies.dependency`), first occurrence each.
fn xml_keys(text: &str, facts: &mut Facts) {
    let mut stack: Vec<String> = Vec::new();
    let mut line = 1u32;
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        line += rest[..open].matches('\n').count() as u32;
        rest = &rest[open + 1..];
        if rest.starts_with("!--") {
            let end = rest.find("-->").map_or(rest.len(), |end| end + 3);
            line += rest[..end].matches('\n').count() as u32;
            rest = &rest[end..];
            continue;
        }
        if rest.starts_with('?') || rest.starts_with('!') {
            continue;
        }
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        if let Some(closing) = tag.strip_prefix('/') {
            let name = closing.trim();
            if let Some(position) = stack.iter().rposition(|open| open == name) {
                stack.truncate(position);
            }
        } else {
            let name: String = tag
                .chars()
                .take_while(|ch| !ch.is_whitespace() && *ch != '/')
                .collect();
            if !name.is_empty() {
                stack.push(name);
                push_key(facts, &stack, ValueKind::Object, line);
                if tag.ends_with('/') {
                    stack.pop();
                }
            }
        }
        line += tag.matches('\n').count() as u32;
        rest = &rest[end.min(rest.len())..];
    }
}

/// `NAME=value` and `export NAME=value` lines; the value is dropped.
fn env_example(text: &str, facts: &mut Facts) {
    for (index, raw) in text.lines().enumerate() {
        let trimmed = raw.trim().trim_start_matches("export ").trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = trimmed.split_once('=') {
            push_env(facts, name, EnvSource::EnvExample, index as u32 + 1);
        }
    }
}

/// `ENV NAME=value ...` and `ENV NAME value` define; `${NAME}` and `$NAME` in `ENTRYPOINT`,
/// `CMD` and `RUN` read. `ARG` is a build parameter, no environment.
fn dockerfile(text: &str, facts: &mut Facts) {
    for (index, raw) in text.lines().enumerate() {
        let line = index as u32 + 1;
        let trimmed = raw.trim();
        let mut words = trimmed.split_whitespace();
        let instruction = words.next().unwrap_or_default().to_ascii_uppercase();
        match instruction.as_str() {
            "ENV" => {
                let rest: Vec<&str> = words.collect();
                if rest.iter().any(|word| word.contains('=')) {
                    for word in rest.iter().filter(|word| word.contains('=')) {
                        push_env(
                            facts,
                            word.split('=').next().unwrap_or_default(),
                            EnvSource::Dockerfile,
                            line,
                        );
                    }
                } else if let Some(name) = rest.first() {
                    push_env(facts, name, EnvSource::Dockerfile, line);
                }
            }
            "ENTRYPOINT" | "CMD" | "RUN" => {
                let rest = trimmed[instruction.len()..].to_owned();
                for name in shell_variables(&rest) {
                    if !facts.envs.iter().any(|env| env.name == name) {
                        facts.envs.push(EnvRead {
                            name,
                            scope: None,
                            line,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Upper-case `$NAME` and `${NAME...}` references in a shell command.
fn shell_variables(command: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = command;
    while let Some(dollar) = rest.find('$') {
        rest = &rest[dollar + 1..];
        let braced = rest.starts_with('{');
        let start = usize::from(braced);
        let name: String = rest[start..]
            .chars()
            .take_while(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || *ch == '_')
            .collect();
        if name.len() > 1
            && name.starts_with(|ch: char| ch.is_ascii_uppercase())
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    #[test]
    fn yaml_keys_preserve_valid_forms_and_report_invalid_syntax() {
        for (text, expected) in [
            ("\"name\": 1", "1:name:number"),
            ("'name': x", "1:name:string"),
            ("plain: y", "1:plain:string"),
            ("plain:", "1:plain:null"),
            ("two words: v", "1:two words:string"),
            ("{flow: map}", "1:flow:string"),
        ] {
            let facts = run("app.yml", FileType::Yaml, text);
            assert_eq!(facts.coverage, Coverage::Complete, "{text}");
            assert_eq!(keys(&facts), [expected], "{text}");
        }
        for text in ["\"open", "'open", "\"name\" no colon"] {
            assert_eq!(
                run("app.yml", FileType::Yaml, text).coverage,
                Coverage::ParseErrors,
                "{text}"
            );
        }
        for text in [
            "!custom key: value",
            "!custom {key: value}",
            "!custom [one, two]",
        ] {
            let facts = run("app.yml", FileType::Yaml, text);
            assert_eq!(facts.coverage, Coverage::NoGrammar, "{text}");
            assert!(facts.config_keys.is_empty(), "{text}");
        }
    }

    use super::*;

    fn run(path: &str, file_type: FileType, text: &str) -> Facts {
        let mut facts = Facts::default();
        extract(path, file_type, text, &mut facts);
        facts
    }

    fn keys(facts: &Facts) -> Vec<String> {
        facts
            .config_keys
            .iter()
            .map(|key| format!("{}:{}:{}", key.line, key.path, key.kind.id()))
            .collect()
    }

    fn envs(facts: &Facts) -> Vec<String> {
        facts
            .env_defs
            .iter()
            .map(|def| format!("{}:{}:{}", def.line, def.name, def.source.id()))
            .collect()
    }

    #[test]
    fn json_toml_ini_xml_key_paths_without_values() {
        let json = run(
            "a.json",
            FileType::Json,
            "{\n  \"db\": {\"url\": \"postgres://secret\", \"pool\": 5},\n  \"tags\": [{\"name\": \"x\"}]\n}",
        );
        assert_eq!(
            keys(&json),
            [
                "2:db:object",
                "2:db.url:string",
                "2:db.pool:number",
                "3:tags:array",
                "3:tags.[].name:string"
            ]
        );
        assert!(!format!("{json:?}").contains("secret"));
        let toml = run(
            "a.toml",
            FileType::Toml,
            "title = \"x\"\n[server]\nport = 8080\n[[workers]]\nname = \"a\"\n",
        );
        assert_eq!(
            keys(&toml),
            [
                "1:title:string",
                "2:server:object",
                "3:server.port:number",
                "4:workers:array",
                "5:workers.name:string"
            ]
        );
        let ini = run(
            "a.properties",
            FileType::Properties,
            "# c\napp.name=demo\napp.debug: true\n",
        );
        assert_eq!(keys(&ini), ["2:app.name:string", "3:app.debug:bool"]);
        let xml = run(
            "a.xml",
            FileType::Xml,
            "<?xml?>\n<project>\n  <!-- c -->\n  <name>x</name>\n  <deps><dep/></deps>\n</project>\n",
        );
        assert_eq!(
            keys(&xml),
            [
                "2:project:object",
                "4:project.name:object",
                "5:project.deps:object",
                "5:project.deps.dep:object"
            ]
        );
    }

    #[test]
    fn yaml_paths_and_env_definitions_from_compose_workflows_and_kubernetes() {
        let compose = run(
            "docker-compose.yml",
            FileType::Yaml,
            "services:\n  api:\n    image: app\n    environment:\n      - DATABASE_URL=postgres://secret\n    command: |\n      run --flag: x\n  db:\n    image: pg\n    environment:\n      DEBUG: \"1\"\n",
        );
        assert_eq!(
            keys(&compose),
            [
                "1:services:object",
                "2:services.api:object",
                "3:services.api.image:string",
                "4:services.api.environment:array",
                "6:services.api.command:string",
                "8:services.db:object",
                "9:services.db.image:string",
                "10:services.db.environment:object",
                "11:services.db.environment.DEBUG:string",
            ]
        );
        assert_eq!(
            envs(&compose),
            ["5:DATABASE_URL:compose", "11:DEBUG:compose"]
        );
        assert!(!format!("{compose:?}").contains("secret"));
        let workflow = run(
            ".github/workflows/ci.yml",
            FileType::Yaml,
            "env:\n  RUST_LOG: debug\njobs:\n  test:\n    env:\n      TOKEN: ${{ secrets.T }}\n",
        );
        assert_eq!(envs(&workflow), ["2:RUST_LOG:ci", "6:TOKEN:ci"]);
        let deployment = run(
            "k8s/app.yaml",
            FileType::Yaml,
            "apiVersion: apps/v1\nkind: Deployment\nspec:\n  containers:\n    - name: app\n      env:\n        - name: PORT\n          value: \"80\"\n---\nkind: Pod\nspec:\n  containers:\n  - name: web\n    env:\n    - name: MODE\n      value: x\n    - name: LEVEL\n",
        );
        assert_eq!(
            envs(&deployment),
            ["7:PORT:k8s", "15:MODE:k8s", "17:LEVEL:k8s"]
        );
    }

    #[test]
    fn env_example_and_dockerfile_definitions() {
        let example = run(
            ".env.example",
            FileType::EnvExample,
            "# comment\nAPI_KEY=changeme\nexport PORT=80\n",
        );
        assert_eq!(
            envs(&example),
            ["2:API_KEY:.env.example", "3:PORT:.env.example"]
        );
        let docker = run(
            "Dockerfile",
            FileType::Dockerfile,
            "FROM rust\nARG VERSION=1\nENV APP_HOME=/app MODE=prod\nENV LEGACY value\nENTRYPOINT run --host=${HOST} -u \"${USERS:-10}\" $mode\n",
        );
        assert_eq!(
            envs(&docker),
            [
                "3:APP_HOME:dockerfile",
                "3:MODE:dockerfile",
                "4:LEGACY:dockerfile"
            ]
        );
        let reads: Vec<String> = docker
            .envs
            .iter()
            .map(|env| format!("{}:{}", env.line, env.name))
            .collect();
        assert_eq!(reads, ["5:HOST", "5:USERS"]);
    }

    #[test]
    fn server_config_keys_and_paths_follow_statements_and_skip_comments() {
        let nginx = run(
            "nginx.conf",
            FileType::Text,
            "http {\n  server { include \"child config.conf\";\n    listen\n      8080; # include ignored.conf;\n  }\n}\n",
        );
        assert_eq!(
            keys(&nginx),
            [
                "1:http:object",
                "2:http.server:object",
                "2:http.server.include:string",
                "3:http.server.listen:number"
            ]
        );
        assert_eq!(nginx.coverage, Coverage::Complete);
        assert_eq!(nginx.refs.len(), 1);
        assert_eq!(
            (nginx.refs[0].name.as_str(), nginx.refs[0].line),
            ("child config.conf", 2)
        );

        let ssh = run(
            "sshd_config",
            FileType::Text,
            "PermitRootLogin no\nInclude child.conf\n# Include ignored.conf\n",
        );
        assert_eq!(keys(&ssh), ["1:PermitRootLogin:bool", "2:Include:string"]);
        assert_eq!(ssh.refs.len(), 1);
        assert_eq!(ssh.refs[0].name, "child.conf");

        let unit = run(
            "app.service",
            FileType::Text,
            "[Service]\nEnvironmentFile=child.conf\nExecStart=/bin/server \\\n# comment\n  --config=ignored.conf\n",
        );
        assert_eq!(
            keys(&unit),
            [
                "1:Service:object",
                "2:Service.EnvironmentFile:string",
                "3:Service.ExecStart:string"
            ]
        );
        assert_eq!(unit.refs.len(), 1);
        assert_eq!(unit.refs[0].name, "child.conf");
        let malformed = run("nginx.conf", FileType::Text, "http { include child.conf;\n");
        assert_eq!(malformed.coverage, Coverage::ParseErrors);

        let json = run(
            "app.json",
            FileType::Json,
            "{\n// \"ignored.conf\"\n\"include\": \"child config.conf\"\n}",
        );
        assert_eq!(json.refs.len(), 1);
        assert_eq!(
            (json.refs[0].name.as_str(), json.refs[0].line),
            ("child config.conf", 3)
        );
    }
}
