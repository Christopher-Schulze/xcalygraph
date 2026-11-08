//! Python: leading-underscore privacy, `test_*` and `Test*` tests, `__main__` entry, import
//! statements expanded per name.

use super::Pack;
use crate::domain::facts::{
    Coverage, FLAG_TEST, Import, SourceIdentity, SourceIdentityKind, SymKind,
};
use crate::domain::filetype::FileType;
use crate::domain::hash::{FastMap, FastSet};
use crate::domain::name::Name;
use crate::extract::engine::{Definition, FileContext, Hooks, ImportExpansion};
use tree_sitter::Node;

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Python,
    language: |_| tree_sitter_python::LANGUAGE.into(),
    query: include_str!("../queries/python.scm"),
    hooks: Hooks {
        exported: |definition| !definition.name.starts_with('_'),
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(expand_import),
        builtin,
    },
};

/// Preserve direct module imports separately from imports nested in executable branches.
pub(in crate::extract) fn post_parse(
    source: &[u8],
    root: Node,
    parsed: &mut crate::extract::engine::Parsed,
    _: bool,
) {
    if root.has_error() || parsed.facts.coverage != Coverage::Complete {
        return;
    }
    if parsed.facts.imports.is_empty() {
        let mut names = FastSet::default();
        if !parsed
            .facts
            .symbols
            .iter()
            .filter(|symbol| symbol.parent.is_none() && symbol.kind() == SymKind::Fn)
            .any(|symbol| !names.insert(symbol.name.as_str()))
        {
            return;
        }
    }
    let mut identities = parsed
        .facts
        .source_identities
        .take()
        .map_or_else(Vec::new, |items| items.into_vec());
    let mut direct_imports = vec![false; parsed.facts.imports.len()];
    let mut direct_definitions = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if node.kind() == "function_definition" {
            let start = parsed
                .spans
                .partition_point(|(start, _)| *start < node.start_byte());
            for index in start..parsed.spans.len() {
                if parsed.spans[index].0 != node.start_byte() {
                    break;
                }
                if parsed
                    .facts
                    .symbols
                    .get(index)
                    .is_some_and(|symbol| symbol.kind() == SymKind::Fn && symbol.parent.is_none())
                {
                    direct_definitions.push(index);
                }
            }
        }
        if !matches!(node.kind(), "import_statement" | "import_from_statement") {
            continue;
        }
        let Ok(text) = node.utf8_text(source) else {
            continue;
        };
        let Ok(line) = u32::try_from(node.start_position().row + 1) else {
            continue;
        };
        let start = parsed
            .facts
            .imports
            .partition_point(|import| import.line < line);
        let end = parsed
            .facts
            .imports
            .partition_point(|import| import.line <= line);
        for import in expand_import(text, line, false) {
            for (offset, _) in parsed.facts.imports[start..end]
                .iter()
                .enumerate()
                .filter(|(_, found)| **found == import)
            {
                if let Ok(index) = u32::try_from(start + offset) {
                    direct_imports[index as usize] = true;
                    identities.push(SourceIdentity {
                        kind: SourceIdentityKind::PythonDirectImport,
                        index,
                        identity: "direct".to_owned(),
                    });
                }
            }
        }
    }
    let mut scopes = vec![None; parsed.facts.imports.len()];
    let mut after_tries = vec![None; parsed.facts.imports.len()];
    let mut optional_tries = vec![None; parsed.facts.imports.len()];
    let mut repeated = vec![false; parsed.facts.imports.len()];
    let mut type_candidates = vec![false; parsed.facts.imports.len()];
    let mut writes: FastMap<Option<u32>, FastSet<&str>> = FastMap::default();
    let mut module_write_counts: FastMap<&str, usize> = FastMap::default();
    let mut module_definition_counts: FastMap<&str, usize> = FastMap::default();
    let mut callable_fallbacks = Vec::new();
    let mut external_writes: FastSet<&str> = FastSet::default();
    let mut expression_writes: FastMap<&str, Vec<std::ops::Range<u32>>> = FastMap::default();
    let mut walk = root.walk();
    loop {
        let node = walk.node();
        if node.is_named() {
            let target = match node.kind() {
                "assignment" | "augmented_assignment" | "for_statement" => {
                    node.child_by_field_name("left")
                }
                "named_expression" => node.child_by_field_name("name"),
                "as_pattern_target" | "delete_statement" | "parameters" | "case_pattern" => {
                    Some(node)
                }
                _ => None,
            };
            if let Some(target) = target {
                let owner = import_owner(target.start_byte(), parsed);
                let optional_fallback = is_optional_import_fallback(node, source);
                if !optional_fallback
                    && target.kind() == "attribute"
                    && target
                        .child_by_field_name("object")
                        .and_then(|item| item.utf8_text(source).ok())
                        == Some("typing")
                    && target
                        .child_by_field_name("attribute")
                        .and_then(|item| item.utf8_text(source).ok())
                        == Some("TYPE_CHECKING")
                {
                    external_writes.insert("typing");
                }
                collect_written_names(target, source, &mut |name| {
                    if owner.is_none() {
                        *module_write_counts.entry(name).or_default() += 1;
                    }
                    if !optional_fallback {
                        writes.entry(owner).or_default().insert(name);
                    }
                });
            }
            if node.kind() == "lambda_parameters" {
                let range = node
                    .parent()
                    .and_then(|lambda| lambda.child_by_field_name("body"))
                    .and_then(|body| {
                        Some(
                            u32::try_from(body.start_byte()).ok()?
                                ..u32::try_from(body.end_byte()).ok()?,
                        )
                    });
                let owner = import_owner(node.start_byte(), parsed);
                collect_written_names(node, source, &mut |name| {
                    if let Some(range) = &range {
                        expression_writes
                            .entry(name)
                            .or_default()
                            .push(range.clone());
                    } else {
                        writes.entry(owner).or_default().insert(name);
                    }
                });
            }
            if node.kind() == "for_in_clause"
                && let Some(target) = node.child_by_field_name("left")
            {
                let mut parent = node.parent();
                while parent.is_some_and(|item| {
                    !matches!(
                        item.kind(),
                        "list_comprehension"
                            | "set_comprehension"
                            | "dictionary_comprehension"
                            | "generator_expression"
                            | "function_definition"
                            | "module"
                    )
                }) {
                    parent = parent.and_then(|item| item.parent());
                }
                let range = parent
                    .filter(|item| {
                        matches!(
                            item.kind(),
                            "list_comprehension"
                                | "set_comprehension"
                                | "dictionary_comprehension"
                                | "generator_expression"
                        )
                    })
                    .and_then(|item| {
                        Some(
                            u32::try_from(item.start_byte()).ok()?
                                ..u32::try_from(item.end_byte()).ok()?,
                        )
                    });
                let owner = import_owner(node.start_byte(), parsed);
                collect_written_names(target, source, &mut |name| {
                    if let Some(range) = &range {
                        expression_writes
                            .entry(name)
                            .or_default()
                            .push(range.clone());
                    } else {
                        writes.entry(owner).or_default().insert(name);
                    }
                });
            }
            if matches!(node.kind(), "global_statement" | "nonlocal_statement") {
                collect_written_names(node, source, &mut |name| {
                    external_writes.insert(name);
                });
            }
            if node.kind() == "try_statement"
                && node
                    .parent()
                    .is_some_and(|parent| parent.kind() == "module")
                && let Some(fallback) = callable_import_fallback(node, source, parsed)
            {
                callable_fallbacks.push(fallback);
            }
            if matches!(node.kind(), "function_definition" | "class_definition")
                && let Some(name) = node.child_by_field_name("name")
                && let Ok(name) = name.utf8_text(source)
            {
                let owner = import_owner(node.start_byte(), parsed)
                    .and_then(|index| parsed.facts.symbols.get(index as usize)?.parent);
                if owner.is_none() {
                    *module_definition_counts.entry(name).or_default() += 1;
                }
                writes.entry(owner).or_default().insert(name);
            }
            if matches!(node.kind(), "import_statement" | "import_from_statement")
                && node
                    .parent()
                    .is_some_and(|parent| parent.kind() != "module")
                && let (Ok(text), Ok(line)) = (
                    node.utf8_text(source),
                    u32::try_from(node.start_position().row + 1),
                )
            {
                let identity = import_scope_identity(node, parsed);
                let after_try = import_after_try_identity(node, parsed);
                let type_checking = is_type_checking_import(node, source);
                let start = parsed
                    .facts
                    .imports
                    .partition_point(|import| import.line < line);
                let end = parsed
                    .facts
                    .imports
                    .partition_point(|import| import.line <= line);
                for import in expand_import(text, line, false) {
                    let optional_try = import_optional_try_identity(node, &import, source, parsed);
                    for (offset, _) in parsed.facts.imports[start..end]
                        .iter()
                        .enumerate()
                        .filter(|(_, found)| **found == import)
                    {
                        let index = start + offset;
                        if scopes[index].is_some()
                            || after_tries[index].is_some()
                            || optional_tries[index].is_some()
                            || type_candidates[index]
                        {
                            repeated[index] = true;
                        } else {
                            scopes[index] = identity.clone();
                            after_tries[index] = after_try.clone();
                            optional_tries[index] = optional_try.clone();
                            type_candidates[index] = type_checking;
                        }
                    }
                }
            }
            if walk.goto_first_child() {
                continue;
            }
        }
        while !walk.goto_next_sibling() {
            if !walk.goto_parent() {
                for (index, import) in parsed.facts.imports.iter().enumerate() {
                    let Some(name) = bound_import_name(import) else {
                        continue;
                    };
                    if module_definition_counts.contains_key(name)
                        && !module_write_counts.contains_key(name)
                        && !external_writes.contains(name)
                        && let Ok(index) = u32::try_from(index)
                        && direct_imports[index as usize]
                    {
                        identities.push(SourceIdentity {
                            kind: SourceIdentityKind::PythonImportNoModuleWrite,
                            index,
                            identity: "unwritten".to_owned(),
                        });
                    }
                }
                let typing_imports: Vec<_> = parsed
                    .facts
                    .imports
                    .iter()
                    .enumerate()
                    .filter(|(_, import)| {
                        import
                            .alias
                            .as_deref()
                            .or_else(|| import.names.first().map(|name| name.as_str()))
                            .or_else(|| import.spec.split('.').next())
                            == Some("typing")
                    })
                    .collect();
                let typing_proven = matches!(typing_imports.as_slice(), [(index, import)]
                    if import.spec == "typing" && import.names.is_empty() && direct_imports[*index])
                    && !writes
                        .get(&None)
                        .is_some_and(|names| names.contains("typing"))
                    && !external_writes.contains("typing");
                if typing_proven {
                    for (index, candidate) in type_candidates.iter().enumerate() {
                        if *candidate && !repeated[index] {
                            parsed.facts.imports[index].type_only = true;
                        }
                    }
                }
                for (index, identity) in scopes.into_iter().enumerate() {
                    let import = &parsed.facts.imports[index];
                    let name = import
                        .alias
                        .as_deref()
                        .or_else(|| import.names.first().map(|name| name.as_str()))
                        .or_else(|| import.spec.split('.').next());
                    let safe = identity
                        .as_deref()
                        .and_then(crate::domain::facts::python_import_scope)
                        .zip(name)
                        .is_some_and(|((owner, _), name)| {
                            !writes.get(&owner).is_some_and(|names| names.contains(name))
                                && !external_writes.contains(name)
                        });
                    if safe
                        && !repeated[index]
                        && let (Some(identity), Ok(index)) = (identity, u32::try_from(index))
                    {
                        identities.push(SourceIdentity {
                            kind: SourceIdentityKind::PythonImportScope,
                            index,
                            identity,
                        });
                    }
                }
                for (index, identity) in after_tries.into_iter().enumerate() {
                    let import = &parsed.facts.imports[index];
                    let name = import
                        .alias
                        .as_deref()
                        .or_else(|| import.names.first().map(|name| name.as_str()))
                        .or_else(|| import.spec.split('.').next());
                    let safe = identity
                        .as_deref()
                        .and_then(crate::domain::facts::python_import_scope)
                        .zip(name)
                        .is_some_and(|((owner, _), name)| {
                            !writes.get(&owner).is_some_and(|names| names.contains(name))
                                && !external_writes.contains(name)
                        });
                    if safe
                        && !repeated[index]
                        && let (Some(identity), Ok(index)) = (identity, u32::try_from(index))
                    {
                        identities.push(SourceIdentity {
                            kind: SourceIdentityKind::PythonImportAfterTry,
                            index,
                            identity,
                        });
                    }
                }
                for (index, identity) in optional_tries.into_iter().enumerate() {
                    let import = &parsed.facts.imports[index];
                    let name = bound_import_name(import);
                    let safe = identity
                        .as_deref()
                        .and_then(crate::domain::facts::python_import_scope)
                        .zip(name)
                        .is_some_and(|((owner, _), name)| {
                            !writes.get(&owner).is_some_and(|names| names.contains(name))
                                && !external_writes.contains(name)
                        });
                    if safe
                        && !repeated[index]
                        && let (Some(identity), Ok(index)) = (identity, u32::try_from(index))
                    {
                        identities.push(SourceIdentity {
                            kind: SourceIdentityKind::PythonImportOptionalTry,
                            index,
                            identity,
                        });
                    }
                }
                let imported: FastSet<&str> = parsed
                    .facts
                    .imports
                    .iter()
                    .filter_map(|import| {
                        import
                            .alias
                            .as_deref()
                            .or_else(|| import.names.first().map(|name| name.as_str()))
                            .or_else(|| import.spec.split('.').next())
                    })
                    .collect();
                for index in direct_definitions {
                    let name = parsed.facts.symbols[index].name.as_str();
                    if (imported.contains(name)
                        || module_definition_counts
                            .get(name)
                            .is_some_and(|count| *count > 1))
                        && !module_write_counts.contains_key(name)
                        && !external_writes.contains(name)
                        && let Ok(index) = u32::try_from(index)
                    {
                        identities.push(SourceIdentity {
                            kind: SourceIdentityKind::PythonDirectDefinition,
                            index,
                            identity: "direct".to_owned(),
                        });
                    }
                }
                let callable_fallbacks: Vec<_> = callable_fallbacks
                    .into_iter()
                    .filter(|(selected, fallback, _)| {
                        let Some(selected_name) =
                            bound_import_name(&parsed.facts.imports[*selected])
                        else {
                            return false;
                        };
                        let Some(fallback_name) =
                            bound_import_name(&parsed.facts.imports[*fallback])
                        else {
                            return false;
                        };
                        selected_name != fallback_name
                            && module_write_counts.get(selected_name) == Some(&1)
                            && !module_definition_counts.contains_key(selected_name)
                            && !writes
                                .get(&None)
                                .is_some_and(|names| names.contains(fallback_name))
                            && !external_writes.contains(selected_name)
                            && !external_writes.contains(fallback_name)
                            && direct_imports[*fallback]
                            && parsed.facts.imports[*fallback].line
                                < parsed.facts.imports[*selected].line
                            && parsed
                                .facts
                                .imports
                                .iter()
                                .filter(|import| bound_import_name(import) == Some(selected_name))
                                .count()
                                == 1
                            && parsed
                                .facts
                                .imports
                                .iter()
                                .filter(|import| bound_import_name(import) == Some(fallback_name))
                                .count()
                                == 1
                    })
                    .collect();
                for (index, reference) in parsed.facts.refs.iter().enumerate() {
                    let head = reference
                        .qualifier
                        .as_deref()
                        .map_or(reference.name.as_str(), |qualifier| {
                            qualifier.split('.').next().unwrap_or_default()
                        });
                    if !imported.contains(head) {
                        continue;
                    }
                    let callable = reference
                        .scope
                        .and_then(|scope| parsed.facts.symbols.get(scope as usize))
                        .is_some_and(|symbol| symbol.kind().is_callable());
                    let mut owner = reference.scope;
                    let mut shadowed = external_writes.contains(head)
                        || expression_writes.get(head).is_some_and(|ranges| {
                            ranges.iter().any(|range| range.contains(&reference.byte))
                        });
                    let mut local_shadowed = shadowed;
                    loop {
                        let skip_class = callable
                            && owner != reference.scope
                            && owner
                                .and_then(|index| parsed.facts.symbols.get(index as usize))
                                .is_some_and(|symbol| symbol.kind() == SymKind::Class);
                        if !skip_class
                            && writes.get(&owner).is_some_and(|names| names.contains(head))
                        {
                            shadowed = true;
                            if owner.is_some() {
                                local_shadowed = true;
                            }
                        }
                        let Some(current) = owner else { break };
                        owner = parsed
                            .facts
                            .symbols
                            .get(current as usize)
                            .and_then(|symbol| symbol.parent);
                    }
                    if callable
                        && !local_shadowed
                        && reference.kind == crate::domain::facts::RefKind::Call
                        && reference.qualifier.is_none()
                        && let Some((selected, fallback, _)) =
                            callable_fallbacks.iter().find(|(selected, _, end)| {
                                reference.byte >= *end
                                    && bound_import_name(&parsed.facts.imports[*selected])
                                        == Some(head)
                            })
                        && let Ok(index) = u32::try_from(index)
                    {
                        identities.push(SourceIdentity {
                            kind: SourceIdentityKind::PythonCallableImportFallback,
                            index,
                            identity: format!("{selected}:{fallback}"),
                        });
                    }
                    if shadowed && let Ok(index) = u32::try_from(index) {
                        identities.push(SourceIdentity {
                            kind: SourceIdentityKind::PythonShadowedImport,
                            index,
                            identity: "shadowed".to_owned(),
                        });
                    }
                }
                identities.sort_unstable_by_key(|identity| (identity.kind, identity.index));
                identities.dedup_by_key(|identity| (identity.kind, identity.index));
                parsed.facts.source_identities =
                    (!identities.is_empty()).then(|| identities.into_boxed_slice());
                return;
            }
        }
    }
}

