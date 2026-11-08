//! Static CommonJS callable identity, without interpreting module execution.

use tree_sitter::Node;

use super::engine::{Parsed, unquote};
use crate::domain::facts::{Import, SourceIdentity, SourceIdentityKind, SymKind};
use crate::domain::hash::FastMap;

pub(super) fn post_parse(source: &[u8], root: Node, parsed: &mut Parsed, _: bool) {
    if root.has_error() || parsed.facts.coverage != crate::domain::facts::Coverage::Complete {
        return;
    }
    if parsed
        .facts
        .imports
        .iter()
        .any(|import| import.alias.is_some())
        && unshadowed_require(root, source)
    {
        let stable: Vec<SourceIdentity> =
            stable_require_bindings(root, source, &parsed.facts.imports)
                .into_iter()
                .enumerate()
                .filter(|&(_, stable)| stable)
                .filter_map(|(index, _)| u32::try_from(index).ok())
                .map(|index| SourceIdentity {
                    kind: SourceIdentityKind::CommonJsRequire,
                    index,
                    identity: "unshadowed".to_owned(),
                })
                .collect();
        add_identities(parsed, stable);
    }
    if memchr::memmem::find(source, b"exports").is_none()
        || memchr::memmem::find(source, b"module").is_none()
    {
        return;
    }
    let Some(assignment) = assignment(root, source) else {
        return;
    };
    let Some(value) = assignment.child_by_field_name("right") else {
        return;
    };
    let identity = match value.kind() {
        "identifier" => local_export(value, source, root, parsed),
        "function_expression" => expression_export(value, source, parsed),
        "call_expression" => forwarded_export(value, source, root, parsed),
        _ => None,
    };
    if let Some(identity) = identity {
        add_identity(parsed, SourceIdentityKind::CommonJsExport, 0, identity);
    }
}

fn expression_export(value: Node, source: &[u8], parsed: &mut Parsed) -> Option<String> {
    let name = value.child_by_field_name("name")?.utf8_text(source).ok()?;
    let expected = format!("js-expression:{}:{}", value.start_byte(), value.end_byte());
    let mut matches = parsed
        .facts
        .symbols
        .iter()
        .enumerate()
        .filter(|(index, symbol)| {
            symbol.name.as_ref() == name
                && u32::try_from(*index).ok().is_some_and(|index| {
                    parsed
                        .facts
                        .source_identity(SourceIdentityKind::Symbol, index)
                        == Some(expected.as_str())
                })
        });
    let (index, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    parsed.facts.symbols[index].parent = None;
    Some(format!("expression:{index}"))
}

fn add_identity(parsed: &mut Parsed, kind: SourceIdentityKind, index: u32, identity: String) {
    add_identities(
        parsed,
        vec![SourceIdentity {
            kind,
            index,
            identity,
        }],
    );
}

fn add_identities(parsed: &mut Parsed, added: Vec<SourceIdentity>) {
    if added.is_empty() {
        return;
    }
    let mut identities = parsed
        .facts
        .source_identities
        .take()
        .map_or_else(Vec::new, |items| items.into_vec());
    identities.extend(added);
    identities.sort_unstable_by_key(|item| (item.kind, item.index));
    parsed.facts.source_identities = Some(identities.into_boxed_slice());
}

fn contains_name(node: Node, source: &[u8], name: &str) -> bool {
    nodes(node).any(|node| {
        matches!(
            node.kind(),
            "identifier" | "shorthand_property_identifier_pattern"
        ) && node.utf8_text(source).ok() == Some(name)
    })
}

/// Per import: whether its alias names exactly one binding in the file, a top-level
/// `require` of its own specifier, and nothing else declares, assigns or shadows that name.
/// One walk serves every import, so the cost is linear in the tree, not in imports times tree.
fn stable_require_bindings(root: Node, source: &[u8], imports: &[Import]) -> Vec<bool> {
    struct Binding<'tree> {
        declarators: Vec<Node<'tree>>,
        shadowed: bool,
    }
    let mut bindings: FastMap<&str, Binding> = FastMap::default();
    for import in imports {
        if let Some(name) = import.alias.as_deref().filter(|_| import.names.is_empty()) {
            bindings.entry(name).or_insert_with(|| Binding {
                declarators: Vec::new(),
                shadowed: false,
            });
        }
    }
    if bindings.is_empty() {
        return vec![false; imports.len()];
    }
    let mut named = Vec::new();
    for node in nodes(root) {
        named.clear();
        let declarator = node.kind() == "variable_declarator";
        let binding = if declarator {
            node.child_by_field_name("name")
        } else {
            shadowing_binding(node)
        };
        let Some(binding) = binding else {
            continue;
        };
        bound_names(binding, source, &bindings, &mut named);
        for name in &named {
            let Some(entry) = bindings.get_mut(name) else {
                continue;
            };
            if declarator {
                entry.declarators.push(node);
            } else {
                entry.shadowed = true;
            }
        }
    }
    imports
        .iter()
        .map(|import| {
            let Some(name) = import.alias.as_deref().filter(|_| import.names.is_empty()) else {
                return false;
            };
            bindings.get(name).is_some_and(|entry| {
                !entry.shadowed
                    && matches!(entry.declarators.as_slice(), [only] if require_declaration(*only, source, name, import))
            })
        })
        .collect()
}

