//! Config facts (`docs/spec.md` section 5): dotted key paths (max depth 6) with value kind and
//! line for JSON, YAML, TOML, XML, INI and properties files; env definitions (names only) from
//! `.env.example`, Dockerfiles, compose files, GitHub Actions workflows and Kubernetes manifests.
//! Values are never stored.

use super::json::{self, Value};
use crate::domain::facts::{ConfigKey, EnvDef, EnvRead, EnvSource, Facts, ValueKind};
use crate::domain::filetype::FileType;

const MAX_DEPTH: usize = 6;
/// Key paths kept per file: large data files would flood the search index.
const MAX_KEYS: usize = 400;

/// Fills `config_keys` and `env_defs` of a config file.
pub fn extract(path: &str, file_type: FileType, text: &str, facts: &mut Facts) {
    match file_type {
        FileType::Json => json_keys(text, facts),
        FileType::Yaml => yaml(path, text, facts),
        FileType::Toml => toml_keys(text, facts),
        FileType::Ini | FileType::Properties => ini_keys(text, facts),
        FileType::Xml => xml_keys(text, facts),
        FileType::EnvExample => env_example(text, facts),
        FileType::Dockerfile => dockerfile(text, facts),
        _ => {}
    }
    let mut seen = std::collections::HashSet::new();
    facts
        .config_keys
        .retain(|key| seen.insert(key.path.clone()));
    facts.config_keys.truncate(MAX_KEYS);
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

fn json_keys(text: &str, facts: &mut Facts) {
    let Some(value) = json::parse(text) else {
        return;
    };
    let mut path = Vec::new();
    walk_json(&value, &mut path, facts);
}

fn json_kind(value: &Value) -> ValueKind {
    match value {
        Value::Null => ValueKind::Null,
        Value::Bool(_) => ValueKind::Bool,
        Value::Number(_) => ValueKind::Number,
        Value::Str(_) => ValueKind::String,
        Value::Array(_) => ValueKind::Array,
        Value::Object(_) => ValueKind::Object,
    }
}

fn walk_json(value: &Value, path: &mut Vec<String>, facts: &mut Facts) {
    if path.len() >= MAX_DEPTH {
        return;
    }
    match value {
        Value::Object(entries) => {
            for (key, line, child) in entries {
                path.push(key.clone());
                push_key(facts, path, json_kind(child), *line);
                walk_json(child, path, facts);
                path.pop();
            }
        }
        Value::Array(items) => {
            // Objects in arrays share one path segment `[]`.
            for item in items.iter().filter(|item| matches!(item, Value::Object(_))) {
                path.push("[]".to_owned());
                walk_json(item, path, facts);
                path.pop();
            }
        }
        _ => {}
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
    let compose = {
        let name = path.rsplit('/').next().unwrap_or(path);
        name.starts_with("docker-compose") || name.starts_with("compose.")
    };
    // Stack of (indent, key segment).
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut block_scalar: Option<usize> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index as u32 + 1;
        let content = raw.split(" #").next().unwrap_or_default().trim_end();
        let trimmed = content.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" || trimmed == "..." {
            continue;
        }
        let mut indent = content.len() - trimmed.len();
        if let Some(scalar_indent) = block_scalar {
            if indent > scalar_indent {
                continue;
            }
            block_scalar = None;
        }
        let mut body = trimmed;
        let mut item = false;
        if let Some(rest) = body
            .strip_prefix("- ")
            .or_else(|| (body == "-").then_some(""))
        {
            item = true;
            // Sequences may sit at their key's indentation (`env:` then `- name: X`): a key at the
            // same level stays open, a previous item at that level closes.
            while stack
                .last()
                .is_some_and(|(level, key)| *level > indent || (*level == indent && key == "[]"))
            {
                stack.pop();
            }
            stack.push((indent, "[]".to_owned()));
            indent += 2;
            body = rest.trim_start();
            // `- NAME=value` under compose `environment`.
            if parent_is_env(&stack[..stack.len() - 1])
                && (compose || workflow)
                && !body.contains(": ")
            {
                let name = body.split('=').next().unwrap_or_default();
                push_env(
                    facts,
                    name,
                    if compose {
                        EnvSource::Compose
                    } else {
                        EnvSource::Ci
                    },
                    line,
                );
            }
            if body.is_empty() {
                continue;
            }
        }
        let Some((key, value)) = split_yaml_key(body) else {
            continue;
        };
        while stack.last().is_some_and(|(level, _)| *level >= indent) {
            stack.pop();
        }
        let in_env = parent_is_env(&stack);
        stack.push((indent, key.to_owned()));
        let segments: Vec<String> = stack.iter().map(|(_, segment)| segment.clone()).collect();
        push_key(facts, &segments, scalar_kind(value), line);
        if matches!(value.trim(), "|" | ">" | "|-" | ">-" | "|+" | ">+") {
            block_scalar = Some(indent);
        }
        // Map form `environment: {NAME: value}` (compose, workflow) and Kubernetes
        // `env: [{name: NAME}]`.
        if in_env && !item && (compose || workflow) {
            push_env(
                facts,
                key,
                if compose {
                    EnvSource::Compose
                } else {
                    EnvSource::Ci
                },
                line,
            );
        }
        if kubernetes
            && key == "name"
            && item
            && stack.len() >= 3
            && stack[stack.len() - 3].1 == "env"
        {
            push_env(facts, value, EnvSource::Kubernetes, line);
        }
    }
}