fn bound_import_name(import: &Import) -> Option<&str> {
    import
        .alias
        .as_deref()
        .or_else(|| import.names.first().map(|name| name.as_str()))
        .or_else(|| import.spec.split('.').next())
}

fn callable_import_fallback(
    statement: Node,
    source: &[u8],
    parsed: &crate::extract::engine::Parsed,
) -> Option<(usize, usize, u32)> {
    let body = statement.child_by_field_name("body")?;
    let mut cursor = body.walk();
    let body_nodes: Vec<_> = body
        .named_children(&mut cursor)
        .filter(|node| node.kind() != "comment")
        .collect();
    let [import_node] = body_nodes.as_slice() else {
        return None;
    };
    if !matches!(
        import_node.kind(),
        "import_statement" | "import_from_statement"
    ) {
        return None;
    }
    let line = u32::try_from(import_node.start_position().row + 1).ok()?;
    let expanded = expand_import(import_node.utf8_text(source).ok()?, line, false);
    let [import] = expanded.as_slice() else {
        return None;
    };
    let name = bound_import_name(import)?;
    let mut cursor = statement.walk();
    let mut fallback = None;
    for child in statement.named_children(&mut cursor) {
        match child.kind() {
            "except_clause" => {
                if fallback.is_some() {
                    return None;
                }
                let mut tokens = child.walk();
                if child.children(&mut tokens).any(|token| token.kind() == "*") {
                    return None;
                }
                let mut children = child.walk();
                let block = child
                    .named_children(&mut children)
                    .find(|item| item.kind() == "block")?;
                let mut children = block.walk();
                let statements: Vec<_> = block
                    .named_children(&mut children)
                    .filter(|item| item.kind() != "comment")
                    .collect();
                let [expression] = statements.as_slice() else {
                    return None;
                };
                if expression.kind() != "expression_statement"
                    || expression.named_child_count() != 1
                {
                    return None;
                }
                let assignment = expression.named_child(0)?;
                if assignment.kind() != "assignment" {
                    return None;
                }
                let left = assignment.child_by_field_name("left")?;
                let right = assignment.child_by_field_name("right")?;
                if left.kind() != "identifier"
                    || right.kind() != "identifier"
                    || left.utf8_text(source).ok()? != name
                {
                    return None;
                }
                fallback = Some(right.utf8_text(source).ok()?);
            }
            "else_clause" | "finally_clause" => return None,
            _ => {}
        }
    }
    let fallback = fallback?;
    let selected_index = parsed
        .facts
        .imports
        .iter()
        .position(|item| item == import)?;
    let fallback_index = parsed
        .facts
        .imports
        .iter()
        .position(|item| bound_import_name(item) == Some(fallback))?;
    Some((
        selected_index,
        fallback_index,
        u32::try_from(statement.end_byte()).ok()?,
    ))
}

