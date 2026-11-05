//! Source-proven activation and lexical limits of receiver bindings.

use std::ops::Range;

use tree_sitter::Node;

/// Byte visibility for the Rust and ECMAScript binding forms emitted by their queries.
pub(super) fn visibility(path: &str, binding: Node<'_>) -> Option<Range<u32>> {
    let rust = path.ends_with(".rs");
    let javascript = javascript(path);
    if !rust && !javascript {
        return None;
    }
    if rust && binding.kind() == "identifier" {
        return pattern_visibility(binding);
    }
    if javascript && binding.kind() == "for_in_statement" {
        let start = u32::try_from(binding.child_by_field_name("body")?.start_byte()).ok()?;
        let end = u32::try_from(binding.end_byte()).ok()?;
        return (start < end).then_some(start..end);
    }
    let local = matches!(binding.kind(), "let_declaration" | "variable_declarator");
    let function_scoped = binding
        .parent()
        .is_some_and(|parent| parent.kind() == "variable_declaration");
    let mut parent = binding.parent();
    while let Some(node) = parent {
        let function = matches!(
            node.kind(),
            "function_item"
                | "closure_expression"
                | "function_declaration"
                | "generator_function_declaration"
                | "function_expression"
                | "generator_function"
                | "arrow_function"
                | "method_definition"
        );
        let block = local
            && !function_scoped
            && matches!(
                node.kind(),
                "block" | "statement_block" | "for_statement" | "for_in_statement"
            );
        if function || block || node.kind() == "program" {
            let start = u32::try_from(binding.end_byte()).ok()?;
            let end = u32::try_from(node.end_byte()).ok()?;
            return (start < end).then_some(start..end);
        }
        parent = node.parent();
    }
    None
}

/// A pattern name lives from the end of its pattern's value to the end of the arm, consequence,
/// loop body or, for `let ... else`, of the enclosing block.
fn pattern_visibility(name: Node<'_>) -> Option<Range<u32>> {
    let mut node = name.parent();
    let mut scrutinee_end = None;
    while let Some(current) = node {
        let limit = match current.kind() {
            "match_arm" => {
                let start = name.end_byte();
                return bytes(start, current.end_byte());
            }
            "let_condition" => {
                scrutinee_end.get_or_insert(current.end_byte());
                None
            }
            "let_declaration" => {
                scrutinee_end.get_or_insert(current.end_byte());
                None
            }
            "if_expression" => current.child_by_field_name("consequence"),
            "while_expression" => current.child_by_field_name("body"),
            "block" if scrutinee_end.is_some() => Some(current),
            _ => None,
        };
        if let (Some(limit), Some(start)) = (limit, scrutinee_end) {
            return bytes(start, limit.end_byte());
        }
        node = current.parent();
    }
    None
}

fn bytes(start: usize, end: usize) -> Option<Range<u32>> {
    let (start, end) = (u32::try_from(start).ok()?, u32::try_from(end).ok()?);
    (start < end).then_some(start..end)
}

fn javascript(path: &str) -> bool {
    [
        ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".vue", ".svelte", ".astro",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}

/// Element type of a declared ECMAScript array or standard iterable annotation. Nested containers
/// stay unknown: the normalized name could not tell the element from its own element.
pub(super) fn declared_element<'tree>(
    path: &str,
    annotation: Node<'tree>,
    source: &[u8],
) -> Option<Node<'tree>> {
    if !javascript(path) {
        return None;
    }
    let element = container_element(annotation, source)?;
    container_element(element, source)
        .is_none()
        .then_some(element)
}

fn container_element<'tree>(node: Node<'tree>, source: &[u8]) -> Option<Node<'tree>> {
    match node.kind() {
        "readonly_type" => container_element(node.named_child(0)?, source),
        "array_type" => node.named_child(0),
        "generic_type" => {
            let name = node.child_by_field_name("name")?.utf8_text(source).ok()?;
            matches!(
                name,
                "Array" | "ReadonlyArray" | "Set" | "ReadonlySet" | "Iterable" | "IterableIterator"
            )
            .then(|| node.child_by_field_name("type_arguments")?.named_child(0))
            .flatten()
        }
        _ => None,
    }
}
