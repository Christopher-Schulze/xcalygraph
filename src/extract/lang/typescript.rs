//! TypeScript and JavaScript: `export` visibility, ES module and CommonJS imports, JSX
//! component usage; `.tsx` uses the TSX grammar.

use super::Pack;
#[cfg(feature = "lang-typescript")]
use crate::domain::facts::FieldName;
use crate::domain::facts::{FLAG_DECLARATION, Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, unquote};

#[cfg(feature = "lang-typescript")]
#[path = "typescript_number.rs"]
mod number;

pub const HOOKS: Hooks = Hooks {
    exported,
    signature_prefix,
    flags,
    expand_import,
    builtin,
};

#[cfg(feature = "lang-typescript")]
pub static TYPESCRIPT: Pack = Pack {
    file_type: FileType::TypeScript,
    suffixes: &[],
    language: |_| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    query: include_str!("../queries/typescript.scm"),
    hooks: HOOKS,
};

#[cfg(feature = "lang-typescript")]
pub static TSX: Pack = Pack {
    file_type: FileType::TypeScript,
    suffixes: &[".tsx"],
    language: |_| tree_sitter_typescript::LANGUAGE_TSX.into(),
    query: concat!(
        include_str!("../queries/typescript.scm"),
        include_str!("../queries/jsx.scm")
    ),
    hooks: HOOKS,
};

#[cfg(feature = "lang-javascript")]
pub static JAVASCRIPT: Pack = Pack {
    file_type: FileType::JavaScript,
    suffixes: &[],
    language: |_| tree_sitter_javascript::LANGUAGE.into(),
    query: concat!(
        include_str!("../queries/javascript.scm"),
        include_str!("../queries/jsx.scm")
    ),
    hooks: HOOKS,
};

/// Direct literal keys representable without ambiguous field-list rendering.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn simple_property_key(
    node: tree_sitter::Node,
    source: &[u8],
) -> Option<FieldName> {
    let text = node.utf8_text(source).ok()?;
    match node.kind() {
        "string" => {
            let quote = text.chars().next()?;
            if !matches!(quote, '\'' | '"') {
                return None;
            }
            let content = text.strip_prefix(quote)?.strip_suffix(quote)?;
            if content.contains('\\') {
                let units = decode_string_content(content)?;
                Some(match String::from_utf16(&units) {
                    Ok(value) => FieldName::Text(value),
                    Err(_) => FieldName::Utf16(units),
                })
            } else {
                Some(FieldName::Text(content.to_owned()))
            }
        }
        "number" => number::property_key(text).map(FieldName::Text),
        _ => None,
    }
}

/// Complete supported member syntax; unknown or computed members are not merge evidence.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn interface_member_identity(
    interface: tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    use std::fmt::Write as _;

    if interface.has_error() || interface.is_missing() {
        return None;
    }
    let body = interface.child_by_field_name("body")?;
    let mut cursor = body.walk();
    let mut identity = String::from("interface_members:");
    for member in body.named_children(&mut cursor) {
        let tag = match member.kind() {
            "property_signature" => 'p',
            "method_signature" => {
                let mut children = member.walk();
                if member
                    .children(&mut children)
                    .any(|child| child.kind() == "?")
                {
                    'o'
                } else {
                    'm'
                }
            }
            kind if kind.contains("comment") => continue,
            _ => return None,
        };
        let name_node = member.child_by_field_name("name")?;
        let name = match name_node.kind() {
            "property_identifier" => {
                let name = name_node.utf8_text(source).ok()?;
                if name.contains('\\') {
                    return None;
                }
                FieldName::Text(name.to_owned())
            }
            "string" | "number" => simple_property_key(name_node, source)?,
            _ => return None,
        };
        let name = name.token();
        let signature = if tag == 'p' {
            crate::extract::engine::signature_identity_without_node(member, source, name_node)?
        } else {
            crate::extract::engine::signature_identity(member, source)?
        };
        let _ = write!(
            identity,
            "{tag}{}:{name}{}:{signature}",
            name.len(),
            signature.len()
        );
    }
    Some(identity)
}