fn is_optional_import_fallback(assignment: Node, source: &[u8]) -> bool {
    if assignment.kind() != "assignment" {
        return false;
    }
    let Some(left) = assignment.child_by_field_name("left") else {
        return false;
    };
    if left.kind() != "identifier" {
        return false;
    }
    let Some(mut value) = assignment.child_by_field_name("right") else {
        return false;
    };
    while value.kind() == "assignment" {
        let Some(next) = value.child_by_field_name("right") else {
            return false;
        };
        value = next;
    }
    if value.kind() != "none" {
        return false;
    }
    let Ok(name) = left.utf8_text(source) else {
        return false;
    };
    let mut outer = assignment;
    while let Some(parent) = outer
        .parent()
        .filter(|parent| parent.kind() == "assignment")
    {
        outer = parent;
    }
    let Some(expression) = outer
        .parent()
        .filter(|parent| parent.kind() == "expression_statement")
    else {
        return false;
    };
    if none_assignment_names(expression, source).is_none_or(|names| !names.contains(&name)) {
        return false;
    }
    let Some(try_statement) = expression
        .parent()
        .and_then(|item| item.parent())
        .and_then(|item| item.parent())
    else {
        return false;
    };
    optional_import_try(try_statement, name, source)
}

fn none_assignment_names<'a>(expression: Node, source: &'a [u8]) -> Option<Vec<&'a str>> {
    if expression.kind() != "expression_statement" || expression.named_child_count() != 1 {
        return None;
    }
    let mut assignment = expression.named_child(0)?;
    let mut names = Vec::new();
    loop {
        if assignment.kind() != "assignment" {
            return None;
        }
        let left = assignment.child_by_field_name("left")?;
        if left.kind() != "identifier" {
            return None;
        }
        names.push(left.utf8_text(source).ok()?);
        let value = assignment.child_by_field_name("right")?;
        if value.kind() == "none" {
            return Some(names);
        }
        assignment = value;
    }
}