fn parent_is_env(stack: &[(usize, String)]) -> bool {
    stack
        .last()
        .is_some_and(|(_, key)| matches!(key.as_str(), "environment" | "env"))
}

/// `key: value` with a plain or quoted key.
pub(super) fn split_yaml_key(body: &str) -> Option<(&str, &str)> {
    let (key, value) = if let Some(rest) = body.strip_prefix('"') {
        let end = rest.find('"')?;
        (&rest[..end], rest[end + 1..].strip_prefix(':')?)
    } else if let Some(rest) = body.strip_prefix('\'') {
        let end = rest.find('\'')?;
        (&rest[..end], rest[end + 1..].strip_prefix(':')?)
    } else {
        let colon = body
            .find(": ")
            .or_else(|| body.ends_with(':').then(|| body.len() - 1))?;
        (&body[..colon], &body[colon + 1..])
    };
    let key = key.trim();
    (!key.is_empty() && !key.contains(' ') && !key.starts_with(['{', '[', '&', '*', '!']))
        .then_some((key, value))
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
    fn yaml_keys_may_be_quoted_and_unsafe_forms_are_rejected() {
        use super::split_yaml_key;
        assert_eq!(split_yaml_key("\"name\": 1"), Some(("name", " 1")));
        assert_eq!(split_yaml_key("'name': x"), Some(("name", " x")));
        assert_eq!(split_yaml_key("plain: y"), Some(("plain", " y")));
        assert_eq!(split_yaml_key("plain:"), Some(("plain", "")));
        assert_eq!(split_yaml_key("\"open"), None);
        assert_eq!(split_yaml_key("'open"), None);
        assert_eq!(split_yaml_key("\"name\" no colon"), None);
        assert_eq!(split_yaml_key("two words: v"), None);
        assert_eq!(split_yaml_key("{flow: map}"), None);
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
            "services:\n  api:\n    image: app\n    environment:\n      - DATABASE_URL=postgres://secret\n      DEBUG: \"1\"\n    command: |\n      run --flag: x\n  db:\n    image: pg\n",
        );
        assert_eq!(
            keys(&compose),
            [
                "1:services:object",
                "2:services.api:object",
                "3:services.api.image:string",
                "4:services.api.environment:object",
                "6:services.api.environment.DEBUG:string",
                "7:services.api.command:string",
                "9:services.db:object",
                "10:services.db.image:string",
            ]
        );
        assert_eq!(
            envs(&compose),
            ["5:DATABASE_URL:compose", "6:DEBUG:compose"]
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
}
