//! NAME forms of `show`, `impact` and `path` (`docs/spec.md` section 2): `qualified.name`,
//! `path:line`, `path::name`, bare name.

use super::{Filter, Index, Node};
use crate::app::error::AppError;
use crate::domain::graph::has_qualified_suffix;

/// Resolves NAME to nodes: one when unique, several for an ambiguous name, none otherwise.
pub fn lookup(index: &Index, filter: &Filter, name: &str) -> Vec<u32> {
    let name = name.trim();
    if let Some((path, line)) = name.rsplit_once(':')
        && !path.ends_with(':')
        && let Ok(line) = line.parse::<u32>()
    {
        return at_line(index, path, line);
    }
    if let Some((path, symbol)) = name.split_once("::")
        && (path.contains('/') || path.contains('.'))
        && file_index(index, path).is_some()
    {
        return in_file(index, filter, path, symbol);
    }
    if let Some(file) = file_index(index, name) {
        return vec![index.symbol_count() + file];
    }
    let normalized = name.replace("::", ".");
    let symbols = index.derived.symbols();
    let matches = |predicate: &dyn Fn(&str, &str) -> bool| -> Vec<u32> {
        (0..symbols.len() as u32)
            .filter(|&node| {
                let symbol = &symbols[node as usize];
                let qualified = index.qualified(symbol).replace("::", ".");
                predicate(index.string(symbol.name), &qualified) && filter.symbol(index, symbol)
            })
            .collect()
    };
    let qualified = normalized.contains('.');
    let exact = matches(&|_, qualified_name| qualified_name == normalized);
    if !exact.is_empty() {
        return prefer_definitions(index, exact);
    }
    let found = if qualified {
        matches(&|_, qualified_name| has_qualified_suffix(qualified_name, &normalized))
    } else {
        matches(&|symbol_name, _| symbol_name == normalized)
    };
    prefer_definitions(index, found)
}

/// Among same-named matches, non-test symbols win.
fn prefer_definitions(index: &Index, found: Vec<u32>) -> Vec<u32> {
    if found.len() < 2 {
        return found;
    }
    let primary: Vec<u32> = found
        .iter()
        .copied()
        .filter(|&node| matches!(index.node(node), Node::Symbol(symbol) if symbol.flags & crate::domain::facts::FLAG_TEST == 0))
        .collect();
    if primary.is_empty() { found } else { primary }
}

fn file_index(index: &Index, path: &str) -> Option<u32> {
    let path = path.trim_start_matches("./");
    index
        .derived
        .files()
        .iter()
        .position(|file| index.string(file.path) == path)
        .map(|position| position as u32)
}

/// Innermost symbol of the file whose range contains the line; the file itself when none does.
fn at_line(index: &Index, path: &str, line: u32) -> Vec<u32> {
    let Some(file) = file_index(index, path) else {
        return Vec::new();
    };
    let record = &index.derived.files()[file as usize];
    let first = record.first_symbol;
    let innermost = (first..first + record.symbol_count)
        .filter(|&node| {
            let symbol = &index.derived.symbols()[node as usize];
            symbol.line <= line && line <= symbol.end_line
        })
        .min_by_key(|&node| {
            let symbol = &index.derived.symbols()[node as usize];
            symbol.end_line - symbol.line
        });
    vec![innermost.unwrap_or(index.symbol_count() + file)]
}

fn in_file(index: &Index, filter: &Filter, path: &str, name: &str) -> Vec<u32> {
    let Some(file) = file_index(index, path) else {
        return Vec::new();
    };
    let record = &index.derived.files()[file as usize];
    let normalized = name.replace("::", ".");
    (record.first_symbol..record.first_symbol + record.symbol_count)
        .filter(|&node| {
            let symbol = &index.derived.symbols()[node as usize];
            let qualified = index.qualified(symbol).replace("::", ".");
            (index.string(symbol.name) == normalized
                || has_qualified_suffix(&qualified, &normalized))
                && filter.symbol(index, symbol)
        })
        .collect()
}

/// Exactly one node for NAME; ambiguity lists the candidates (exit 3), no match is exit 4.
pub fn resolve_one(
    index: &Index,
    filter: &Filter,
    name: &str,
    json: bool,
) -> Result<u32, AppError> {
    let found = lookup(index, filter, name);
    match found.as_slice() {
        [only] => Ok(*only),
        [] => Err(AppError::NoMatch(format!(
            "no symbol or file matches '{name}'"
        ))),
        many => {
            let hits: Vec<super::find::Hit> = many
                .iter()
                .map(|&node| super::find::Hit { node, score: 0.0 })
                .collect();
            Err(AppError::Ambiguous(super::find::render(
                index, &hits, json, 600,
            )))
        }
    }
}