fn optional_import_try(statement: Node, name: &str, source: &[u8]) -> bool {
    if statement.kind() != "try_statement" {
        return false;
    }
    let Some(body) = statement.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    let found = body
        .named_children(&mut cursor)
        .filter(|node| matches!(node.kind(), "import_statement" | "import_from_statement"))
        .any(|node| {
            node.utf8_text(source).ok().is_some_and(|text| {
                expand_import(text, 0, false)
                    .iter()
                    .any(|import| bound_import_name(import) == Some(name))
            })
        });
    if !found {
        return false;
    }
    let mut cursor = statement.walk();
    let mut handlers = 0;
    for child in statement.named_children(&mut cursor) {
        match child.kind() {
            "except_clause" => {
                let mut tokens = child.walk();
                if child.children(&mut tokens).any(|item| item.kind() == "*") {
                    return false;
                }
                let mut items = child.walk();
                let Some(block) = child
                    .named_children(&mut items)
                    .find(|item| item.kind() == "block")
                else {
                    return false;
                };
                let mut statements = block.walk();
                let mut bound = false;
                for expression in block
                    .named_children(&mut statements)
                    .filter(|item| item.kind() != "comment")
                {
                    let Some(names) = none_assignment_names(expression, source) else {
                        return false;
                    };
                    bound |= names.contains(&name);
                }
                if !bound {
                    return false;
                }
                handlers += 1;
            }
            "else_clause" | "finally_clause" => return false,
            _ => {}
        }
    }
    handlers > 0
}

fn is_type_checking_import(node: Node, source: &[u8]) -> bool {
    let Some(block) = node.parent() else {
        return false;
    };
    let Some(statement) = block.parent() else {
        return false;
    };
    if block.kind() != "block"
        || statement.kind() != "if_statement"
        || statement
            .parent()
            .is_none_or(|parent| parent.kind() != "module")
        || statement.child_by_field_name("consequence") != Some(block)
    {
        return false;
    }
    let Some(condition) = statement.child_by_field_name("condition") else {
        return false;
    };
    condition.kind() == "attribute"
        && condition
            .child_by_field_name("object")
            .and_then(|item| item.utf8_text(source).ok())
            == Some("typing")
        && condition
            .child_by_field_name("attribute")
            .and_then(|item| item.utf8_text(source).ok())
            == Some("TYPE_CHECKING")
}

fn import_scope_identity(node: Node, parsed: &crate::extract::engine::Parsed) -> Option<String> {
    let mut parent = node.parent()?;
    while parent.kind() != "block" {
        parent = parent.parent()?;
    }
    let start = u32::try_from(node.end_byte()).ok()?;
    let end = u32::try_from(parent.end_byte()).ok()?;
    if start >= end {
        return None;
    }
    let encoded_owner = match import_owner(node.start_byte(), parsed) {
        Some(owner) => owner.checked_add(1)?,
        None => 0,
    };
    Some(format!("{encoded_owner}:{start}:{end}"))
}

