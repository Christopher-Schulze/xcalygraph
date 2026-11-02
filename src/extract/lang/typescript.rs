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

#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn is_typescript_path(path: &str) -> bool {
    path.rsplit('.').next().is_some_and(|extension| {
        ["ts", "tsx", "mts", "cts", "ets"]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}

/// Record whether file-wide augmentation and parse completeness can be established.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn file_scope(
    root: tree_sitter::Node,
    source: &[u8],
    exceeded: bool,
) -> &'static str {
    if root.has_error() || exceeded {
        return "incomplete";
    }
    if !source
        .windows(6)
        .any(|window| window == b"module" || window == b"global")
    {
        return "clear";
    }
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.kind() == "module"
            && node
                .child_by_field_name("name")
                .is_some_and(|name| name.kind() == "string")
        {
            return "augmentation";
        }
        if node.kind() == "ambient_declaration" {
            let mut walker = node.walk();
            if node
                .children(&mut walker)
                .any(|child| child.kind() == "global")
            {
                return "augmentation";
            }
        }
        let mut walker = node.walk();
        pending.extend(node.named_children(&mut walker));
    }
    "clear"
}

/// Capture every supported relative module augmentation, or leave the file unproven.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn augmentation_evidence(
    root: tree_sitter::Node,
    source: &[u8],
    parsed: &crate::extract::engine::Parsed,
) -> Option<Vec<crate::domain::facts::SourceIdentity>> {
    use crate::domain::facts::{SourceIdentity, SourceIdentityKind};

    let mut walker = root.walk();
    if !root
        .named_children(&mut walker)
        .any(|node| matches!(node.kind(), "import_statement" | "export_statement"))
    {
        return None;
    }
    let mut pending = vec![root];
    let mut evidence = Vec::new();
    while let Some(node) = pending.pop() {
        if node.kind() == "ambient_declaration" {
            let mut cursor = node.walk();
            if node
                .children(&mut cursor)
                .any(|child| child.kind() == "global")
            {
                return None;
            }
        }
        if node.kind() == "module"
            && node
                .child_by_field_name("name")
                .is_some_and(|name| name.kind() == "string")
        {
            if node.parent().is_none_or(|parent| {
                parent.kind() != "ambient_declaration" || parent.parent() != Some(root)
            }) {
                return None;
            }
            let literal = node.child_by_field_name("name")?.utf8_text(source).ok()?;
            let quote = literal.chars().next()?;
            if !matches!(quote, '\'' | '"') {
                return None;
            }
            let spec = literal.strip_prefix(quote)?.strip_suffix(quote)?;
            if !(spec.starts_with("./") || spec.starts_with("../"))
                || spec.contains('\\')
                || spec.chars().any(char::is_control)
            {
                return None;
            }
            let body = node.child_by_field_name("body")?;
            let mut cursor = body.walk();
            for interface in body.named_children(&mut cursor) {
                if interface.kind().contains("comment") {
                    continue;
                }
                if interface.kind() != "interface_declaration" {
                    return None;
                }
                let mut matching = parsed.spans.iter().enumerate().filter(|(_, span)| {
                    span.0 == interface.start_byte() && span.1 == interface.end_byte()
                });
                let (index, _) = matching.next()?;
                if matching.next().is_some() {
                    return None;
                }
                let symbol = parsed.facts.symbols.get(index)?;
                if symbol.kind != Some(SymKind::Iface) || symbol.parent.is_some() {
                    return None;
                }
                let index = u32::try_from(index).ok()?;
                let header = crate::extract::engine::signature_identity(interface, source)?;
                let members = interface_member_identity(interface, source)?;
                for (kind, identity) in [
                    (SourceIdentityKind::InterfaceHeader, header),
                    (SourceIdentityKind::InterfaceMembers, members),
                    (SourceIdentityKind::TypeScriptAugmentation, spec.to_owned()),
                ] {
                    if let Some(existing) = parsed.facts.source_identity(kind, index) {
                        if existing != identity {
                            return None;
                        }
                    } else {
                        evidence.push(SourceIdentity {
                            kind,
                            index,
                            identity,
                        });
                    }
                }
            }
            continue;
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    (!evidence.is_empty()).then_some(evidence)
}

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
        if member.kind() == "index_signature" {
            let name = member
                .child_by_field_name("index_type")?
                .utf8_text(source)
                .ok()?;
            if !matches!(name, "string" | "number") {
                return None;
            }
            let value = member
                .child_by_field_name("type")?
                .named_child(0)?
                .utf8_text(source)
                .ok()?;
            if !matches!(value, "string" | "number" | "boolean" | "any" | "unknown") {
                return None;
            }
            let _ = write!(identity, "i{}:{name}{}:{value}", name.len(), value.len());
            continue;
        }
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
        if matches!(tag, 'p' | 'm' | 'o') {
            let value = interface_member_value_type(member, source);
            let _ = write!(identity, "v{}:{name}{}:{value}", name.len(), value.len());
        }
    }
    Some(identity)
}

