//! TypeScript and JavaScript: `export` visibility, ES module and CommonJS imports, JSX
//! component usage; `.tsx` uses the TSX grammar.

use super::Pack;
#[cfg(feature = "lang-typescript")]
use crate::domain::facts::FieldName;
use crate::domain::facts::{FLAG_DECLARATION, Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, ImportExpansion, unquote};

#[cfg(feature = "lang-typescript")]
#[path = "typescript_number.rs"]
mod number;

pub const HOOKS: Hooks = Hooks {
    exported,
    signature_prefix,
    flags,
    expand_import: ImportExpansion::Tree(expand_import),
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
    let mut walker = root.walk();
    if root.named_children(&mut walker).any(|node| {
        node.kind() == "comment"
            && node.utf8_text(source).ok().is_some_and(|comment| {
                comment
                    .trim_start()
                    .strip_prefix("///")
                    .is_some_and(|directive| directive.trim_start().starts_with("<reference"))
            })
    }) {
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

/// Source-level TypeScript module indicator status, independent of project compiler options.
#[cfg(feature = "lang-typescript")]
pub(crate) fn module_syntax(
    root: tree_sitter::Node,
    source: &[u8],
    path: &str,
    exceeded: bool,
) -> &'static str {
    if root.has_error() || exceeded {
        return "incomplete";
    }
    if is_typescript_declaration_path(path) {
        return "declaration";
    }
    let mut walker = root.walk();
    if root
        .named_children(&mut walker)
        .any(|node| matches!(node.kind(), "import_statement" | "export_statement"))
    {
        return "indicator";
    }
    let mut pending = vec![root];
    let mut has_jsx = false;
    while let Some(node) = pending.pop() {
        if node.kind().starts_with("jsx_") {
            has_jsx = true;
        }
        if node.kind() == "meta_property" && node.utf8_text(source).ok() == Some("import.meta") {
            return "indicator";
        }
        let mut walker = node.walk();
        pending.extend(node.named_children(&mut walker));
    }
    if has_jsx { "jsx" } else { "none" }
}

#[cfg(feature = "lang-typescript")]
pub(crate) enum GlobalAugmentationEvidence {
    Absent,
    Complete(Vec<crate::domain::facts::SourceIdentity>),
    Unproven,
}

/// Capture complete interface declarations inside every source-level `declare global` block.
#[cfg(feature = "lang-typescript")]
pub(crate) fn global_augmentation_evidence(
    root: tree_sitter::Node,
    source: &[u8],
    path: &str,
    exceeded: bool,
    parsed: &crate::extract::engine::Parsed,
) -> GlobalAugmentationEvidence {
    let mut pending = vec![root];
    let mut blocks = Vec::new();
    while let Some(node) = pending.pop() {
        if node.kind() == "ambient_declaration" {
            let mut walker = node.walk();
            if node
                .children(&mut walker)
                .any(|child| child.kind() == "global")
            {
                let mut walker = node.walk();
                let Some(body) = node
                    .named_children(&mut walker)
                    .find(|child| child.kind() == "statement_block")
                else {
                    return GlobalAugmentationEvidence::Unproven;
                };
                blocks.push(body);
                continue;
            }
        }
        let mut walker = node.walk();
        pending.extend(node.named_children(&mut walker));
    }
    if blocks.is_empty() {
        return GlobalAugmentationEvidence::Absent;
    }
    if root.has_error() || exceeded || parsed.incomplete {
        return GlobalAugmentationEvidence::Unproven;
    }
    if is_typescript_declaration_path(path) {
        return GlobalAugmentationEvidence::Unproven;
    }
    let mut identities = Vec::new();
    for body in blocks {
        if collect_global_declarations(body, source, parsed, &mut identities).is_none() {
            return GlobalAugmentationEvidence::Unproven;
        }
    }
    identities.sort_unstable_by_key(|identity| (identity.kind, identity.index));
    GlobalAugmentationEvidence::Complete(identities)
}

#[cfg(feature = "lang-typescript")]
fn is_typescript_declaration_path(path: &str) -> bool {
    [".d.ts", ".d.mts", ".d.cts", ".d.ets"]
        .iter()
        .any(|suffix| {
            path.len() >= suffix.len()
                && path[path.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
        })
}

#[cfg(feature = "lang-typescript")]
fn collect_global_declarations(
    body: tree_sitter::Node,
    source: &[u8],
    parsed: &crate::extract::engine::Parsed,
    identities: &mut Vec<crate::domain::facts::SourceIdentity>,
) -> Option<()> {
    let mut walker = body.walk();
    for child in body.named_children(&mut walker) {
        if child.kind() == "comment" {
            continue;
        }
        if child.kind() == "export_statement" {
            let declaration = child.child_by_field_name("declaration")?;
            collect_global_declaration(declaration, source, parsed, identities)?;
        } else {
            collect_global_declaration(child, source, parsed, identities)?;
        }
    }
    Some(())
}

#[cfg(feature = "lang-typescript")]
fn collect_global_declaration(
    node: tree_sitter::Node,
    source: &[u8],
    parsed: &crate::extract::engine::Parsed,
    identities: &mut Vec<crate::domain::facts::SourceIdentity>,
) -> Option<()> {
    use crate::domain::facts::{SourceIdentity, SourceIdentityKind, SymKind};

    match node.kind() {
        "interface_declaration" => {
            let name = node.child_by_field_name("name")?.utf8_text(source).ok()?;
            if name.contains('\\') {
                return None;
            }
            let mut matches = parsed
                .spans
                .iter()
                .enumerate()
                .filter(|(_, span)| span.0 == node.start_byte() && span.1 == node.end_byte());
            let (index, _) = matches.next()?;
            if matches.next().is_some()
                || parsed.facts.symbols.get(index)?.kind != Some(SymKind::Iface)
            {
                return None;
            }
            let index = u32::try_from(index).ok()?;
            let header = crate::extract::engine::signature_identity(node, source)?;
            let members = interface_member_identity(node, source)?;
            for (kind, identity) in [
                (SourceIdentityKind::InterfaceHeader, header),
                (SourceIdentityKind::InterfaceMembers, members),
            ] {
                if let Some(existing) = parsed.facts.source_identity(kind, index) {
                    if existing != identity {
                        return None;
                    }
                } else {
                    identities.push(SourceIdentity {
                        kind,
                        index,
                        identity,
                    });
                }
            }
            identities.push(SourceIdentity {
                kind: SourceIdentityKind::TypeScriptGlobalAugmentation,
                index,
                identity: "global".to_owned(),
            });
            Some(())
        }
        "internal_module" | "module" => {
            let name = node.child_by_field_name("name")?;
            if !matches!(name.kind(), "identifier" | "nested_identifier")
                || name.utf8_text(source).ok()?.contains('\\')
            {
                return None;
            }
            let body = node.child_by_field_name("body")?;
            if body.kind() != "statement_block" {
                return None;
            }
            collect_global_declarations(body, source, parsed, identities)
        }
        "expression_statement" => {
            let mut walker = node.walk();
            let mut children = node.named_children(&mut walker);
            let declaration = children.next()?;
            if children.next().is_some() || declaration.kind() != "internal_module" {
                return None;
            }
            collect_global_declaration(declaration, source, parsed, identities)
        }
        _ => None,
    }
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

/// Decode and validate a TypeScript identifier while retaining ordinary Unicode spelling.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn normalize_identifier_name(name: &str) -> Option<String> {
    if !name.contains('\\') {
        return Some(name.to_owned());
    }
    let decoded = decode_identifier_name(name)?;
    static IDENTIFIER: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
    let identifier = IDENTIFIER
        .get_or_init(|| {
            regex::Regex::new(r"\A[$_\p{ID_Start}][$\p{ID_Continue}\u{200C}\u{200D}_]*\z").ok()
        })
        .as_ref()?;
    identifier.is_match(&decoded).then_some(decoded)
}

/// Stores flat source type combinations without collapsing them to their first name.
#[cfg(feature = "lang-typescript")]
pub(crate) fn field_type_identity(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::domain::facts::{
        TYPESCRIPT_INTERSECTION_FIELD_TYPE_PREFIX, TYPESCRIPT_UNION_FIELD_TYPE_PREFIX,
    };

    let (prefix, separator) = match node.kind() {
        "union_type" => (TYPESCRIPT_UNION_FIELD_TYPE_PREFIX, '|'),
        "intersection_type" => (TYPESCRIPT_INTERSECTION_FIELD_TYPE_PREFIX, '&'),
        "array_type" | "tuple_type" | "parenthesized_type" => return None,
        _ => {
            return node
                .utf8_text(source)
                .ok()
                .map(crate::extract::engine::type_name);
        }
    };
    if has_optional_field_marker(node, source) {
        return None;
    }
    let mut cursor = node.walk();
    let names: Vec<String> = node
        .named_children(&mut cursor)
        .map(|child| simple_type_reference_name(child, source))
        .collect::<Option<_>>()?;
    if names.len() < 2 {
        return None;
    }
    Some(format!("{prefix}{}", names.join(&separator.to_string())))
}

#[cfg(feature = "lang-typescript")]
fn has_optional_field_marker(node: tree_sitter::Node, source: &[u8]) -> bool {
    let Some(annotation) = node
        .parent()
        .filter(|parent| parent.kind() == "type_annotation")
    else {
        return false;
    };
    let Some(field) = annotation.parent().filter(|parent| {
        matches!(
            parent.kind(),
            "property_signature" | "public_field_definition"
        )
    }) else {
        return false;
    };
    let Some(name) = field.child_by_field_name("name") else {
        return false;
    };
    source
        .get(name.end_byte()..annotation.start_byte())
        .is_some_and(|between| between.contains(&b'?'))
}

#[cfg(feature = "lang-typescript")]
fn simple_type_reference_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => normalize_identifier_name(node.utf8_text(source).ok()?),
        "nested_type_identifier" => Some(format!(
            "{}.{}",
            simple_type_reference_name(node.child_by_field_name("module")?, source)?,
            simple_type_reference_name(node.child_by_field_name("name")?, source)?
        )),
        _ => None,
    }
}