/// Group only same-file declarations whose complete source facts prove compatible members.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn interface_groups(
    symbols: &[crate::domain::facts::Symbol],
    identities: &[crate::domain::facts::SourceIdentity],
    nodes: &[Option<tree_sitter::Node<'_>>],
    source: &[u8],
) -> Vec<crate::domain::facts::SourceIdentity> {
    use crate::domain::facts::{SourceIdentity, SourceIdentityKind};
    use std::collections::HashMap;

    let mut names: HashMap<&str, Vec<u32>> = HashMap::new();
    for (index, symbol) in symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.parent.is_none())
    {
        if let Ok(index) = u32::try_from(index) {
            names.entry(&symbol.name).or_default().push(index);
        }
    }
    let lookup: HashMap<_, _> = identities
        .iter()
        .map(|identity| ((identity.kind, identity.index), identity.identity.as_str()))
        .collect();
    let mut groups = Vec::new();
    for parts in names.values().filter(|parts| parts.len() > 1) {
        let first = &symbols[parts[0] as usize];
        let Some(header) = lookup.get(&(SourceIdentityKind::InterfaceHeader, parts[0])) else {
            continue;
        };
        let mut member_sets = Vec::new();
        for &part in parts {
            let symbol = &symbols[part as usize];
            if symbol.kind != Some(SymKind::Iface)
                || symbol.exported != first.exported
                || lookup.get(&(SourceIdentityKind::InterfaceHeader, part)) != Some(header)
            {
                member_sets.clear();
                break;
            }
            let Some(members) = lookup.get(&(SourceIdentityKind::InterfaceMembers, part)) else {
                member_sets.clear();
                break;
            };
            member_sets.push(*members);
        }
        if member_sets.len() != parts.len()
            || !compatible_interface_members(&member_sets)
            || !compatible_direct_base(parts[0], &member_sets, symbols, &names, nodes, source)
        {
            continue;
        }
        let canonical = parts[0].to_string();
        groups.extend(parts.iter().map(|&index| SourceIdentity {
            kind: SourceIdentityKind::InterfaceGroup,
            index,
            identity: canonical.clone(),
        }));
    }
    groups
}

