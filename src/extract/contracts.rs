//! API contracts (`docs/spec.md` sections 5 and 6): OpenAPI operations as `op` symbols (name =
//! `operationId`, else `METHOD /path`; signature `METHOD /path`), Protobuf messages, enums,
//! services and rpcs, GraphQL types, interfaces, inputs, enums and fields. Handlers are matched
//! in the resolver. Scanners instead of grammars: the needed structure is shallow.

use super::json::{self, Value};
use crate::domain::facts::{self, Facts, NO_ARITY, Ref, RefKind, SymKind, Symbol};

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

fn push_symbol(
    facts: &mut Facts,
    kind: SymKind,
    name: &str,
    signature: &str,
    line: u32,
    end_line: u32,
    parent: Option<u32>,
) -> u32 {
    facts.symbols.push(Symbol {
        kind: Some(kind),
        name: name.to_owned(),
        parent,
        line,
        end_line: end_line.max(line),
        signature: facts::clip_space(signature, 160),
        exported: true,
        ..Symbol::default()
    });
    (facts.symbols.len() - 1) as u32
}

fn push_type_ref(facts: &mut Facts, scope: u32, name: &str, line: u32) {
    let name = name.trim().trim_start_matches('.');
    let name = name.rsplit('.').next().unwrap_or(name);
    if name.is_empty() || !name.starts_with(|ch: char| ch.is_ascii_uppercase()) {
        return;
    }
    facts.refs.push(Ref {
        kind: RefKind::Type,
        name: name.to_owned(),
        qualifier: None,
        scope: Some(scope),
        line,
        arity: NO_ARITY,
    });
}

/// Whether a YAML or JSON config file is an OpenAPI or Swagger document.
pub fn is_openapi(text: &str) -> bool {
    let head: String = text.chars().take(400).collect();
    head.contains("openapi") || head.contains("swagger")
}

/// OpenAPI operations of a YAML document (`paths: /x: get: operationId: y`).
pub fn openapi_yaml(text: &str, facts: &mut Facts) {
    let lines: Vec<&str> = text.lines().collect();
    let mut in_paths = false;
    let mut path: Option<(String, usize)> = None;
    let mut index = 0;
    while index < lines.len() {
        let raw = lines[index];
        let line = index as u32 + 1;
        index += 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        if indent == 0 {
            in_paths = trimmed == "paths:";
            path = None;
            continue;
        }
        if !in_paths {
            continue;
        }
        let key = trimmed
            .trim_end_matches(':')
            .trim_matches(|ch| ch == '"' || ch == '\'');
        if key.starts_with('/') && trimmed.ends_with(':') {
            path = Some((key.to_owned(), indent));
            continue;
        }
        let Some((route, route_indent)) = &path else {
            continue;
        };
        if indent <= *route_indent || !METHODS.contains(&key) || !trimmed.ends_with(':') {
            continue;
        }
        let method = key.to_ascii_uppercase();
        let start = index;
        let mut end = index;
        let mut operation_id = None;
        while end < lines.len() {
            let next = lines[end];
            let next_indent = next.len() - next.trim_start().len();
            if !next.trim().is_empty() && next_indent <= indent {
                break;
            }
            if let Some(value) = next.trim().strip_prefix("operationId:") {
                operation_id = Some(
                    value
                        .trim()
                        .trim_matches(|ch| ch == '"' || ch == '\'')
                        .to_owned(),
                );
            }
            end += 1;
        }
        let signature = format!("{method} {route}");
        let name = operation_id.unwrap_or_else(|| signature.clone());
        push_symbol(
            facts,
            SymKind::Op,
            &name,
            &signature,
            line,
            end as u32,
            None,
        );
        index = start;
    }
}