fn import_after_try_identity(
    node: Node,
    parsed: &crate::extract::engine::Parsed,
) -> Option<String> {
    let body = node.parent()?;
    let statement = body.parent()?;
    if body.kind() != "block"
        || statement.kind() != "try_statement"
        || statement.child_by_field_name("body") != Some(body)
    {
        return None;
    }
    let mut cursor = statement.walk();
    let mut handlers = 0;
    for child in statement.named_children(&mut cursor) {
        match child.kind() {
            "except_clause" => {
                let mut tokens = child.walk();
                if child.children(&mut tokens).any(|item| item.kind() == "*") {
                    return None;
                }
                let mut items = child.walk();
                let handler = child
                    .named_children(&mut items)
                    .find(|item| item.kind() == "block")?;
                let mut statements = handler.walk();
                let last = handler
                    .named_children(&mut statements)
                    .filter(|item| item.kind() != "comment")
                    .last()?;
                if !matches!(last.kind(), "return_statement" | "raise_statement") {
                    return None;
                }
                handlers += 1;
            }
            "else_clause" | "finally_clause" => return None,
            _ => {}
        }
    }
    if handlers == 0 {
        return None;
    }
    let outer = statement.parent()?;
    if !matches!(outer.kind(), "block" | "module") {
        return None;
    }
    let start = u32::try_from(statement.end_byte()).ok()?;
    let end = u32::try_from(outer.end_byte()).ok()?;
    if start >= end {
        return None;
    }
    let owner = match import_owner(node.start_byte(), parsed) {
        Some(owner) => owner.checked_add(1)?,
        None => 0,
    };
    Some(format!("{owner}:{start}:{end}"))
}

fn import_optional_try_identity(
    node: Node,
    import: &Import,
    source: &[u8],
    parsed: &crate::extract::engine::Parsed,
) -> Option<String> {
    let body = node.parent()?;
    let statement = body.parent()?;
    if body.kind() != "block"
        || statement.child_by_field_name("body") != Some(body)
        || !(optional_import_try(statement, bound_import_name(import)?, source)
            || function_local_import_try(statement, node, parsed))
    {
        return None;
    }
    let outer = statement.parent()?;
    if !matches!(outer.kind(), "block" | "module") {
        return None;
    }
    let start = u32::try_from(statement.end_byte()).ok()?;
    let end = u32::try_from(outer.end_byte()).ok()?;
    if start >= end {
        return None;
    }
    let owner = match import_owner(node.start_byte(), parsed) {
        Some(owner) => owner.checked_add(1)?,
        None => 0,
    };
    Some(format!("{owner}:{start}:{end}"))
}

fn function_local_import_try(
    statement: Node,
    node: Node,
    parsed: &crate::extract::engine::Parsed,
) -> bool {
    if statement.kind() != "try_statement"
        || !import_owner(node.start_byte(), parsed)
            .and_then(|owner| parsed.facts.symbols.get(owner as usize))
            .is_some_and(|symbol| symbol.kind().is_callable())
    {
        return false;
    }
    let mut handlers = 0;
    let mut cursor = statement.walk();
    for child in statement.named_children(&mut cursor) {
        match child.kind() {
            "except_clause" => {
                let mut tokens = child.walk();
                if child.children(&mut tokens).any(|item| item.kind() == "*") {
                    return false;
                }
                handlers += 1;
            }
            "else_clause" | "finally_clause" => return false,
            _ => {}
        }
    }
    handlers > 0
}

fn import_owner(position: usize, parsed: &crate::extract::engine::Parsed) -> Option<u32> {
    let after = parsed
        .spans
        .partition_point(|&(start, _)| start <= position);
    let mut owner = after.checked_sub(1);
    while let Some(index) = owner {
        if position < parsed.spans[index].1
            && matches!(
                parsed.facts.symbols.get(index)?.kind(),
                SymKind::Fn | SymKind::Method | SymKind::Class
            )
        {
            return u32::try_from(index).ok();
        }
        owner = parsed
            .facts
            .symbols
            .get(index)?
            .parent
            .map(|parent| parent as usize);
    }
    None
}

fn collect_written_names<'a>(node: Node, source: &'a [u8], add: &mut impl FnMut(&'a str)) {
    match node.kind() {
        "identifier" => {
            if let Ok(name) = node.utf8_text(source) {
                add(name);
            }
        }
        "parameters" | "lambda_parameters" => {
            let mut cursor = node.walk();
            for parameter in node.named_children(&mut cursor) {
                let target = match parameter.kind() {
                    "typed_parameter" => parameter.named_child(0),
                    "default_parameter" | "typed_default_parameter" => {
                        parameter.child_by_field_name("name")
                    }
                    _ => Some(parameter),
                };
                if let Some(target) = target {
                    collect_written_names(target, source, add);
                }
            }
        }
        "attribute" | "subscript" => {}
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_written_names(child, source, add);
            }
        }
    }
}

/// Callee whose result is assigned by `with`, rather than the manager expression itself.
pub(in crate::extract) fn context_manager_callee(
    item: Node,
    callee: Node,
    source: &[u8],
) -> Option<String> {
    let statement = item.parent()?.parent()?;
    if item.kind() != "with_item" || statement.kind() != "with_statement" {
        return None;
    }
    let mut cursor = statement.walk();
    let asynchronous = statement
        .children(&mut cursor)
        .any(|node| node.kind() == "async");
    let method = if asynchronous {
        "__aenter__"
    } else {
        "__enter__"
    };
    let call = callee
        .parent()
        .is_some_and(|parent| parent.kind() == "call");
    let callee = callee.utf8_text(source).ok()?;
    Some(format!("{callee}{}.{method}", if call { "()" } else { "" }))
}

/// Keep protocol return annotations intact so unions cannot become their first branch.
pub(in crate::extract) fn context_manager_result(node: Node, source: &[u8]) -> Option<String> {
    let function = node.parent()?;
    if function.kind() != "function_definition"
        || !matches!(
            function
                .child_by_field_name("name")?
                .utf8_text(source)
                .ok()?,
            "__enter__" | "__aenter__"
        )
    {
        return None;
    }
    if returned_receiver(function, source).is_some() {
        return Some("Self".to_owned());
    }
    let text = node.utf8_text(source).ok()?;
    let unquoted = text
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .or_else(|| {
            text.strip_prefix('\'')
                .and_then(|text| text.strip_suffix('\''))
        })
        .filter(|text| !text.contains('\\'))
        .unwrap_or(text);
    Some(crate::domain::facts::normalize_space(unquoted))
}