/// A single local, nongeneric interface base proves compatibility only when its members do not
/// overlap the declarations being merged. Other inheritance needs stronger type evidence.
#[cfg(feature = "lang-typescript")]
fn compatible_direct_base(
    part: u32,
    members: &[&str],
    symbols: &[crate::domain::facts::Symbol],
    names: &std::collections::HashMap<&str, Vec<u32>>,
    nodes: &[Option<tree_sitter::Node<'_>>],
    source: &[u8],
) -> bool {
    let Some(interface) = nodes.get(part as usize).copied().flatten() else {
        return false;
    };
    let mut walker = interface.walk();
    let Some(clause) = interface
        .named_children(&mut walker)
        .find(|child| child.kind() == "extends_type_clause")
    else {
        return true;
    };
    if symbols[part as usize].exported {
        return false;
    }
    let mut root = interface;
    while let Some(parent) = root.parent() {
        root = parent;
    }
    if root.kind() != "program" {
        return false;
    }
    let mut walker = root.walk();
    let mut external_module = false;
    for child in root.named_children(&mut walker) {
        match child.kind() {
            "import_statement" => external_module = true,
            "export_statement" => {
                external_module = true;
                if child.child_by_field_name("declaration").is_none() {
                    return false;
                }
            }
            _ => {}
        }
    }
    if !external_module {
        return false;
    }
    if interface.child_by_field_name("type_parameters").is_some() {
        return false;
    }
    let mut walker = clause.walk();
    let mut bases = clause.named_children(&mut walker);
    let (Some(base), None) = (bases.next(), bases.next()) else {
        return false;
    };
    if base.kind() != "type_identifier" {
        return false;
    }
    let Ok(name) = base.utf8_text(source) else {
        return false;
    };
    if name.contains('\\') {
        return false;
    }
    let Some([index]) = names.get(name).map(Vec::as_slice) else {
        return false;
    };
    if *index == part
        || symbols[*index as usize].kind != Some(SymKind::Iface)
        || symbols[*index as usize].exported
    {
        return false;
    }
    let Some(base) = nodes.get(*index as usize).copied().flatten() else {
        return false;
    };
    if base.child_by_field_name("type_parameters").is_some() {
        return false;
    }
    let mut walker = base.walk();
    if base
        .named_children(&mut walker)
        .any(|child| child.kind() == "extends_type_clause")
    {
        return false;
    }
    let Some(base_members) = interface_member_identity(base, source) else {
        return false;
    };
    if !compatible_interface_members(&[&base_members]) {
        return false;
    }
    let Some(base_names) = interface_member_names(&base_members) else {
        return false;
    };
    members.iter().all(|part| {
        interface_member_names(part).is_some_and(|names| names.is_disjoint(&base_names))
    })
}

#[cfg(feature = "lang-typescript")]
fn interface_member_names(identity: &str) -> Option<std::collections::HashSet<&str>> {
    let mut rest = identity.strip_prefix("interface_members:")?;
    let mut names = std::collections::HashSet::new();
    while !rest.is_empty() {
        let (&tag, tail) = rest.as_bytes().split_first()?;
        if !matches!(tag, b'p' | b'm' | b'o') {
            return None;
        }
        let tail = std::str::from_utf8(tail).ok()?;
        let (name, tail) = take_identity_part(tail)?;
        let (_, tail) = take_identity_part(tail)?;
        names.insert(name);
        rest = tail;
    }
    Some(names)
}

#[cfg(feature = "lang-typescript")]
fn compatible_interface_members(parts: &[&str]) -> bool {
    use std::collections::{HashMap, HashSet};

    let mut seen: HashMap<&str, (u8, &str)> = HashMap::new();
    for part in parts {
        let Some(mut rest) = part.strip_prefix("interface_members:") else {
            return false;
        };
        let mut own_properties = HashSet::new();
        while !rest.is_empty() {
            let Some((&tag, tail)) = rest.as_bytes().split_first() else {
                return false;
            };
            let Some(tail) = std::str::from_utf8(tail).ok() else {
                return false;
            };
            let Some((name, tail)) = take_identity_part(tail) else {
                return false;
            };
            let Some((signature, tail)) = take_identity_part(tail) else {
                return false;
            };
            if !matches!(tag, b'p' | b'm' | b'o') || (tag == b'p' && !own_properties.insert(name)) {
                return false;
            }
            if let Some(&(previous_tag, previous_signature)) = seen.get(name) {
                if tag != previous_tag || (tag == b'p' && signature != previous_signature) {
                    return false;
                }
            } else {
                seen.insert(name, (tag, signature));
            }
            rest = tail;
        }
    }
    true
}

#[cfg(feature = "lang-typescript")]
fn take_identity_part(value: &str) -> Option<(&str, &str)> {
    let (length, value) = value.split_once(':')?;
    let length = length.parse::<usize>().ok()?;
    (length <= value.len() && value.is_char_boundary(length)).then(|| value.split_at(length))
}

/// Decode ECMAScript string content into UTF-16 without replacing unpaired surrogates.
#[cfg(feature = "lang-typescript")]
fn decode_string_content(content: &str) -> Option<Vec<u16>> {
    let mut chars = content.chars().peekable();
    let mut units = Vec::with_capacity(content.len());
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            if matches!(ch, '\n' | '\r') {
                return None;
            }
            let mut buffer = [0; 2];
            units.extend_from_slice(ch.encode_utf16(&mut buffer));
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            '\n' | '\u{2028}' | '\u{2029}' => {}
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            'b' => units.push(8),
            'f' => units.push(12),
            'n' => units.push(10),
            'r' => units.push(13),
            't' => units.push(9),
            'v' => units.push(11),
            '0' if !chars.peek().is_some_and(char::is_ascii_digit) => units.push(0),
            '0'..='9' => return None,
            'x' => units.push(read_hex(&mut chars, 2)? as u16),
            'u' => {
                let value = if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut value = 0u32;
                    let mut digits = 0;
                    while chars.peek() != Some(&'}') {
                        let digit = chars.next()?.to_digit(16)?;
                        value = value.checked_mul(16)?.checked_add(digit)?;
                        digits += 1;
                    }
                    chars.next();
                    if digits == 0 || value > 0x10ffff {
                        return None;
                    }
                    value
                } else {
                    read_hex(&mut chars, 4)?
                };
                if value <= 0xffff {
                    units.push(value as u16);
                } else {
                    let scalar = char::from_u32(value)?;
                    let mut buffer = [0; 2];
                    units.extend_from_slice(scalar.encode_utf16(&mut buffer));
                }
            }
            other => {
                let mut buffer = [0; 2];
                units.extend_from_slice(other.encode_utf16(&mut buffer));
            }
        }
    }
    Some(units)
}

#[cfg(feature = "lang-typescript")]
fn read_hex(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, count: usize) -> Option<u32> {
    let mut value = 0;
    for _ in 0..count {
        value = value * 16 + chars.next()?.to_digit(16)?;
    }
    Some(value)
}