/// OpenAPI operations of a JSON document.
pub fn openapi_json(text: &str, facts: &mut Facts) {
    let Some(value) = json::parse(text) else {
        return;
    };
    for (route, line, item) in value.get("paths").map(Value::entries).unwrap_or_default() {
        for (method, method_line, operation) in item.entries() {
            if !METHODS.contains(&method.as_str()) {
                continue;
            }
            let signature = format!("{} {route}", method.to_ascii_uppercase());
            let name = operation
                .get("operationId")
                .and_then(Value::as_str)
                .map_or_else(|| signature.clone(), str::to_owned);
            push_symbol(
                facts,
                SymKind::Op,
                &name,
                &signature,
                (*method_line).max(*line),
                *method_line,
                None,
            );
        }
    }
}

/// Protobuf `message`, `enum`, `service` and `rpc` with message field and rpc types as uses.
pub fn proto(text: &str, facts: &mut Facts) {
    let mut stack: Vec<(u32, i32)> = Vec::new();
    let mut depth = 0i32;
    for (index, raw) in text.lines().enumerate() {
        let line = index as u32 + 1;
        let code = raw.split("//").next().unwrap_or_default().trim();
        let words: Vec<&str> = code.split_whitespace().collect();
        let parent = stack.last().map(|(symbol, _)| *symbol);
        match words.as_slice() {
            ["message", name, ..] | ["enum", name, ..] | ["service", name, ..]
                if code.ends_with('{') =>
            {
                let kind = match words[0] {
                    "message" => SymKind::Struct,
                    "enum" => SymKind::Enum,
                    _ => SymKind::Iface,
                };
                let name = name.trim_end_matches('{');
                let symbol = push_symbol(
                    facts,
                    kind,
                    name,
                    code.trim_end_matches('{'),
                    line,
                    line,
                    parent,
                );
                stack.push((symbol, depth));
            }
            ["rpc", ..] => {
                let name = code[3..]
                    .trim()
                    .split(['(', ' '])
                    .next()
                    .unwrap_or_default();
                let signature = code
                    .trim_end_matches(';')
                    .trim_end_matches("{}")
                    .trim_end_matches('{')
                    .trim_end();
                let symbol =
                    push_symbol(facts, SymKind::Method, name, signature, line, line, parent);
                for part in code.split(['(', ')']).skip(1).step_by(2) {
                    push_type_ref(facts, symbol, part.trim_start_matches("stream "), line);
                }
            }
            [first, ..]
                if parent.is_some()
                    && words.len() >= 3
                    && code.contains('=')
                    && !matches!(*first, "option" | "reserved" | "oneof") =>
            {
                // Field `[repeated] Type name = 1;`.
                let kind = if matches!(*first, "repeated" | "optional" | "required") {
                    words[1]
                } else {
                    first
                };
                let kind = kind
                    .split('<')
                    .next_back()
                    .unwrap_or(kind)
                    .trim_end_matches('>');
                if let Some(scope) = parent {
                    push_type_ref(facts, scope, kind.trim_end_matches(','), line);
                }
            }
            _ => {}
        }
        depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
        while stack.last().is_some_and(|(_, open)| depth <= *open) {
            if let Some((symbol, _)) = stack.pop() {
                facts.symbols[symbol as usize].end_line = line;
            }
        }
    }
}