/// A successful exit returns the original receiver: no alternative return or local rebinding.
fn returned_receiver<'source>(function: Node, source: &'source [u8]) -> Option<&'source str> {
    if function.has_error() || function.parent()?.parent()?.kind() != "class_definition" {
        return None;
    }
    let parameter = function.child_by_field_name("parameters")?.named_child(0)?;
    let parameter = match parameter.kind() {
        "identifier" => parameter,
        "typed_parameter" => parameter.named_child(0)?,
        _ => return None,
    };
    if parameter.kind() != "identifier" {
        return None;
    }
    let receiver = parameter.utf8_text(source).ok()?;
    let body = function.child_by_field_name("body")?;
    let mut cursor = body.walk();
    if body
        .named_children(&mut cursor)
        .filter(|node| node.kind() != "comment")
        .last()?
        .kind()
        != "return_statement"
    {
        return None;
    }
    let mut cursor = body.walk();
    loop {
        let node = cursor.node();
        match node.kind() {
            "return_statement" => {
                let value = node.named_child(0)?;
                if node.named_child_count() != 1
                    || value.kind() != "identifier"
                    || value.utf8_text(source).ok()? != receiver
                {
                    return None;
                }
            }
            "function_definition"
            | "class_definition"
            | "lambda"
            | "yield"
            | "import_statement"
            | "import_from_statement"
            | "global_statement"
            | "nonlocal_statement"
            | "delete_statement"
            | "match_statement"
            | "except_clause" => return None,
            _ => {}
        }
        let target = match node.kind() {
            "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => {
                node.child_by_field_name("left")
            }
            "named_expression" => node.child_by_field_name("name"),
            "as_pattern_target" => Some(node),
            _ => None,
        };
        if target.is_some_and(|target| target_changes_receiver(target, receiver, source)) {
            return None;
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return Some(receiver);
            }
        }
    }
}

fn target_changes_receiver(node: Node, name: &str, source: &[u8]) -> bool {
    match node.kind() {
        "identifier" => node.utf8_text(source).ok() == Some(name),
        "attribute" => {
            node.child_by_field_name("attribute")
                .and_then(|attribute| attribute.utf8_text(source).ok())
                == Some("__class__")
        }
        "subscript" => false,
        _ => node
            .named_children(&mut node.walk())
            .any(|child| target_changes_receiver(child, name, source)),
    }
}

/// Evidence for conservative structural protocol matching; decorated or dynamic class bodies are
/// intentionally excluded because their runtime members cannot be established from method names.
pub(in crate::extract) fn structural_identity(node: Node, source: &[u8]) -> Option<String> {
    if node.has_error() || node.is_missing() {
        return None;
    }
    if node.kind() == "class_definition" && node.parent()?.kind() == "module" {
        let body = node.child_by_field_name("body")?;
        let mut cursor = body.walk();
        let simple = body.named_children(&mut cursor).all(|child| {
            matches!(
                child.kind(),
                "function_definition" | "comment" | "pass_statement"
            ) || (child.kind() == "expression_statement"
                && child.named_child_count() == 1
                && child
                    .named_child(0)
                    .is_some_and(|item| item.kind() == "string"))
        });
        if !simple {
            return None;
        }
        if node.child_by_field_name("superclasses").is_none() {
            return Some("python:plain-class:".to_owned());
        }
        let module = node.parent()?;
        let mut cursor = module.walk();
        let lines = module
            .named_children(&mut cursor)
            .take_while(|child| child.start_byte() < node.start_byte())
            .filter(|child| child.kind() == "import_from_statement")
            .filter_map(|child| {
                let name = child
                    .child_by_field_name("module_name")?
                    .utf8_text(source)
                    .ok()?;
                matches!(name, "typing" | "typing_extensions")
                    .then(|| (child.start_position().row + 1).to_string())
            })
            .collect::<Vec<_>>()
            .join(",");
        return Some(format!("python:plain-class:{lines}"));
    }
    if node.kind() != "function_definition" {
        return None;
    }
    let block = node.parent()?;
    let class = block.parent()?;
    if block.kind() != "block"
        || class.kind() != "class_definition"
        || class.parent()?.kind() != "module"
    {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    let header = source.get(node.start_byte()..body.start_byte())?;
    if header.len() > 1024 {
        return None;
    }
    let identity = crate::extract::engine::signature_identity(node, source)?;
    Some(format!("python:method:{identity}"))
}

/// Detects a Python contract method whose complete executable body raises `NotImplementedError`.
pub(in crate::extract) fn is_abstract_stub(node: Node, source: &[u8]) -> bool {
    if node.kind() != "function_definition" || node.has_error() || node.is_missing() {
        return false;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    let mut found_raise = false;
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "comment" => {}
            "expression_statement" if !found_raise && is_docstring(child) => {}
            "raise_statement" if !found_raise && raises_not_implemented(child, source) => {
                found_raise = true;
            }
            _ => return false,
        }
    }
    found_raise
}

fn is_docstring(statement: Node) -> bool {
    statement.kind() == "expression_statement"
        && statement.named_child_count() == 1
        && statement
            .named_child(0)
            .is_some_and(|child| matches!(child.kind(), "string" | "concatenated_string"))
}

fn raises_not_implemented(statement: Node, source: &[u8]) -> bool {
    let mut cursor = statement.walk();
    let mut expressions = statement.named_children(&mut cursor);
    let Some(exception) = expressions.next() else {
        return false;
    };
    if expressions.next().is_some() {
        return false;
    }
    let exception_name = match exception.kind() {
        "identifier" => Some(exception),
        "call" => exception.child_by_field_name("function"),
        _ => None,
    };
    exception_name.is_some_and(|name| {
        name.kind() == "identifier"
            && name
                .utf8_text(source)
                .is_ok_and(|text| text == "NotImplementedError")
    })
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    let decorated = if matches!(definition.kind, SymKind::Fn | SymKind::Method)
        && definition.decorations.contains('@')
    {
        crate::domain::facts::FLAG_PYTHON_DECORATED
    } else {
        0
    };
    // A registering decorator (`@app.route`, `@pytest.fixture`, `@click.command`) means the
    // framework calls the function: an entrypoint, never dead.
    let registered = definition
        .decorations
        .split('@')
        .map(str::trim)
        .filter(|decorator| !decorator.is_empty())
        .any(|decorator| {
            let name = decorator.split(['(', ' ']).next().unwrap_or_default();
            !matches!(
                name,
                "property" | "staticmethod" | "classmethod" | "abstractmethod" | "override"
            ) && !name.ends_with(".setter")
                && !name.ends_with(".getter")
        });
    let entry = if registered {
        crate::domain::facts::FLAG_ENTRY
    } else {
        0
    };
    let test_file = crate::domain::role::is_test_path(FileType::Python, context.path);
    let test_name = (matches!(definition.kind, SymKind::Fn | SymKind::Method)
        && definition.name.starts_with("test"))
        || (definition.kind == SymKind::Class && definition.name.starts_with("Test"));
    if test_name && (test_file || matches!(definition.parent, Some((SymKind::Class, _)))) {
        FLAG_TEST | entry | decorated
    } else {
        entry | decorated
    }
}