/// Normalize escaped names in the dotted receiver chains accepted by the resolver.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn normalize_member_qualifier(
    node: tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    fn member_chain(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        match node.kind() {
            "identifier" | "property_identifier" => {
                normalize_identifier_name(node.utf8_text(source).ok()?)
            }
            "this" | "super" => node.utf8_text(source).ok().map(str::to_owned),
            "member_expression" => {
                let object = node.child_by_field_name("object")?;
                let property = node.child_by_field_name("property")?;
                if source.get(object.end_byte()..property.start_byte())? != b"." {
                    return None;
                }
                Some(format!(
                    "{}.{}",
                    member_chain(object, source)?,
                    normalize_identifier_name(property.utf8_text(source).ok()?)?
                ))
            }
            _ => None,
        }
    }

    let text = node.utf8_text(source).ok()?;
    if !text.contains('\\') {
        return Some(text.to_owned());
    }
    member_chain(node, source)
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
            "property_identifier" => FieldName::Text(normalize_identifier_name(
                name_node.utf8_text(source).ok()?,
            )?),
            "string" | "number" => simple_property_key(name_node, source)?,
            _ => return None,
        };
        let name = name.token();
        let signature =
            crate::extract::engine::signature_identity_without_node(member, source, name_node)?;
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

/// Namespace path of a declaration, flattened across dotted and nested namespace syntax.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn interface_namespace_path(
    node: tree_sitter::Node<'_>,
    source: &[u8],
) -> Option<Vec<String>> {
    let mut scopes = Vec::new();
    let mut current = node.parent();
    while let Some(ancestor) = current {
        match ancestor.kind() {
            "internal_module" | "module" => {
                let name = ancestor.child_by_field_name("name")?;
                if name.kind() == "string" {
                    return None;
                }
                if !matches!(name.kind(), "identifier" | "nested_identifier") {
                    return None;
                }
                let name = name.utf8_text(source).ok()?;
                let parts: Vec<_> = name.split('.').map(|part| part.trim().to_owned()).collect();
                if parts.iter().any(String::is_empty) {
                    return None;
                }
                scopes.push(parts);
            }
            "global_declaration" => return None,
            "ambient_declaration" => {
                let mut walker = ancestor.walk();
                if ancestor
                    .children(&mut walker)
                    .any(|child| child.kind() == "global")
                {
                    return None;
                }
            }
            "statement_block"
                if ancestor.parent().is_none_or(|parent| {
                    !matches!(parent.kind(), "internal_module" | "module")
                }) =>
            {
                return None;
            }
            _ => {}
        }
        current = ancestor.parent();
    }
    scopes.reverse();
    Some(scopes.into_iter().flatten().collect())
}