#[cfg(feature = "lang-typescript")]
fn interface_member_value_type(member: tree_sitter::Node, source: &[u8]) -> String {
    if member.kind() == "method_signature" {
        return "function".to_owned();
    }
    let mut cursor = member.walk();
    let optional = member
        .children(&mut cursor)
        .any(|child| child.kind() == "?");
    let value = member
        .child_by_field_name("type")
        .and_then(|annotation| annotation.named_child(0))
        .filter(|node| node.kind() == "predefined_type")
        .and_then(|node| node.utf8_text(source).ok())
        .filter(|value| matches!(*value, "string" | "number" | "boolean"));
    match value {
        Some(value) if optional => format!("optional:{value}"),
        Some(value) => format!("required:{value}"),
        None => "unsupported".to_owned(),
    }
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
    let escaped_names: Vec<_> = names
        .keys()
        .filter(|name| name.contains('\\'))
        .map(|name| decode_string_content(name).and_then(|units| String::from_utf16(&units).ok()))
        .collect();
    let mut groups = Vec::new();
    for parts in names.values().filter(|parts| parts.len() > 1) {
        if parts.iter().any(|&part| {
            let mut current = nodes.get(part as usize).copied().flatten();
            while let Some(node) = current {
                if node.kind() == "module"
                    && node
                        .child_by_field_name("name")
                        .is_some_and(|name| name.kind() == "string")
                {
                    return true;
                }
                current = node.parent();
            }
            false
        }) {
            continue;
        }
        let first = &symbols[parts[0] as usize];
        if first.name.contains('\\') || escaped_name_may_match(&first.name, &escaped_names) {
            continue;
        }
        let Some(header) =
            nodes[parts[0] as usize].and_then(|node| interface_header_without_bases(node, source))
        else {
            continue;
        };
        let mut member_sets = Vec::new();
        for &part in parts {
            let symbol = &symbols[part as usize];
            if symbol.kind != Some(SymKind::Iface)
                || symbol.exported != first.exported
                || nodes[part as usize]
                    .and_then(|node| interface_header_without_bases(node, source))
                    .as_ref()
                    != Some(&header)
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
        if member_sets.len() != parts.len() || !compatible_interface_members(&member_sets) {
            continue;
        }
        let mut generic_fields = Vec::new();
        if !compatible_local_bases(
            parts,
            &member_sets,
            symbols,
            &names,
            &escaped_names,
            nodes,
            source,
        ) {
            let Some(fields) = compatible_generic_base(
                parts,
                &member_sets,
                symbols,
                &names,
                &escaped_names,
                nodes,
                source,
            ) else {
                continue;
            };
            generic_fields = fields;
        }
        let canonical = parts[0].to_string();
        groups.extend(parts.iter().map(|&index| SourceIdentity {
            kind: SourceIdentityKind::InterfaceGroup,
            index,
            identity: canonical.clone(),
        }));
        if !generic_fields.is_empty() {
            generic_fields.sort_unstable();
            let fields: Vec<_> = generic_fields
                .iter()
                .map(|(base, field, concrete)| (*base, field.as_str(), concrete.as_str()))
                .collect();
            groups.push(SourceIdentity {
                kind: SourceIdentityKind::InterfaceGenericFields,
                index: parts[0],
                identity: crate::domain::facts::interface_generic_fields_identity(&fields),
            });
        }
    }
    groups
}

#[cfg(feature = "lang-typescript")]
fn interface_header_without_bases(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut walker = node.walk();
    match node
        .named_children(&mut walker)
        .find(|child| child.kind() == "extends_type_clause")
    {
        Some(clause) => {
            crate::extract::engine::signature_identity_without_node(node, source, clause)
        }
        None => crate::extract::engine::signature_identity(node, source),
    }
}

#[cfg(feature = "lang-typescript")]
fn escaped_name_may_match(name: &str, escaped_names: &[Option<String>]) -> bool {
    escaped_names
        .iter()
        .any(|candidate| candidate.as_ref().is_none_or(|decoded| decoded == name))
}

#[cfg(feature = "lang-typescript")]
pub(crate) fn declaration_name_may_match(candidate: &str, name: &str) -> bool {
    candidate == name
        || (candidate.contains('\\')
            && decode_string_content(candidate)
                .and_then(|units| String::from_utf16(&units).ok())
                .is_none_or(|decoded| decoded == name))
}

/// Local, nongeneric interface bases prove compatibility only when their complete supported
/// members agree and overlapping inherited methods retain their complete signatures.
#[cfg(feature = "lang-typescript")]
fn compatible_local_bases(
    parts: &[u32],
    members: &[&str],
    symbols: &[crate::domain::facts::Symbol],
    names: &std::collections::HashMap<&str, Vec<u32>>,
    escaped_names: &[Option<String>],
    nodes: &[Option<tree_sitter::Node<'_>>],
    source: &[u8],
) -> bool {
    use std::collections::HashSet;

    let Some(interface) = nodes.get(parts[0] as usize).copied().flatten() else {
        return false;
    };
    let has_bases = parts.iter().any(|&part| {
        nodes
            .get(part as usize)
            .copied()
            .flatten()
            .is_some_and(|node| {
                let mut walker = node.walk();
                node.named_children(&mut walker)
                    .any(|child| child.kind() == "extends_type_clause")
            })
    });
    if !has_bases {
        return true;
    }
    if symbols[parts[0] as usize].exported || !private_module_interface(interface) {
        return false;
    }
    if interface.child_by_field_name("type_parameters").is_some() {
        return false;
    }
    let mut pending: Vec<_> = parts.iter().map(|&part| (part, false)).collect();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut inherited = Vec::new();
    while let Some((index, exiting)) = pending.pop() {
        if exiting {
            visiting.remove(&index);
            visited.insert(index);
            continue;
        }
        if visited.contains(&index) {
            continue;
        }
        if !visiting.insert(index) {
            return false;
        }
        let Some(base) = nodes.get(index as usize).copied().flatten() else {
            return false;
        };
        if !parts.contains(&index) {
            if symbols[index as usize].kind != Some(SymKind::Iface)
                || symbols[index as usize].exported
                || base.child_by_field_name("type_parameters").is_some()
            {
                return false;
            }
            let Some(identity) = interface_member_identity(base, source) else {
                return false;
            };
            inherited.push(identity);
        }
        pending.push((index, true));
        let mut walker = base.walk();
        if let Some(clause) = base
            .named_children(&mut walker)
            .find(|child| child.kind() == "extends_type_clause")
        {
            let mut walker = clause.walk();
            for child in clause.named_children(&mut walker) {
                if child.kind() != "type_identifier" {
                    return false;
                }
                let Ok(name) = child.utf8_text(source) else {
                    return false;
                };
                if name.contains('\\') || escaped_name_may_match(name, escaped_names) {
                    return false;
                }
                let Some([parent]) = names.get(name).map(Vec::as_slice) else {
                    return false;
                };
                pending.push((*parent, false));
            }
        }
    }
    let mut inherited_methods = std::collections::HashMap::new();
    for identity in &inherited {
        let Some(methods) = interface_method_signatures(identity) else {
            return false;
        };
        for (name, signatures) in methods {
            if let Some(previous) = inherited_methods.get(name) {
                if previous != &signatures {
                    return false;
                }
            } else {
                inherited_methods.insert(name, signatures);
            }
        }
    }
    let mut local_methods = std::collections::HashMap::new();
    for identity in members {
        let Some(methods) = interface_method_signatures(identity) else {
            return false;
        };
        for (name, signatures) in methods {
            local_methods
                .entry(name)
                .or_insert_with(HashSet::new)
                .extend(signatures);
        }
    }
    for (name, inherited) in inherited_methods {
        if local_methods
            .get(name)
            .is_some_and(|local| !inherited.is_subset(local))
        {
            return false;
        }
    }
    let all_members: Vec<_> = inherited
        .iter()
        .map(String::as_str)
        .chain(members.iter().copied())
        .collect();
    compatible_interface_members(&all_members)
}

#[cfg(feature = "lang-typescript")]
fn private_module_interface(interface: tree_sitter::Node) -> bool {
    let mut root = interface;
    while let Some(parent) = root.parent() {
        root = parent;
    }
    if root.kind() != "program" {
        return false;
    }
    let mut walker = root.walk();
    let mut external = false;
    for child in root.named_children(&mut walker) {
        match child.kind() {
            "import_statement" => external = true,
            "export_statement" => {
                external = true;
                if child.child_by_field_name("declaration").is_none() {
                    return false;
                }
            }
            _ => {}
        }
    }
    external
}

#[cfg(feature = "lang-typescript")]
type GenericFields = Vec<(u32, String, String)>;

/// Prove a direct local instantiation with unconstrained parameters and simple fields.
#[cfg(feature = "lang-typescript")]
fn compatible_generic_base(
    parts: &[u32],
    members: &[&str],
    symbols: &[crate::domain::facts::Symbol],
    names: &std::collections::HashMap<&str, Vec<u32>>,
    escaped_names: &[Option<String>],
    nodes: &[Option<tree_sitter::Node<'_>>],
    source: &[u8],
) -> Option<GenericFields> {
    let interface = nodes.get(parts[0] as usize).copied().flatten()?;
    if symbols[parts[0] as usize].exported
        || !private_module_interface(interface)
        || interface.child_by_field_name("type_parameters").is_some()
    {
        return None;
    }
    let header = crate::extract::engine::signature_identity(interface, source)?;
    for &part in &parts[1..] {
        if crate::extract::engine::signature_identity(
            nodes.get(part as usize).copied().flatten()?,
            source,
        )? != header
        {
            return None;
        }
    }
    let mut walker = interface.walk();
    let clause = interface
        .named_children(&mut walker)
        .find(|child| child.kind() == "extends_type_clause")?;
    let mut walker = clause.walk();
    let mut pending: Vec<_> = clause
        .named_children(&mut walker)
        .map(|node| (node, Vec::new(), Vec::new()))
        .collect();
    if pending.is_empty() {
        return None;
    }
    let mut inherited = Vec::new();
    let mut inherited_fields = Vec::new();
    let mut visited = std::collections::HashSet::new();
    while let Some((application, outer_bindings, mut path)) = pending.pop() {
        let base_name = match application.kind() {
            "generic_type" => application.child_by_field_name("name")?,
            "type_identifier" => application,
            _ => return None,
        };
        if base_name.kind() != "type_identifier" {
            return None;
        }
        let name = base_name.utf8_text(source).ok()?;
        if name.contains('\\') || escaped_name_may_match(name, escaped_names) {
            return None;
        }
        let [base_index] = names.get(name)?.as_slice() else {
            return None;
        };
        if path.contains(base_index) {
            return None;
        }
        path.push(*base_index);
        let base_symbol = symbols.get(*base_index as usize)?;
        if base_symbol.kind != Some(SymKind::Iface) || base_symbol.exported {
            return None;
        }
        let base = nodes.get(*base_index as usize).copied().flatten()?;
        let parameters: Vec<_> = match base.child_by_field_name("type_parameters") {
            Some(parameters) => {
                let mut cursor = parameters.walk();
                parameters.named_children(&mut cursor).collect()
            }
            None => Vec::new(),
        };
        let arguments: Vec<_> = match application.child_by_field_name("type_arguments") {
            Some(arguments) => {
                let mut cursor = arguments.walk();
                arguments.named_children(&mut cursor).collect()
            }
            None => Vec::new(),
        };
        if arguments.len() > parameters.len() {
            return None;
        }
        let parameter_names: Vec<_> = parameters
            .iter()
            .map(|parameter| {
                parameter
                    .child_by_field_name("name")?
                    .utf8_text(source)
                    .ok()
            })
            .collect::<Option<_>>()?;
        let mut bindings = Vec::new();
        let mut saw_default = false;
        for (index, parameter) in parameters.into_iter().enumerate() {
            if parameter.kind() != "type_parameter" {
                return None;
            }
            let param = parameter_names[index];
            if param.contains('\\') || bindings.iter().any(|(name, _)| *name == param) {
                return None;
            }
            let resolve = |node: tree_sitter::Node, default: bool| -> Option<&str> {
                if !matches!(node.kind(), "type_identifier" | "predefined_type") {
                    return None;
                }
                let text = node.utf8_text(source).ok()?;
                if text.contains('\\') || escaped_name_may_match(text, escaped_names) {
                    return None;
                }
                if default && parameter_names.contains(&text) {
                    return bindings
                        .iter()
                        .find(|(name, _)| *name == text)
                        .map(|(_, value)| *value);
                }
                if !default
                    && let Some((_, value)) = outer_bindings.iter().find(|(name, _)| *name == text)
                {
                    return Some(*value);
                }
                if node.kind() == "type_identifier" {
                    let [target] = names.get(text)?.as_slice() else {
                        return None;
                    };
                    if !symbols
                        .get(*target as usize)?
                        .kind
                        .is_some_and(SymKind::is_type)
                    {
                        return None;
                    }
                }
                Some(text)
            };
            let default = parameter.child_by_field_name("value");
            if saw_default && default.is_none() {
                return None;
            }
            saw_default |= default.is_some();
            let default = match default {
                Some(default) => Some(resolve(default.named_child(0)?, true)?),
                None => None,
            };
            let actual = match arguments.get(index) {
                Some(argument) => resolve(*argument, false)?,
                None => default?,
            };
            if let Some(constraint) = parameter.child_by_field_name("constraint") {
                if arguments.iter().any(|argument| {
                    argument
                        .utf8_text(source)
                        .ok()
                        .is_some_and(|text| outer_bindings.iter().any(|(name, _)| *name == text))
                }) {
                    return None;
                }
                let bound_node = constraint.named_child(0)?;
                let bound = resolve(bound_node, true)?;
                if actual != bound || default.is_some_and(|value| value != bound) {
                    return None;
                }
                if let Some(default_node) = parameter.child_by_field_name("value") {
                    let default_text = default_node.named_child(0)?.utf8_text(source).ok()?;
                    let bound_text = bound_node.utf8_text(source).ok()?;
                    if (parameter_names.contains(&default_text)
                        || parameter_names.contains(&bound_text))
                        && default_text != bound_text
                    {
                        return None;
                    }
                }
            }
            bindings.push((param, actual));
        }
        if !visited.insert((*base_index, bindings.clone())) {
            continue;
        }
        let (identity, fields) =
            instantiated_generic_properties(base, *base_index, source, &bindings)?;
        inherited.push(identity);
        inherited_fields.extend(fields);
        let mut walker = base.walk();
        if let Some(clause) = base
            .named_children(&mut walker)
            .find(|child| child.kind() == "extends_type_clause")
        {
            let mut walker = clause.walk();
            pending.extend(
                clause
                    .named_children(&mut walker)
                    .map(|node| (node, bindings.clone(), path.clone())),
            );
        }
    }
    inherited_fields.sort_unstable();
    if inherited_fields
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1 && pair[0].2 != pair[1].2)
    {
        return None;
    }
    inherited_fields.dedup();
    for (index, identity) in inherited.iter().enumerate() {
        let others: Vec<_> = inherited[index + 1..].iter().map(String::as_str).collect();
        if !imported_base_members_are_disjoint(identity, &others) {
            return None;
        }
    }
    let all: Vec<_> = inherited
        .iter()
        .map(String::as_str)
        .chain(members.iter().copied())
        .collect();
    compatible_interface_members(&all).then_some(inherited_fields)
}

#[cfg(feature = "lang-typescript")]
fn instantiated_generic_properties(
    base: tree_sitter::Node,
    base_index: u32,
    source: &[u8],
    bindings: &[(&str, &str)],
) -> Option<(String, GenericFields)> {
    use std::fmt::Write as _;
    let body = base.child_by_field_name("body")?;
    let mut cursor = body.walk();
    let mut replacements = std::collections::HashMap::new();
    let mut fields = Vec::new();
    for member in body.named_children(&mut cursor) {
        if member.kind().contains("comment") {
            continue;
        }
        if member.kind() != "property_signature" {
            return None;
        }
        let type_node = member
            .child_by_field_name("type")
            .and_then(|annotation| annotation.named_child(0));
        let mut element = type_node;
        while element.is_some_and(|node| node.kind() == "array_type") {
            element = element.and_then(|node| node.named_child(0));
        }
        let substitution = element
            .filter(|node| node.kind() == "type_identifier")
            .and_then(|node| node.utf8_text(source).ok())
            .and_then(|name| {
                bindings
                    .iter()
                    .find(|(parameter, _)| *parameter == name)
                    .copied()
            });
        if let Some(node) = type_node {
            let mut pending = vec![node];
            while let Some(node) = pending.pop() {
                if matches!(
                    node.kind(),
                    "infer_type" | "type_parameters" | "mapped_type_clause"
                ) {
                    return None;
                }
                if matches!(
                    node.kind(),
                    "type_identifier"
                        | "identifier"
                        | "property_identifier"
                        | "shorthand_property_identifier"
                ) && bindings
                    .iter()
                    .any(|(parameter, _)| node.utf8_text(source).ok() == Some(*parameter))
                    && Some(node) != element.filter(|_| substitution.is_some())
                {
                    return None;
                }
                let mut walker = node.walk();
                pending.extend(node.named_children(&mut walker));
            }
        }
        if let Some((parameter, actual)) = substitution {
            let name_node = member.child_by_field_name("name")?;
            let name = match name_node.kind() {
                "property_identifier" => {
                    FieldName::Text(name_node.utf8_text(source).ok()?.to_owned())
                }
                "string" | "number" => simple_property_key(name_node, source)?,
                _ => return None,
            };
            let text = name.text()?.to_owned();
            if text.is_empty()
                || replacements
                    .insert(name.token(), (parameter, actual))
                    .is_some()
            {
                return None;
            }
            if type_node == element {
                fields.push((base_index, text, actual.to_owned()));
            }
        }
    }
    let original = interface_member_identity(base, source)?;
    let mut rest = original.strip_prefix("interface_members:")?;
    let mut result = String::from("interface_members:");
    while !rest.is_empty() {
        let (&tag, tail) = rest.as_bytes().split_first()?;
        let (name, tail) = take_identity_part(std::str::from_utf8(tail).ok()?)?;
        let (signature, tail) = take_identity_part(tail)?;
        let signature = match (tag, replacements.get(name)) {
            (b'v' | b'i', _) => signature.to_owned(),
            (_, Some(&(parameter, actual))) => {
                substitute_signature_token(signature, parameter, actual)?
            }
            (_, None) => signature.to_owned(),
        };
        let _ = write!(
            result,
            "{}{}:{name}{}:{signature}",
            tag as char,
            name.len(),
            signature.len()
        );
        rest = tail;
    }
    Some((result, fields))
}

#[cfg(feature = "lang-typescript")]
fn substitute_signature_token(signature: &str, parameter: &str, actual: &str) -> Option<String> {
    use std::fmt::Write as _;
    let (kind, mut rest) = signature.split_once(':')?;
    let mut result = kind.to_owned();
    let mut replaced = false;
    loop {
        let (token, tail) = take_identity_part(rest)?;
        let token = if token == parameter {
            replaced = true;
            actual
        } else {
            token
        };
        let _ = write!(result, ":{}:{token}", token.len());
        if tail.is_empty() {
            break;
        }
        rest = tail.strip_prefix(':')?;
    }
    replaced.then_some(result)
}

#[cfg(feature = "lang-typescript")]
fn interface_method_signatures(
    identity: &str,
) -> Option<std::collections::HashMap<&str, std::collections::HashSet<(u8, &str)>>> {
    let mut rest = identity.strip_prefix("interface_members:")?;
    let mut methods = std::collections::HashMap::new();
    while !rest.is_empty() {
        let (&tag, tail) = rest.as_bytes().split_first()?;
        if !matches!(tag, b'p' | b'm' | b'o' | b'v' | b'i') {
            return None;
        }
        let tail = std::str::from_utf8(tail).ok()?;
        let (name, tail) = take_identity_part(tail)?;
        let (signature, tail) = take_identity_part(tail)?;
        if matches!(tag, b'm' | b'o') {
            methods
                .entry(name)
                .or_insert_with(std::collections::HashSet::new)
                .insert((tag, signature));
        }
        rest = tail;
    }
    Some(methods)
}

/// Cross-file type spellings can resolve differently, so only disjoint member names prove safety.
#[cfg(feature = "lang-typescript")]
pub(crate) fn imported_base_members_are_disjoint(base: &str, parts: &[&str]) -> bool {
    use std::collections::HashSet;
    if !compatible_interface_members(&[base])
        || !compatible_interface_members(parts)
        || interface_has_index_signature(base) != Some(false)
        || parts
            .iter()
            .any(|part| interface_has_index_signature(part) != Some(false))
    {
        return false;
    }
    fn names(identity: &str) -> Option<HashSet<&str>> {
        let mut rest = identity.strip_prefix("interface_members:")?;
        let mut names = HashSet::new();
        while !rest.is_empty() {
            let (&tag, tail) = rest.as_bytes().split_first()?;
            if !matches!(tag, b'p' | b'm' | b'o' | b'v' | b'i') {
                return None;
            }
            let (name, tail) = take_identity_part(std::str::from_utf8(tail).ok()?)?;
            let (_, tail) = take_identity_part(tail)?;
            if matches!(tag, b'p' | b'm' | b'o') {
                names.insert(name);
            }
            rest = tail;
        }
        Some(names)
    }
    let Some(base_names) = names(base) else {
        return false;
    };
    parts
        .iter()
        .all(|part| names(part).is_some_and(|part_names| base_names.is_disjoint(&part_names)))
}

#[cfg(feature = "lang-typescript")]
fn compatible_interface_members(parts: &[&str]) -> bool {
    use std::collections::{HashMap, HashSet};

    let mut seen: HashMap<&str, (u8, &str)> = HashMap::new();
    let mut index_values = HashMap::new();
    let mut member_values = Vec::new();
    let mut complete_member_values = true;
    for part in parts {
        let Some(mut rest) = part.strip_prefix("interface_members:") else {
            return false;
        };
        let mut own_properties = HashSet::new();
        let mut member_count = 0usize;
        let mut value_count = 0usize;
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
            match tag {
                b'p' | b'm' | b'o' => {
                    if tag == b'p' && !own_properties.insert(name) {
                        return false;
                    }
                    member_count += 1;
                    if let Some(&(previous_tag, previous_signature)) = seen.get(name) {
                        if tag != previous_tag || (tag == b'p' && signature != previous_signature) {
                            return false;
                        }
                    } else {
                        seen.insert(name, (tag, signature));
                    }
                }
                b'v' => {
                    if !seen.contains_key(name)
                        || !matches!(
                            signature,
                            "function"
                                | "unsupported"
                                | "required:string"
                                | "required:number"
                                | "required:boolean"
                                | "optional:string"
                                | "optional:number"
                                | "optional:boolean"
                        )
                    {
                        return false;
                    }
                    value_count += 1;
                    member_values.push(signature);
                }
                b'i' => {
                    if !matches!(name, "string" | "number")
                        || !matches!(
                            signature,
                            "string" | "number" | "boolean" | "any" | "unknown"
                        )
                        || index_values.insert(name, signature).is_some()
                    {
                        return false;
                    }
                }
                _ => return false,
            }
            rest = tail;
        }
        complete_member_values &= member_count == value_count;
    }
    if !index_values.is_empty() && !complete_member_values {
        return false;
    }
    if let (Some(number), Some(string)) = (index_values.get("number"), index_values.get("string"))
        && number != string
    {
        return false;
    }
    for index in index_values.values() {
        if !compatible_index_members(index, &member_values) {
            return false;
        }
    }
    true
}

#[cfg(feature = "lang-typescript")]
fn interface_has_index_signature(identity: &str) -> Option<bool> {
    let mut rest = identity.strip_prefix("interface_members:")?;
    let mut found = false;
    while !rest.is_empty() {
        let (&tag, tail) = rest.as_bytes().split_first()?;
        if !matches!(tag, b'p' | b'm' | b'o' | b'v' | b'i') {
            return None;
        }
        let (_, tail) = take_identity_part(std::str::from_utf8(tail).ok()?)?;
        let (_, tail) = take_identity_part(tail)?;
        found |= tag == b'i';
        rest = tail;
    }
    Some(found)
}

#[cfg(feature = "lang-typescript")]
fn compatible_index_members(index: &str, members: &[&str]) -> bool {
    if matches!(index, "any" | "unknown") {
        return true;
    }
    members
        .iter()
        .all(|member| member.strip_prefix("required:") == Some(index))
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
    fn typescript_scope_and_exported_base_evidence_are_source_backed() {
        use crate::domain::corpus::FileRecord;
        use crate::domain::facts::SourceIdentityKind;
        use crate::domain::role::Role;

        let record = FileRecord {
            path: "scope.TS".to_owned(),
            size: 0,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::TypeScript),
        };
        for (source, expected) in [
            (&b"export interface Base { id: string }"[..], "clear"),
            (
                &b"declare module './base' { interface Base { id: number } }"[..],
                "augmentation",
            ),
            (
                &b"import './base'; declare module './base' { interface Base { extra(): number } }"
                    [..],
                "augmentations",
            ),
            (
                &b"import './base'; declare module './base' { type Extra = string }"[..],
                "augmentation",
            ),
            (
                &b"import './base'; declare module './base' { interface Base { id: number } } declare global { interface Window { x: number } }"[..],
                "augmentation",
            ),
            (
                &b"export {}; declare global { interface Window { id: number } }"[..],
                "augmentation",
            ),
            (&b"export interface Broken { id:"[..], "incomplete"),
            (&b"const word = 'module';"[..], "clear"),
        ] {
            let facts = crate::extract::extract(&record, source);
            assert_eq!(
                facts.source_identity(SourceIdentityKind::TypeScriptScope, 0),
                Some(expected)
            );
        }
        let facts = crate::extract::extract(&record, b"export interface Base { id: string }");
        let index = facts
            .symbols
            .iter()
            .position(|symbol| symbol.name == "Base")
            .map(|index| index as u32);
        assert!(index.is_some_and(|index| {
            facts
                .source_identity(SourceIdentityKind::InterfaceMembers, index)
                .is_some()
        }));
        let distinct = crate::extract::extract(
            &record,
            b"export {}; interface Base { local: string } declare module './base' { interface Base { remote: number } }",
        );
        let interfaces: Vec<_> = distinct
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| symbol.kind == Some(SymKind::Iface) && symbol.name == "Base")
            .map(|(index, _)| index as u32)
            .collect();
        assert_eq!(interfaces.len(), 2);
        assert!(interfaces.iter().all(|&index| {
            distinct
                .source_identity(SourceIdentityKind::InterfaceGroup, index)
                .is_none()
        }));
        let augmented = crate::extract::extract(
            &record,
            b"import './base'; declare module './base' { interface Base { extra(): number } }",
        );
        let index = augmented
            .symbols
            .iter()
            .position(|symbol| symbol.name == "Base")
            .map(|index| index as u32);
        assert!(index.is_some_and(|index| {
            augmented.source_identity(SourceIdentityKind::TypeScriptAugmentation, index)
                == Some("./base")
                && augmented
                    .source_identity(SourceIdentityKind::InterfaceMembers, index)
                    .is_some()
        }));
        let exports = crate::extract::extract(&record, b"class A {} export { A };");
        assert_eq!(
            exports.source_identity(SourceIdentityKind::TypeScriptComplexExports, 0),
            Some("complex")
        );
    }

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

        let indexed = crate::extract::extract(
            &record,
            b"interface Unknown { [x: string]: string }\ninterface Unknown { safe: string }",
        );
        assert_eq!(indexed.symbols.len(), 2);
        let kind = crate::domain::facts::SourceIdentityKind::InterfaceMembers;
        assert!(
            indexed
                .source_identity(kind, 0)
                .is_some_and(|identity| identity.contains("i6:string6:string"))
        );
        assert!(indexed.source_identity(kind, 1).is_some());
        assert_eq!(
            indexed.source_identity(group, 0),
            indexed.source_identity(group, 1)
        );
        assert!(indexed.source_identity(group, 0).is_some());

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