/// The subtree of a non-declarator node that binds or writes names: assignment targets,
/// parameters, catch parameters, function and class names.
fn shadowing_binding(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "assignment_expression"
        | "augmented_assignment_expression"
        | "update_expression"
        | "for_in_statement" => node
            .child_by_field_name("left")
            .or_else(|| node.named_child(0))
            .filter(|left| {
                matches!(
                    left.kind(),
                    "identifier" | "array_pattern" | "object_pattern"
                )
            }),
        "formal_parameters" => Some(node),
        "catch_clause" => node.child_by_field_name("parameter"),
        "function_declaration" | "function_expression" | "class_declaration" | "class" => {
            node.child_by_field_name("name")
        }
        "arrow_function" => node.child_by_field_name("parameter"),
        _ => None,
    }
}

/// Tracked names occurring as identifiers in a binding subtree, each once.
fn bound_names<'s>(
    binding: Node,
    source: &'s [u8],
    tracked: &FastMap<&str, impl Sized>,
    named: &mut Vec<&'s str>,
) {
    for node in nodes(binding) {
        if matches!(
            node.kind(),
            "identifier" | "shorthand_property_identifier_pattern"
        ) && let Ok(text) = node.utf8_text(source)
            && tracked.contains_key(text)
            && !named.contains(&text)
        {
            named.push(text);
        }
    }
}

fn require_declaration(node: Node, source: &[u8], name: &str, import: &Import) -> bool {
    let direct = node.parent().is_some_and(|parent| {
        matches!(
            parent.kind(),
            "lexical_declaration" | "variable_declaration"
        ) && parent.parent().is_some_and(|root| root.kind() == "program")
    });
    direct
        && node.child_by_field_name("name").is_some_and(|binding| {
            binding.kind() == "identifier" && binding.utf8_text(source).ok() == Some(name)
        })
        && node.child_by_field_name("value").is_some_and(|value| {
            value.kind() == "call_expression"
                && value
                    .child_by_field_name("function")
                    .is_some_and(|function| {
                        function.kind() == "identifier"
                            && function.utf8_text(source).ok() == Some("require")
                    })
                && value
                    .child_by_field_name("arguments")
                    .is_some_and(|arguments| {
                        arguments.named_child_count() == 1
                            && arguments.named_child(0).is_some_and(|spec| {
                                spec.kind() == "string"
                                    && spec.utf8_text(source).ok().is_some_and(|text| {
                                        !text.contains('\\')
                                            && unquote(text) == import.spec.as_ref()
                                    })
                            })
                    })
        })
}

fn writes_name(node: Node, source: &[u8], name: &str) -> bool {
    matches!(
        node.kind(),
        "assignment_expression"
            | "augmented_assignment_expression"
            | "update_expression"
            | "for_in_statement"
    ) && node
        .child_by_field_name("left")
        .or_else(|| node.named_child(0))
        .is_some_and(|left| {
            matches!(
                left.kind(),
                "identifier" | "array_pattern" | "object_pattern"
            ) && contains_name(left, source, name)
        })
}

/// Named nodes of the subtree in preorder, through one cursor without per-node allocation.
fn nodes(root: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    let mut cursor = root.walk();
    let mut finished = false;
    std::iter::from_fn(move || {
        loop {
            if finished {
                return None;
            }
            let node = cursor.node();
            if !cursor.goto_first_child() {
                while !cursor.goto_next_sibling() {
                    if !cursor.goto_parent() {
                        finished = true;
                        break;
                    }
                }
            }
            if node.is_named() {
                return Some(node);
            }
        }
    })
}