/// `import a.b as c, d` and `from .x import y as z, w` into one import per name: `spec` is the
/// module path (leading dots kept for relative imports), `names` the imported name (empty for
/// whole-module imports), `alias` the `as` name.
fn expand_import(statement: &str, line: u32, _reexport: bool) -> Vec<Import> {
    let uncommented = statement
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ");
    let text: String = uncommented
        .replace(['(', ')', '\\'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let split_alias = |part: &str| -> (String, Option<String>) {
        match part.trim().split_once(" as ") {
            Some((name, alias)) => (name.trim().to_owned(), Some(alias.trim().to_owned())),
            None => (part.trim().to_owned(), None),
        }
    };
    if let Some(rest) = text.strip_prefix("from ") {
        let Some((module, names)) = rest.split_once(" import ") else {
            return Vec::new();
        };
        return names
            .split(',')
            .filter(|part| !part.trim().is_empty())
            .map(|part| {
                let (name, alias) = split_alias(part);
                Import {
                    spec: module.trim().into(),
                    alias: alias.map(Name::from),
                    names: vec![name.into()],
                    line,
                    reexport: false,
                    type_only: false,
                }
            })
            .collect();
    }
    let Some(rest) = text.strip_prefix("import ") else {
        return Vec::new();
    };
    rest.split(',')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let (module, alias) = split_alias(part);
            Import {
                spec: module.into(),
                alias: alias.map(Name::from),
                names: Vec::new(),
                line,
                reexport: false,
                type_only: false,
            }
        })
        .collect()
}