/// `export`ed declarations, plus members of exported classes and interfaces that are not
/// `private`, `protected` or `#private`.
/// Overload signatures (`function scale(x: number): number;`) declare the implementation that
/// follows them.
fn flags(definition: &Definition, _: &FileContext) -> u8 {
    if definition.node_kind == "function_signature" {
        FLAG_DECLARATION
    } else {
        0
    }
}

fn exported(definition: &Definition) -> bool {
    // `exports.f = function`, `app.f = function`: reachable through the object.
    if definition.export_wrapper || definition.node_kind == "assignment_expression" {
        return true;
    }
    let member_of_exported = matches!(
        definition.parent,
        Some((SymKind::Class | SymKind::Iface, true))
    );
    member_of_exported
        && !definition.name.starts_with('#')
        && !definition.text.starts_with("private")
        && !definition.text.starts_with("protected")
}

fn signature_prefix(kind: SymKind, text: &str) -> &'static str {
    match kind {
        // `exports.f = function` reads as written.
        SymKind::Const | SymKind::Fn
            if !text.starts_with("function")
                && !text.starts_with("async")
                && !text
                    .split(" = ")
                    .next()
                    .is_some_and(|left| left.contains('.')) =>
        {
            "const "
        }
        _ => "",
    }
}

/// One import per bound name: `import D, {a as b} from "m"`, `import * as ns from "m"`,
/// `export {x} from "m"`, `export * from "m"`. `names` holds the imported name (`default`
/// for default imports, `*` for namespaces and star re-exports), `alias` the local name.
fn expand_import(statement: &str, line: u32, _reexport: bool) -> Vec<Import> {
    let text: String = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    let reexport = text.starts_with("export");
    let Some(spec) = text
        .rsplit(" from ")
        .next()
        .filter(|_| text.contains(" from "))
        .or_else(|| text.strip_prefix("import "))
    else {
        return Vec::new();
    };
    let spec = unquote(spec.trim_end_matches(';').trim()).to_owned();
    let import = |name: &str, alias: Option<&str>| Import {
        spec: spec.clone(),
        alias: alias.map(str::to_owned),
        names: vec![name.to_owned()],
        line,
        reexport,
    };
    let Some((head, _)) = text.split_once(" from ") else {
        // Side-effect import: `import "./polyfill"`.
        return vec![Import {
            spec,
            line,
            ..Import::default()
        }];
    };
    let clause = head
        .trim_start_matches("export")
        .trim_start()
        .trim_start_matches("import")
        .trim_start()
        .trim_start_matches("type ")
        .trim();
    let mut imports = Vec::new();
    let (outside, braces) = match (clause.find('{'), clause.rfind('}')) {
        (Some(open), Some(close)) if open < close => (
            format!("{}{}", &clause[..open], &clause[close + 1..]),
            Some(&clause[open + 1..close]),
        ),
        _ => (clause.to_owned(), None),
    };
    for part in outside
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some(alias) = part.strip_prefix("* as ") {
            imports.push(import("*", Some(alias.trim())));
        } else if part == "*" {
            imports.push(import("*", None));
        } else {
            imports.push(import("default", Some(part)));
        }
    }
    for part in braces
        .into_iter()
        .flat_map(|inner| inner.split(','))
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let part = part.trim_start_matches("type ").trim();
        match part.split_once(" as ") {
            Some((name, alias)) => imports.push(import(name.trim(), Some(alias.trim()))),
            None => imports.push(import(part, None)),
        }
    }
    imports
}