fn assignment<'a>(root: Node<'a>, source: &[u8]) -> Option<Node<'a>> {
    let mut export = None;
    for node in nodes(root) {
        if matches!(
            node.kind(),
            "export_statement" | "import_statement" | "with_statement"
        ) || (node.kind() == "identifier" && {
            let name = node.utf8_text(source).ok()?;
            name.contains('\\') || name == "eval"
        }) {
            return None;
        }
        if matches!(
            node.kind(),
            "identifier"
                | "shorthand_property_identifier"
                | "shorthand_property_identifier_pattern"
        ) && node.utf8_text(source).ok() == Some("module")
            && !node
                .parent()
                .is_some_and(|parent| module_exports(parent, source))
        {
            return None;
        }
        if node.kind() == "assignment_expression"
            && node.child_by_field_name("left").is_some_and(|left| {
                matches!(left.kind(), "array_pattern" | "object_pattern")
                    && nodes(left).any(|item| module_exports(item, source))
            })
        {
            return None;
        }
        if node.kind() == "assignment_expression"
            && node
                .child_by_field_name("left")
                .is_some_and(|left| module_exports(left, source))
        {
            if export.is_some() || !top_level(node, source) {
                return None;
            }
            export = Some(node);
        }
        // An update of the export itself cannot establish a callable identity.
        if matches!(node.kind(), "update_expression" | "unary_expression")
            && node
                .named_child(0)
                .is_some_and(|value| module_exports(value, source))
        {
            return None;
        }
        if matches!(
            node.kind(),
            "augmented_assignment_expression" | "for_in_statement"
        ) && node
            .child_by_field_name("left")
            .is_some_and(|left| module_exports(left, source))
        {
            return None;
        }
    }
    export
}

fn module_exports(node: Node, source: &[u8]) -> bool {
    node.kind() == "member_expression"
        && node.child_by_field_name("object").is_some_and(|object| {
            object.kind() == "identifier" && object.utf8_text(source).ok() == Some("module")
        })
        && node
            .child_by_field_name("property")
            .is_some_and(|property| {
                property.kind() == "property_identifier"
                    && property.utf8_text(source).ok() == Some("exports")
            })
}

fn top_level(mut node: Node, source: &[u8]) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == "assignment_expression"
            && parent.child_by_field_name("right") == Some(node)
            && parent.child_by_field_name("left").is_some_and(|left| {
                left.kind() == "identifier" && left.utf8_text(source).ok() == Some("exports")
            })
        {
            node = parent;
            continue;
        }
        return parent.kind() == "expression_statement"
            && parent.parent().is_some_and(|root| root.kind() == "program");
    }
    false
}

fn local_export(value: Node, source: &[u8], root: Node, parsed: &Parsed) -> Option<String> {
    let name = value.utf8_text(source).ok()?;
    let mut definitions = parsed
        .facts
        .symbols
        .iter()
        .filter(|symbol| symbol.parent.is_none() && symbol.name.as_ref() == name);
    let definition = definitions.next()?;
    if definitions.next().is_some() || definition.kind() != SymKind::Fn {
        return None;
    }
    for node in nodes(root) {
        if writes_name(node, source, name) {
            return None;
        }
        if node.kind() == "variable_declarator"
            && node
                .child_by_field_name("name")
                .is_some_and(|binding| contains_name(binding, source, name))
        {
            return None;
        }
    }
    Some(format!("local:{name}"))
}

fn forwarded_export(value: Node, source: &[u8], root: Node, parsed: &mut Parsed) -> Option<String> {
    let function = value.child_by_field_name("function")?;
    if function.kind() != "identifier" || function.utf8_text(source).ok()? != "require" {
        return None;
    }
    if !unshadowed_require(root, source) {
        return None;
    }
    let arguments = value.child_by_field_name("arguments")?;
    if arguments.named_child_count() != 1 {
        return None;
    }
    let spec = arguments.named_child(0)?;
    if spec.kind() != "string" {
        return None;
    }
    let text = spec.utf8_text(source).ok()?;
    if text.contains('\\') {
        return None;
    }
    let spec = unquote(text);
    parsed.facts.imports.push(Import {
        spec: spec.into(),
        line: u32::try_from(value.start_position().row)
            .ok()?
            .checked_add(1)?,
        ..Import::default()
    });
    Some(format!("module:{spec}"))
}

fn unshadowed_require(root: Node, source: &[u8]) -> bool {
    nodes(root).all(|node| {
        if matches!(
            node.kind(),
            "export_statement" | "import_statement" | "with_statement"
        ) {
            return false;
        }
        if !matches!(
            node.kind(),
            "identifier" | "shorthand_property_identifier_pattern"
        ) {
            return true;
        }
        let Ok(name) = node.utf8_text(source) else {
            return false;
        };
        if name.contains('\\') || name == "eval" {
            return false;
        }
        name != "require"
            || node.parent().is_some_and(|parent| {
                parent.kind() == "call_expression"
                    && parent.child_by_field_name("function") == Some(node)
            })
    })
}