/// GraphQL `type`, `interface`, `input`, `enum` (and `extend type`) with fields as members and
/// field types as uses.
pub fn graphql(text: &str, facts: &mut Facts) {
    let mut current: Option<u32> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index as u32 + 1;
        let code = raw.split('#').next().unwrap_or_default().trim();
        let words: Vec<&str> = code
            .trim_start_matches("extend ")
            .split_whitespace()
            .collect();
        if let ["type" | "interface" | "input" | "enum", name, ..] = words.as_slice() {
            let kind = match words[0] {
                "interface" => SymKind::Iface,
                "enum" => SymKind::Enum,
                _ => SymKind::Struct,
            };
            let name = name.trim_end_matches('{');
            let symbol = push_symbol(
                facts,
                kind,
                name,
                code.trim_end_matches('{'),
                line,
                line,
                None,
            );
            if let Some(interfaces) = code.split(" implements ").nth(1) {
                for interface in interfaces.trim_end_matches('{').split(['&', ',']) {
                    facts.refs.push(Ref {
                        kind: RefKind::Implement,
                        name: interface.trim().to_owned(),
                        qualifier: None,
                        scope: Some(symbol),
                        line,
                        arity: NO_ARITY,
                    });
                }
            }
            current = code.ends_with('{').then_some(symbol);
            continue;
        }
        let Some(owner) = current else { continue };
        if code.starts_with('}') {
            facts.symbols[owner as usize].end_line = line;
            current = None;
            continue;
        }
        if facts.symbols[owner as usize].kind() == SymKind::Enum {
            continue;
        }
        // Field `name(args): Type!`.
        let Some(colon) = code.rfind(':') else {
            continue;
        };
        let name = code.split(['(', ':']).next().unwrap_or_default().trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            continue;
        }
        let field = push_symbol(facts, SymKind::Method, name, code, line, line, Some(owner));
        let kind = code[colon + 1..].trim().trim_matches(['[', ']', '!', ' ']);
        push_type_ref(
            facts,
            field,
            kind.split(['!', ']', ' ', '@']).next().unwrap_or(kind),
            line,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols(facts: &Facts) -> Vec<String> {
        facts
            .symbols
            .iter()
            .map(|symbol| {
                format!(
                    "{}-{} {:?} {} | {}",
                    symbol.line,
                    symbol.end_line,
                    symbol.kind(),
                    symbol.name,
                    symbol.signature
                )
            })
            .collect()
    }

    #[test]
    fn openapi_operations_from_yaml_and_json() {
        let mut facts = Facts::default();
        openapi_yaml(
            "openapi: 3.0.0\npaths:\n  /users/{id}:\n    get:\n      operationId: getUser\n      responses: {}\n    delete:\n      summary: x\ncomponents:\n  schemas: {}\n",
            &mut facts,
        );
        assert_eq!(
            symbols(&facts),
            [
                "4-6 Op getUser | GET /users/{id}",
                "7-8 Op DELETE /users/{id} | DELETE /users/{id}"
            ]
        );
        let mut json = Facts::default();
        openapi_json(
            "{\"swagger\": \"2.0\", \"paths\": {\"/pets\": {\"post\": {\"operationId\": \"addPet\"}}}}",
            &mut json,
        );
        assert_eq!(symbols(&json), ["1-1 Op addPet | POST /pets"]);
    }

    #[test]
    fn protobuf_and_graphql_contracts() {
        let mut facts = Facts::default();
        proto(
            "syntax = \"proto3\";\n\nmessage GetUserRequest {\n  string id = 1;\n}\n\nmessage User {\n  repeated Address addresses = 2;\n}\n\nservice UserService {\n  rpc GetUser(GetUserRequest) returns (User);\n  rpc Watch(stream Ping) returns (stream User) {}\n}\n",
            &mut facts,
        );
        assert_eq!(
            symbols(&facts),
            [
                "3-5 Struct GetUserRequest | message GetUserRequest",
                "7-9 Struct User | message User",
                "11-14 Iface UserService | service UserService",
                "12-12 Method GetUser | rpc GetUser(GetUserRequest) returns (User)",
                "13-13 Method Watch | rpc Watch(stream Ping) returns (stream User)",
            ]
        );
        let uses: Vec<&str> = facts
            .refs
            .iter()
            .map(|reference| reference.name.as_str())
            .collect();
        assert_eq!(uses, ["Address", "GetUserRequest", "User", "Ping", "User"]);
        let mut schema = Facts::default();
        graphql(
            "type Query {\n  user(id: ID!): User\n  users: [User!]!\n}\n\ntype User implements Node {\n  id: ID!\n}\n\nenum Role { ADMIN }\n",
            &mut schema,
        );
        assert_eq!(
            symbols(&schema),
            [
                "1-4 Struct Query | type Query",
                "2-2 Method user | user(id: ID!): User",
                "3-3 Method users | users: [User!]!",
                "6-8 Struct User | type User implements Node",
                "7-7 Method id | id: ID!",
                "10-10 Enum Role | enum Role { ADMIN }",
            ]
        );
    }
}