/// Group same-file declarations whose namespace paths and complete source facts prove compatible.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn interface_groups(
    symbols: &[crate::domain::facts::Symbol],
    identities: &[crate::domain::facts::SourceIdentity],
    nodes: &[Option<tree_sitter::Node<'_>>],
    source: &[u8],
) -> Vec<crate::domain::facts::SourceIdentity> {
    use crate::domain::facts::{SourceIdentity, SourceIdentityKind};
    use std::collections::{HashMap, HashSet};

    let candidate_names: HashSet<_> = identities
        .iter()
        .filter(|identity| identity.kind == SourceIdentityKind::InterfaceMembers)
        .filter_map(|identity| symbols.get(identity.index as usize))
        .map(|symbol| symbol.name.as_str())
        .collect();

    let mut namespaces: HashMap<Vec<String>, HashMap<&str, Vec<u32>>> = HashMap::new();
    for (index, symbol) in symbols.iter().enumerate() {
        let mut parent = symbol.parent;
        let mut namespace_level = true;
        let mut remaining = symbols.len();
        while let Some(scope) = parent {
            if remaining == 0 {
                namespace_level = false;
                break;
            }
            remaining -= 1;
            let Some(scope) = symbols.get(scope as usize) else {
                namespace_level = false;
                break;
            };
            if scope.kind != Some(SymKind::Mod) {
                namespace_level = false;
                break;
            }
            parent = scope.parent;
        }
        if !namespace_level {
            continue;
        }
        let Some(namespace) = nodes
            .get(index)
            .copied()
            .flatten()
            .and_then(|node| interface_namespace_path(node, source))
        else {
            continue;
        };
        if let Ok(index) = u32::try_from(index) {
            namespaces
                .entry(namespace)
                .or_default()
                .entry(&symbol.name)
                .or_default()
                .push(index);
        }
    }
    let lookup: HashMap<_, _> = identities
        .iter()
        .map(|identity| ((identity.kind, identity.index), identity.identity.as_str()))
        .collect();
    let mut groups = Vec::new();
    for (namespace, names) in namespaces {
        let escaped_names: Vec<_> = names
            .keys()
            .filter(|name| name.contains('\\'))
            .map(|name| {
                decode_string_content(name).and_then(|units| String::from_utf16(&units).ok())
            })
            .collect();
        for (name, parts) in &names {
            if !candidate_names.contains(name) || parts.len() < 2 {
                continue;
            }
            let first = &symbols[parts[0] as usize];
            if first.name.contains('\\') || escaped_name_may_match(&first.name, &escaped_names) {
                continue;
            }
            let Some(header) = nodes[parts[0] as usize]
                .and_then(|node| interface_header_without_bases(node, source))
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
                let Some(members) = lookup.get(&(SourceIdentityKind::InterfaceMembers, part))
                else {
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
                if !namespace.is_empty() {
                    continue;
                }
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
                "property_identifier" => FieldName::Text(normalize_identifier_name(
                    name_node.utf8_text(source).ok()?,
                )?),
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

/// Complete compatible interface facts for declarations with the same global header.
#[cfg(feature = "lang-typescript")]
pub(crate) fn compatible_global_interface_declarations(headers: &[&str], members: &[&str]) -> bool {
    let Some(header) = headers.first() else {
        return false;
    };
    headers.len() == members.len()
        && headers.iter().all(|candidate| candidate == header)
        && compatible_interface_members(members)
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
fn decode_identifier_name(name: &str) -> Option<String> {
    let mut chars = name.chars().peekable();
    let mut decoded = String::with_capacity(name.len());
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        if chars.next()? != 'u' {
            return None;
        }
        let value = if chars.peek() == Some(&'{') {
            chars.next();
            let mut value = 0u32;
            let mut digits = 0;
            while chars.peek() != Some(&'}') {
                if digits == 6 {
                    return None;
                }
                value = value
                    .checked_mul(16)?
                    .checked_add(chars.next()?.to_digit(16)?)?;
                digits += 1;
            }
            if digits == 0 || chars.next() != Some('}') {
                return None;
            }
            value
        } else {
            read_hex(&mut chars, 4)?
        };
        decoded.push(char::from_u32(value)?);
    }
    Some(decoded)
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
fn expand_import(
    statement: tree_sitter::Node,
    source: &[u8],
    line: u32,
    _reexport: bool,
) -> Vec<Import> {
    let text = |node: tree_sitter::Node| node.utf8_text(source).unwrap_or_default();
    let namespace_alias = |node: tree_sitter::Node| {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|child| matches!(child.kind(), "identifier" | "string"))
            .and_then(|child| child.utf8_text(source).ok())
            .map(unquote)
    };
    let Some(spec) = statement.child_by_field_name("source") else {
        return Vec::new();
    };
    let spec = unquote(text(spec)).to_owned();
    let reexport = statement.kind() == "export_statement";
    let statement_type_only = type_only_import(statement);
    let import = |name: &str, alias: Option<&str>, type_only: bool| Import {
        spec: spec.clone(),
        alias: alias.map(str::to_owned),
        names: vec![name.to_owned()],
        line,
        reexport,
        type_only: statement_type_only || type_only,
    };
    let mut imports = Vec::new();
    // TypeScript's contextual `type` keyword is unnamed; an imported value literally
    // named `type` is the specifier's name field and must not be stripped.
    let mut clauses = statement.walk();
    for clause in statement.named_children(&mut clauses) {
        let mut bindings = clause.walk();
        match clause.kind() {
            "import_clause" | "export_clause" => {
                for binding in clause.named_children(&mut bindings) {
                    match binding.kind() {
                        "identifier" => {
                            imports.push(import("default", Some(text(binding)), false));
                        }
                        "namespace_import" => {
                            if let Some(alias) = namespace_alias(binding) {
                                imports.push(import("*", Some(alias), false));
                            }
                        }
                        "named_imports" => {
                            let mut names = binding.walk();
                            for name in binding.named_children(&mut names) {
                                if let Some(name) = import_specifier(name, source) {
                                    imports.push(import(name.0, name.1, name.2));
                                }
                            }
                        }
                        "export_specifier" => {
                            if let Some(name) = import_specifier(binding, source) {
                                imports.push(import(name.0, name.1, name.2));
                            }
                        }
                        _ => {}
                    }
                }
            }
            "namespace_export" => {
                if let Some(alias) = namespace_alias(clause) {
                    imports.push(import("*", Some(alias), false));
                }
            }
            _ => {}
        }
    }
    if imports.is_empty() {
        let mut cursor = statement.walk();
        let glob = reexport
            && statement
                .children(&mut cursor)
                .any(|child| child.kind() == "*");
        imports.push(Import {
            spec,
            names: if glob {
                vec!["*".to_owned()]
            } else {
                Vec::new()
            },
            line,
            reexport,
            type_only: statement_type_only,
            ..Import::default()
        });
    }
    imports
}

fn type_only_import(node: tree_sitter::Node) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| !child.is_named() && child.kind() == "type")
}

fn import_specifier<'a>(
    node: tree_sitter::Node,
    source: &'a [u8],
) -> Option<(&'a str, Option<&'a str>, bool)> {
    if node.has_error() {
        // The TS grammar treats a value re-export `type as alias` as a type
        // modifier followed by an erroneous `as`. Recover only that complete
        // token sequence, whose names remain explicit in source.
        let mut cursor = node.walk();
        let mut children = node
            .children(&mut cursor)
            .filter(|child| child.kind() != "comment");
        let name = children.next()?;
        let separator = children.next()?;
        let alias = children.next()?;
        if node.kind() == "export_specifier"
            && name.kind() == "type"
            && separator.kind() == "ERROR"
            && separator.utf8_text(source).ok()? == "as"
            && matches!(alias.kind(), "identifier" | "string")
            && !alias.is_missing()
            && !alias.has_error()
            && children.next().is_none()
        {
            return Some((
                name.utf8_text(source).ok()?,
                Some(unquote(alias.utf8_text(source).ok()?)),
                false,
            ));
        }
        return None;
    }
    let name = node.child_by_field_name("name")?;
    let alias = node.child_by_field_name("alias");
    Some((
        unquote(name.utf8_text(source).ok()?),
        alias
            .and_then(|node| node.utf8_text(source).ok())
            .map(unquote),
        type_only_import(node),
    ))
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
    fn generated_function_declarations_share_complete_source_header_identity() {
        use crate::domain::corpus::FileRecord;
        use crate::domain::facts::{SourceIdentityKind, TYPESCRIPT_CALLABLE_IDENTITY_PREFIX};
        use crate::domain::role::Role;

        fn identity(path: &str, source: &[u8]) -> Option<String> {
            let record = FileRecord {
                path: path.to_owned(),
                size: source.len() as u64,
                mtime_ns: 0,
                identity: None,
                hash: 0,
                role: Role::Src,
                file_type: Some(FileType::TypeScript),
            };
            let facts = crate::extract::extract(&record, source);
            let local = facts
                .symbols
                .iter()
                .position(|symbol| symbol.name == "make")?;
            facts
                .source_identity(SourceIdentityKind::Symbol, u32::try_from(local).ok()?)
                .map(str::to_owned)
        }

        let source = identity(
            "widget.ts",
            b"export function make(id: string): string { return id; }",
        );
        let declaration = identity(
            "widget.d.ts",
            b"export declare function make(id: string): string;",
        );
        assert!(
            source
                .as_deref()
                .is_some_and(|identity| identity.starts_with(TYPESCRIPT_CALLABLE_IDENTITY_PREFIX))
        );
        assert_eq!(source, declaration);
    }

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
            (
                &b"/// <reference path=\"./types.d.ts\" />\ninterface Base { id: string }"[..],
                "incomplete",
            ),
            (
                &b"///<reference path=\"./types.d.ts\"/>\ninterface Base { id: string }"[..],
                "incomplete",
            ),
            (
                &b"///    <reference path=\"./types.d.ts\"/>\ninterface Base { id: string }"[..],
                "incomplete",
            ),
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
        let script = crate::extract::extract(&record, b"interface Config { width: number }");
        assert_eq!(
            script.source_identity(SourceIdentityKind::TypeScriptModuleSyntax, 0),
            Some("none")
        );
        let module =
            crate::extract::extract(&record, b"export {}; interface Config { width: number }");
        assert_eq!(
            module.source_identity(SourceIdentityKind::TypeScriptModuleSyntax, 0),
            Some("indicator")
        );
        let jsx_record = FileRecord {
            path: "component.tsx".to_owned(),
            size: 0,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::TypeScript),
        };
        let jsx = crate::extract::extract(&jsx_record, b"const view = <div />;");
        assert_eq!(
            jsx.source_identity(SourceIdentityKind::TypeScriptModuleSyntax, 0),
            Some("jsx")
        );
        let declaration_record = FileRecord {
            path: "types.d.ts".to_owned(),
            size: 0,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::TypeScript),
        };
        let declaration = crate::extract::extract(
            &declaration_record,
            b"export {}; declare global { interface Window { id: string } }",
        );
        assert_eq!(
            declaration.source_identity(SourceIdentityKind::TypeScriptModuleSyntax, 0),
            Some("declaration")
        );
        assert_eq!(
            declaration.source_identity(SourceIdentityKind::TypeScriptGlobalAugmentationScope, 0),
            Some("unproven")
        );
        assert!(
            declaration
                .source_identities
                .as_deref()
                .is_none_or(|identities| !identities.iter().any(|identity| {
                    identity.kind == SourceIdentityKind::TypeScriptGlobalAugmentation
                }))
        );
        let global = crate::extract::extract(
            &record,
            b"export {}; declare global { interface Window { id: string } }",
        );
        assert_eq!(
            global.source_identity(SourceIdentityKind::TypeScriptGlobalAugmentationScope, 0),
            Some("complete")
        );
        let global_parts: Vec<_> = global
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| symbol.kind == Some(SymKind::Iface))
            .map(|(index, _)| index as u32)
            .collect();
        assert_eq!(global_parts.len(), 1);
        assert!(global_parts.iter().all(|&index| {
            global.source_identity(SourceIdentityKind::TypeScriptGlobalAugmentation, index)
                == Some("global")
                && global
                    .source_identity(SourceIdentityKind::InterfaceHeader, index)
                    .is_some()
                && global
                    .source_identity(SourceIdentityKind::InterfaceMembers, index)
                    .is_some()
        }));
        let nested_global = crate::extract::extract(
            &record,
            b"export {}; declare global { namespace Space { export interface Config { left: string } } }",
        );
        assert_eq!(
            nested_global.source_identity(SourceIdentityKind::TypeScriptGlobalAugmentationScope, 0),
            Some("complete")
        );
        let config_index = nested_global
            .symbols
            .iter()
            .position(|symbol| symbol.name == "Config");
        assert!(config_index.is_some_and(|index| {
            nested_global.symbols[index]
                .parent
                .and_then(|parent| nested_global.symbols.get(parent as usize))
                .is_some_and(|parent| parent.name == "Space")
                && nested_global.source_identity(
                    SourceIdentityKind::TypeScriptGlobalAugmentation,
                    index as u32,
                ) == Some("global")
        }));
        let unsupported_global = crate::extract::extract(
            &record,
            b"export {}; declare global { interface Window { id: string } type Extra = string }",
        );
        assert_eq!(
            unsupported_global
                .source_identity(SourceIdentityKind::TypeScriptGlobalAugmentationScope, 0),
            Some("unproven")
        );
        let mixed_augmentation = crate::extract::extract(
            &record,
            b"import './base'; declare module './base' { interface Base { extra(): number } } declare global { interface Window { id: string } }",
        );
        assert_eq!(
            mixed_augmentation.source_identity(SourceIdentityKind::TypeScriptScope, 0),
            Some("augmentation")
        );
        assert_eq!(
            mixed_augmentation
                .source_identity(SourceIdentityKind::TypeScriptGlobalAugmentationScope, 0),
            Some("complete")
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
                    && facts.symbols[identity.index as usize].parent.is_none()
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
                    && facts.symbols[identity.index as usize].parent.is_none()
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
    fn identifier_escape_decoding_preserves_valid_unicode_and_rejects_invalid_names() {
        for (source, expected) in [
            ("\\u0061", "a"),
            ("a\\u{03B1}", "aα"),
            ("\\u2118", "℘"),
            ("a\\u200Cb", "a\u{200C}b"),
            ("é", "é"),
        ] {
            assert_eq!(normalize_identifier_name(source).as_deref(), Some(expected));
        }
        for invalid in [
            "\\u002D",
            "\\u0301a",
            "\\u{1F600}",
            "\\uD800",
            "\\u{}",
            "\\u{110000}",
            "\\u{1234567}",
            "\\u12",
            "\\x61",
        ] {
            assert_eq!(normalize_identifier_name(invalid), None, "{invalid}");
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
            assert_eq!(
                config.types[3],
                format!(
                    "{}Other|Unrelated",
                    crate::domain::facts::TYPESCRIPT_UNION_FIELD_TYPE_PREFIX
                )
            );
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
        use crate::domain::corpus::FileRecord;
        use crate::domain::role::Role;

        #[cfg(feature = "lang-typescript")]
        let file_type = FileType::TypeScript;
        #[cfg(not(feature = "lang-typescript"))]
        let file_type = FileType::JavaScript;
        let record = FileRecord {
            path: if file_type == FileType::TypeScript {
                "entry.ts"
            } else {
                "entry.js"
            }
            .to_owned(),
            size: statement.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(file_type),
        };
        crate::extract::extract(&record, statement.as_bytes())
            .imports
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
            rows("import Store, { open as openStore, Row } from \"./store\";"),
            [
                row("./store", "Row", None, false),
                row("./store", "default", Some("Store"), false),
                row("./store", "open", Some("openStore"), false)
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
        assert_eq!(
            rows("export * as store from './store';"),
            [row("./store", "*", Some("store"), true)]
        );
        assert_eq!(
            rows("export {} from './empty';"),
            [row("./empty", "", None, true)]
        );
        #[cfg(feature = "lang-typescript")]
        assert_eq!(
            rows("import Store, { open as openStore, type Row } from \"./store\";"),
            [
                row("./store", "Row", None, false),
                row("./store", "default", Some("Store"), false),
                row("./store", "open", Some("openStore"), false)
            ]
        );
    }

    #[test]
    fn import_syntax_preserves_comments_and_literal_type_names() {
        assert_eq!(
            rows(
                "import /* module */ Store, { open /* alias */ as openStore, type as named } from './ from /store';"
            ),
            [
                row("./ from /store", "default", Some("Store"), false),
                row("./ from /store", "open", Some("openStore"), false),
                row("./ from /store", "type", Some("named"), false)
            ]
        );
        assert_eq!(
            rows("export { type /* alias */ as named } /* source */ from './api';"),
            [row("./api", "type", Some("named"), true)]
        );
        assert_eq!(
            rows("import * /* alias */ as tools from './tools';"),
            [row("./tools", "*", Some("tools"), false)]
        );
        #[cfg(feature = "lang-typescript")]
        {
            assert_eq!(
                rows("export { type as } from './api';"),
                [row("./api", "as", None, true)]
            );
            assert_eq!(
                rows("export { type as alias extra } from './api';"),
                [row("./api", "", None, true)]
            );
        }
    }
}