/// Builtin functions and types.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "len"
            | "range"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "isinstance"
            | "issubclass"
            | "super"
            | "open"
            | "enumerate"
            | "zip"
            | "map"
            | "filter"
            | "sorted"
            | "reversed"
            | "min"
            | "max"
            | "sum"
            | "any"
            | "all"
            | "getattr"
            | "setattr"
            | "hasattr"
            | "type"
            | "object"
            | "Exception"
            | "ValueError"
            | "TypeError"
            | "KeyError"
            | "RuntimeError"
            | "repr"
            | "iter"
            | "next"
            | "abs"
            | "round"
            | "format"
            | "id"
            | "hash"
            | "bytes"
            | "callable"
            | "vars"
            | "dir"
            | "staticmethod"
            | "classmethod"
            | "property"
            | "NotImplementedError"
            | "None"
            | "Optional"
            | "List"
            | "Dict"
            | "Any"
            | "Union"
            | "Tuple"
            | "Set"
            | "Callable"
            | "Iterable"
            | "Iterator"
            | "Sequence"
            | "Mapping"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_import_identity_excludes_conditional_and_local_imports()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::domain::corpus::FileRecord;
        use crate::domain::role::Role;

        let source = "from a import x as direct\nflag = True; from b import y as semicolon\nif flag:\n    from c import z as conditional\n    conditional()\ndef inner():\n    from d import t as local\n    local()\ndef shadowed():\n    from e import t as changed\n    changed = 1\n    changed()\n";
        let record = FileRecord {
            path: "module.py".to_owned(),
            size: source.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: Role::Src,
            file_type: Some(FileType::Python),
        };
        let facts = crate::extract::extract(&record, source.as_bytes());
        for (index, import) in facts.imports.iter().enumerate() {
            let expected = match import.alias.as_deref() {
                Some("direct" | "semicolon") => Some("direct"),
                Some("conditional" | "local" | "changed") => None,
                other => return Err(format!("unexpected import {other:?}").into()),
            };
            assert_eq!(
                facts.source_identity(SourceIdentityKind::PythonDirectImport, index as u32),
                expected
            );
            let scope = facts
                .source_identity(SourceIdentityKind::PythonImportScope, index as u32)
                .and_then(crate::domain::facts::python_import_scope);
            match import.alias.as_deref() {
                Some("conditional" | "local") => {
                    let (owner, range) = scope.ok_or("missing scoped import evidence")?;
                    assert_eq!(owner.is_some(), import.alias.as_deref() == Some("local"));
                    let call = source
                        .find(&format!(
                            "{}()",
                            import.alias.as_deref().unwrap_or_default()
                        ))
                        .ok_or("missing call")? as u32;
                    assert!(range.contains(&call));
                }
                _ => assert!(scope.is_none()),
            }
        }
        assert_eq!(facts.imports.len(), 5);
        Ok(())
    }

    #[test]
    fn context_bindings_use_the_protocol_result_instead_of_the_manager() -> std::io::Result<()> {
        use crate::domain::graph::{Confidence, EdgeKind};
        use crate::infra::testdir::TestDir;

        let root = TestDir::new("python-context-values")?;
        root.write("contexts.py", concat!(
            "from typing import Self, TypeVar\n",
            "R = TypeVar('R', bound='GenericManager')\nS = TypeVar('S', bound='GenericAsync')\n",
            "W = TypeVar('W', bound='ReboundManager')\nM = TypeVar('M', bound='MutatingManager')\n",
            "class Resource:\n    def consume(self): return 1\n",
            "    @property\n    def closed(self) -> bool: return False\n",
            "    @property\n    def auth(self) -> str: return self._auth\n",
            "    @auth.setter\n    def auth(self, value: str): self._auth = value\n",
            "    def unknown_only(self): return 1\n    def union_only(self): return 1\n",
            "    def rebound_only(self): return 1\n    def mutated_only(self): return 1\n",
            "class Manager:\n",
            "    def __enter__(self) -> Resource: return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 2\n",
            "class Own:\n",
            "    def __enter__(self) -> Self: return self\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 3\n",
            "    def union_only(self): return 3\n",
            "class AsyncManager:\n",
            "    async def __aenter__(self) -> Resource: return Resource()\n",
            "    async def __aexit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 4\n",
            "class UnknownManager:\n",
            "    def __enter__(self): return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 5\n",
            "    def unknown_only(self): return 5\n",
            "class QuotedManager:\n",
            "    def __enter__(self) -> 'Resource': return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 7\n",
            "class AmbiguousManager:\n",
            "    def __enter__(self) -> Resource | Own: return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 6\n",
            "    def union_only(self): return 6\n",
            "class GenericManager:\n",
            "    def __enter__(this: R) -> R:\n",
            "        this.state = 1\n        for piece in (1,): this.state += piece\n",
            "        if this.state < 0: raise ValueError('state')\n        return this\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 8\n",
            "class Derived(GenericManager):\n    def consume(self): return 9\n",
            "class GenericAsync:\n",
            "    async def __aenter__(self: S) -> S:\n        self.state = 1\n        return self\n",
            "    async def __aexit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 10\n",
            "class NominalManager(Resource):\n",
            "    def __enter__(self: Resource) -> Resource: return Resource()\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 11\n",
            "class ReboundManager:\n",
            "    def __enter__(self: W) -> W:\n        self = Resource()\n        return self\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def rebound_only(self): return 12\n",
            "class MutatingManager:\n",
            "    def __enter__(self: M) -> M:\n        self.__class__ = Resource\n        return self\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def mutated_only(self): return 13\n",
            "class MixedManager:\n",
            "    def __init__(self): self.alternative = True\n",
            "    def __enter__(self) -> 'Resource | MixedManager':\n",
            "        if self.alternative: return Resource()\n        return self\n",
            "    def __exit__(self, kind, value, trace): return False\n",
            "    def consume(self): return 14\n",
            "def acquire() -> Manager: return Manager()\n",
            "def direct():\n    with Manager() as value: return value.consume()\n",
            "def factory():\n    with acquire() as value: return value.consume()\n",
            "def existing(manager: Manager):\n    with manager as value: return value.consume()\n",
            "def own():\n    with Own() as value: return value.consume()\n",
            "def quoted():\n    with QuotedManager() as value: return value.consume()\n",
            "async def asynchronous():\n    async with AsyncManager() as value: return value.consume()\n",
            "def unknown():\n    with UnknownManager() as value: return value.unknown_only()\n",
            "def ambiguous():\n    with AmbiguousManager() as value: return value.union_only()\n",
            "def generic():\n    with GenericManager() as value: return value.consume()\n",
            "def derived():\n    with Derived() as value: return value.consume()\n",
            "async def generic_async():\n    async with GenericAsync() as value: return value.consume()\n",
            "def nominal():\n    with NominalManager() as value: return value.consume()\n",
            "def rebound():\n    with ReboundManager() as value: return value.rebound_only()\n",
            "def mutated():\n    with MutatingManager() as value: return value.mutated_only()\n",
            "def mixed():\n    with MixedManager() as value: return value.consume()\n",
            "def property_inside():\n    with Manager() as value: return value.closed\n",
            "def property_after():\n    with Manager() as value: value.consume()\n    return value.closed\n",
            "def ambiguous_property():\n    with Manager() as value:\n        value.auth = 'updated'\n        return value.auth\n",
        ))?;
        root.write("consumer.py", "import contexts as api\ndef qualified():\n    with api.Derived() as value: return value.consume()\n")?;
        let graph = crate::resolve::tests::graph_of(root.path())?;
        let mut calls: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls)
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                if !matches!(
                    target.name.as_str(),
                    "consume" | "rebound_only" | "mutated_only"
                ) {
                    return None;
                }
                let owner = graph.symbols.get(target.parent as usize)?;
                Some((source.name.as_str(), owner.name.as_str(), edge.confidence))
            })
            .collect();
        calls.sort_unstable();
        for name in [
            "unknown_only",
            "union_only",
            "rebound_only",
            "mutated_only",
            "auth",
        ] {
            assert!(
                graph
                    .ambiguous_names
                    .iter()
                    .any(|candidate| candidate == name),
                "{name}: unresolved context members must remain ambiguous"
            );
        }
        assert_eq!(
            calls,
            [
                ("asynchronous", "Resource", Confidence::Inferred),
                ("derived", "Derived", Confidence::Inferred),
                ("direct", "Resource", Confidence::Inferred),
                ("existing", "Resource", Confidence::Inferred),
                ("factory", "Resource", Confidence::Inferred),
                ("generic", "GenericManager", Confidence::Inferred),
                ("generic_async", "GenericAsync", Confidence::Inferred),
                ("nominal", "Resource", Confidence::Inferred),
                ("own", "Own", Confidence::Inferred),
                ("property_after", "Resource", Confidence::Inferred),
                ("qualified", "Derived", Confidence::Inferred),
                ("quoted", "Resource", Confidence::Inferred),
            ]
        );
        let mut properties: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Uses)
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                (target.name == "closed").then_some((source.name.as_str(), edge.confidence))
            })
            .collect();
        properties.sort_unstable();
        assert_eq!(
            properties,
            [
                ("property_after", Confidence::Inferred),
                ("property_inside", Confidence::Inferred),
            ]
        );
        Ok(())
    }

    fn rows(statement: &str) -> Vec<(String, String, Option<String>)> {
        expand_import(statement, 1, false)
            .into_iter()
            .map(|import| {
                (
                    import.spec.to_string(),
                    import
                        .names
                        .iter()
                        .map(|name| name.as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                    import.alias.map(|alias| alias.to_string()),
                )
            })
            .collect()
    }

    #[test]
    fn import_statements_expand_per_name() {
        assert_eq!(
            rows("import os, a.b as c"),
            [
                ("os".to_owned(), String::new(), None),
                ("a.b".to_owned(), String::new(), Some("c".to_owned()))
            ]
        );
        assert_eq!(
            rows("from ..pkg.mod import (x as y,\n    z)"),
            [
                ("..pkg.mod".to_owned(), "x".to_owned(), Some("y".to_owned())),
                ("..pkg.mod".to_owned(), "z".to_owned(), None),
            ]
        );
        assert_eq!(
            rows(
                "from graphify.hooks import (  # comment\n    _HOOK_SCRIPT, # another\n    other as alias,\n)"
            ),
            [
                ("graphify.hooks".to_owned(), "_HOOK_SCRIPT".to_owned(), None),
                (
                    "graphify.hooks".to_owned(),
                    "other".to_owned(),
                    Some("alias".to_owned())
                ),
            ]
        );
        assert_eq!(
            rows("from . import util"),
            [(".".to_owned(), "util".to_owned(), None)]
        );
        assert_eq!(
            rows("from x import *"),
            [("x".to_owned(), "*".to_owned(), None)]
        );
    }
}