fn builtin(name: &str) -> bool {
    matches!(
        name,
        "console"
            | "require"
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "Promise"
            | "Object"
            | "Array"
            | "String"
            | "Number"
            | "Boolean"
            | "JSON"
            | "Math"
            | "Date"
            | "Error"
            | "TypeError"
            | "Map"
            | "Set"
            | "Symbol"
            | "parseInt"
            | "parseFloat"
            | "fetch"
            | "Record"
            | "Partial"
            | "Readonly"
            | "Pick"
            | "Omit"
            | "ReturnType"
            | "Awaited"
            | "string"
            | "number"
            | "boolean"
            | "any"
            | "unknown"
            | "void"
            | "never"
            | "undefined"
            | "null"
            | "describe"
            | "it"
            | "test"
            | "expect"
            | "beforeEach"
            | "afterEach"
            | "beforeAll"
            | "afterAll"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn repeated_interface_headers_retain_complete_source_identity() {
        use crate::domain::corpus::FileRecord;
        use crate::domain::role::Role;

        let prefix = (0..12)
            .map(|index| format!("key{index}abcdefghij: string;"))
            .collect::<String>();
        let source = format!(
            "export interface Config<T extends {{ {prefix} last: string }}> {{ a: T }}\n\
             export interface Config<T extends {{ {prefix} last: number }}> {{ b: T }}\n"
        );
        let record = FileRecord {
            path: "long-headers.ts".to_owned(),
            size: source.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::TypeScript),
        };
        let facts = crate::extract::extract(&record, source.as_bytes());
        assert_eq!(facts.symbols.len(), 2);
        assert_eq!(facts.symbols[0].signature, facts.symbols[1].signature);
        let identities: Vec<_> = facts
            .source_identities
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|identity| identity.identity.starts_with("interface_declaration:"))
            .map(|identity| (identity.index, identity.identity.as_str()))
            .collect();
        assert_eq!(identities.len(), 2, "{identities:?}");
        assert_eq!((identities[0].0, identities[1].0), (0, 1));
        assert_ne!(identities[0].1, identities[1].1);

        let source = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/typescript-merge-model.ts"
        ));
        let facts = crate::extract::extract(&record, source);
        let headers: Vec<_> = facts
            .source_identities
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|identity| {
                identity.kind == crate::domain::facts::SourceIdentityKind::InterfaceHeader
                    && facts.symbols[identity.index as usize].name == "Config"
            })
            .collect();
        assert_eq!(headers.len(), 2, "{headers:?}");
        assert_eq!(headers[0].identity, headers[1].identity);
        let group = crate::domain::facts::SourceIdentityKind::InterfaceGroup;
        assert_eq!(
            facts.source_identity(group, headers[0].index),
            facts.source_identity(group, headers[1].index)
        );
        assert!(facts.source_identity(group, headers[0].index).is_some());
        assert!(
            headers
                .iter()
                .all(|identity| facts.symbols[identity.index as usize].parent.is_none())
        );
        let members: Vec<_> = facts
            .source_identities
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|identity| {
                identity.kind == crate::domain::facts::SourceIdentityKind::InterfaceMembers
                    && facts.symbols[identity.index as usize].name == "Config"
            })
            .collect();
        assert_eq!(members.len(), 2, "{members:?}");
        assert!(members[0].identity.contains("p5:width"));
        assert!(members[1].identity.contains("p6:height"));
        assert!(
            members
                .iter()
                .all(|identity| identity.identity.contains("m3:run"))
        );

        let unsupported = crate::extract::extract(
            &record,
            b"interface Unknown { [x: string]: string }\ninterface Unknown { safe: string }",
        );
        assert_eq!(unsupported.symbols.len(), 2);
        let kind = crate::domain::facts::SourceIdentityKind::InterfaceMembers;
        assert_eq!(unsupported.source_identity(kind, 0), None);
        assert!(unsupported.source_identity(kind, 1).is_some());
        assert_eq!(unsupported.source_identity(group, 0), None);
        assert_eq!(unsupported.source_identity(group, 1), None);

        let invalid = crate::extract::extract(
            &record,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/typescript-merge-invalid.ts"
            )),
        );
        let header = crate::domain::facts::SourceIdentityKind::InterfaceHeader;
        assert_eq!(
            invalid.source_identity(header, 0),
            invalid.source_identity(header, 1)
        );
        assert_ne!(
            invalid.source_identity(kind, 0),
            invalid.source_identity(kind, 1)
        );
        assert_ne!(
            invalid.source_identity(header, 2),
            invalid.source_identity(header, 3)
        );
        for index in 0..4 {
            assert_eq!(invalid.source_identity(group, index), None);
        }

        let mixed = crate::extract::extract(
            &record,
            b"interface Mode { f?(): void }\ninterface Mode { f(): void }",
        );
        let owners: Vec<_> = mixed
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| symbol.kind() == SymKind::Iface && symbol.name == "Mode")
            .map(|(index, _)| index as u32)
            .collect();
        assert_eq!(owners.len(), 2);
        assert!(
            mixed
                .source_identity(kind, owners[0])
                .is_some_and(|s| s.contains("o1:f"))
        );
        assert!(
            mixed
                .source_identity(kind, owners[1])
                .is_some_and(|s| s.contains("m1:f"))
        );
        assert!(
            owners
                .iter()
                .all(|&index| mixed.source_identity(group, index).is_none())
        );

        let identical = crate::extract::extract(
            &record,
            b"interface Same { x: string }\ninterface Same { x: string }",
        );
        assert_eq!(identical.source_identity(group, 0), Some("0"));
        assert_eq!(identical.source_identity(group, 1), Some("0"));

        let duplicate = crate::extract::extract(
            &record,
            b"interface Bad { x: string; x: string }\ninterface Bad { y: number }",
        );
        assert_eq!(duplicate.source_identity(group, 0), None);
        assert_eq!(duplicate.source_identity(group, 1), None);
    }

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn numeric_property_keys_follow_ecmascript_value_and_spelling() {
        use crate::domain::corpus::FileRecord;
        use crate::domain::role::Role;

        let source = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/typescript-numeric-property-keys.ts"
        ));
        let record = FileRecord {
            path: "numeric-property-keys.ts".to_owned(),
            size: source.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::TypeScript),
        };
        let facts = crate::extract::extract(&record, source);
        let numeric = facts
            .fields
            .iter()
            .find(|fields| facts.symbols[fields.owner as usize].name == "NumericKeys");
        assert!(numeric.is_some(), "missing numeric fields: {facts:#?}");
        if let Some(numeric) = numeric {
            assert_eq!(
                numeric
                    .names
                    .iter()
                    .filter_map(FieldName::text)
                    .collect::<Vec<_>>(),
                [
                    "16",
                    "1000",
                    "10",
                    "63",
                    "1234",
                    "92862869855095860",
                    "679496604817411.2",
                    "1e-7",
                    "0.000001",
                    "1e+21",
                    "100000000000000000000",
                    "Infinity",
                    "9007199254740992"
                ]
            );
        }
        let ambiguous = facts
            .fields
            .iter()
            .find(|fields| facts.symbols[fields.owner as usize].name == "AmbiguousDecimalKey");
        assert_eq!(
            ambiguous.map(|fields| fields.names.as_slice()),
            Some([FieldName::Text("safe".to_owned())].as_slice())
        );
    }

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn string_escape_decoding_preserves_utf16_and_rejects_invalid_escapes() {
        assert_eq!(
            decode_string_content("escaped\\u0061"),
            Some("escapeda".encode_utf16().collect())
        );
        assert_eq!(decode_string_content("\\uD800"), Some(vec![0xd800]));
        assert_eq!(
            decode_string_content("\\\\uD800"),
            Some("\\uD800".encode_utf16().collect())
        );
        assert_eq!(
            decode_string_content("\\u{1F600}\\uD83D\\uDE00"),
            Some("😀😀".encode_utf16().collect())
        );
        assert_eq!(
            decode_string_content("a\\\r\nb\\x21"),
            Some("ab!".encode_utf16().collect())
        );
        for invalid in ["\\", "\\x0", "\\u12", "\\u{}", "\\u{110000}", "\\01", "\\8"] {
            assert_eq!(decode_string_content(invalid), None, "{invalid}");
        }
    }

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn string_property_keys_preserve_decoded_names_and_types() {
        use crate::domain::corpus::FileRecord;
        use crate::domain::role::Role;

        let source = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/typescript-string-property-keys.ts"
        ));
        let record = FileRecord {
            path: "string-property-keys.ts".to_owned(),
            size: source.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::TypeScript),
        };
        let facts = crate::extract::extract(&record, source);
        let special = facts
            .fields
            .iter()
            .find(|fields| facts.symbols[fields.owner as usize].name == "SpecialKeys");
        assert!(special.is_some(), "extracted facts: {facts:#?}");
        if let Some(special) = special {
            assert_eq!(
                special.names,
                [
                    FieldName::Text("a,b".to_owned()),
                    FieldName::Text(String::new()),
                    FieldName::Text("escapeda".to_owned()),
                    FieldName::Text("line\nfeed".to_owned()),
                    FieldName::Text("😀".to_owned()),
                    FieldName::Utf16(vec![0xd800]),
                    FieldName::Text("\\uD800".to_owned()),
                ]
            );
            assert_eq!(
                special.types,
                [
                    "string", "boolean", "number", "string", "boolean", "string", "number"
                ]
            );
        }
        let computed = facts
            .fields
            .iter()
            .find(|fields| facts.symbols[fields.owner as usize].name == "ComputedKey");
        assert_eq!(
            computed.map(|fields| fields.names.as_slice()),
            Some([FieldName::Text("regular".to_owned())].as_slice())
        );
    }

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn interface_properties_keep_their_owner_and_declared_types() {
        use crate::domain::corpus::FileRecord;
        use crate::domain::role::Role;

        let source = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/typescript-interface-properties.ts"
        ));
        let record = FileRecord {
            path: "interface-properties.ts".to_owned(),
            size: source.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::TypeScript),
        };
        let facts = crate::extract::extract(&record, source);
        let fields = |name: &str| {
            facts
                .fields
                .iter()
                .find(|fields| facts.symbols[fields.owner as usize].name == name)
        };
        let config = fields("Config");
        assert!(config.is_some(), "Config properties were not captured");
        if let Some(config) = config {
            assert_eq!(
                config.names.iter().map(FieldName::text).collect::<Vec<_>>(),
                [
                    Some("width"),
                    Some("height"),
                    Some("nested"),
                    Some("choice"),
                    Some("display-name"),
                    Some("quoted-key"),
                    Some("ümlaut"),
                    Some("123"),
                    Some("9007199254740991"),
                    Some("9007199254740992"),
                    Some("a,b"),
                    Some(""),
                    Some("escapeda"),
                    None,
                    Some("\\uD800"),
                    Some("16"),
                    Some("1000")
                ]
            );
            assert_eq!(config.types[0..2], ["number", "number"]);
            assert_eq!(config.types[3], "");
            assert_eq!(
                config.types[4..8],
                ["string", "string", "string", "boolean"]
            );
            assert_eq!(config.types[8], "number");
            assert_eq!(config.types[12], "string");
            assert!(
                !config
                    .names
                    .iter()
                    .filter_map(FieldName::text)
                    .any(|name| name == "inner"
                        || name == "run"
                        || name == "escaped\\u0061"
                        || name == "0x10")
            );
        }
        let other = fields("Other");
        assert!(other.is_some(), "Other properties were not captured");
        if let Some(other) = other {
            assert_eq!(other.names, [FieldName::Text("enabled".to_owned())]);
            assert_eq!(other.types, ["boolean"]);
        }
        assert!(fields("Unrelated").is_some());
        let malformed = crate::extract::extract(&record, b"interface Broken { -1: string; }");
        assert!(malformed.fields.is_empty(), "malformed key became a field");
        let malformed = crate::extract::extract(&record, b"interface Broken { -foo: string; }");
        assert!(
            malformed.fields.is_empty(),
            "malformed identifier became a field"
        );
        let mixed = crate::extract::extract(
            &record,
            b"interface Broken { -foo: string; }\ninterface Fine { value: number; 7: boolean; }",
        );
        assert_eq!(mixed.fields.len(), 1);
        assert_eq!(
            mixed.fields[0].names,
            [
                FieldName::Text("value".to_owned()),
                FieldName::Text("7".to_owned())
            ]
        );
    }

    fn rows(statement: &str) -> Vec<(String, String, Option<String>, bool)> {
        expand_import(statement, 1, false)
            .into_iter()
            .map(|import| {
                (
                    import.spec,
                    import.names.join(","),
                    import.alias,
                    import.reexport,
                )
            })
            .collect()
    }

    fn row(
        spec: &str,
        name: &str,
        alias: Option<&str>,
        reexport: bool,
    ) -> (String, String, Option<String>, bool) {
        (
            spec.to_owned(),
            name.to_owned(),
            alias.map(str::to_owned),
            reexport,
        )
    }

    #[test]
    fn import_and_export_clauses_expand_per_binding() {
        assert_eq!(
            rows("import Store, { open as openStore, type Row } from \"./store\";"),
            [
                row("./store", "default", Some("Store"), false),
                row("./store", "open", Some("openStore"), false),
                row("./store", "Row", None, false)
            ]
        );
        assert_eq!(
            rows("import * as db from '../db'"),
            [row("../db", "*", Some("db"), false)]
        );
        assert_eq!(
            rows("export { a, b as c } from './x'"),
            [
                row("./x", "a", None, true),
                row("./x", "b", Some("c"), true)
            ]
        );
        assert_eq!(rows("export * from \"./y\""), [row("./y", "*", None, true)]);
        assert_eq!(
            rows("import \"./polyfill\";"),
            [row("./polyfill", "", None, false)]
        );
    }
}
