//! `path`: shortest dependency path between two nodes (`docs/spec.md` section 7.6).

use std::collections::{HashMap, VecDeque};

use super::output::{Value, Writer};
use super::{Filter, Index};
use crate::domain::graph::EdgeKind;

const KINDS: [EdgeKind; 5] = [
    EdgeKind::Calls,
    EdgeKind::Uses,
    EdgeKind::Inherits,
    EdgeKind::Implements,
    EdgeKind::Imports,
];

/// Hops from `from` to `to`: node and whether the hop was taken against edge direction.
pub fn shortest(
    index: &Index,
    filter: &Filter,
    from: u32,
    to: u32,
    undirected: bool,
) -> Option<Vec<(u32, bool)>> {
    let mut previous: HashMap<u32, (u32, bool)> = HashMap::new();
    let mut queue = VecDeque::from([from]);
    previous.insert(from, (from, false));
    while let Some(node) = queue.pop_front() {
        if node == to {
            let mut hops = vec![(to, false)];
            let mut current = to;
            while current != from {
                let (before, reversed) = previous[&current];
                if let Some(last) = hops.last_mut() {
                    last.1 = reversed;
                }
                hops.push((before, false));
                current = before;
            }
            hops.reverse();
            return Some(hops);
        }
        let forward = index
            .derived
            .outgoing(node)
            .iter()
            .map(|edge| (edge, false));
        let backward = index
            .derived
            .incoming(node)
            .iter()
            .filter(|_| undirected)
            .map(|edge| (edge, true));
        for (edge, reversed) in forward.chain(backward) {
            if !KINDS.contains(&edge.kind())
                || !filter.edge(edge)
                || previous.contains_key(&edge.node)
            {
                continue;
            }
            previous.insert(edge.node, (node, reversed));
            queue.push_back(edge.node);
        }
    }
    None
}

pub fn render(
    index: &Index,
    filter: &Filter,
    from: u32,
    to: u32,
    json: bool,
    budget: usize,
) -> Option<String> {
    let (hops, inferred) = match shortest(index, filter, from, to, false) {
        Some(hops) => (hops, false),
        None => (shortest(index, filter, from, to, true)?, true),
    };
    let mut writer = Writer::new(json, budget);
    for (position, (node, reversed)) in hops.iter().enumerate() {
        let marker = if position == 0 {
            "."
        } else if *reversed {
            "<"
        } else {
            ">"
        };
        let mark = if inferred && position > 0 { "?" } else { "" };
        let label = index.label(*node);
        let path = index.path_of(*node);
        let line = index.line_of(*node);
        let text = if label == path {
            format!("{marker} {path}{mark}")
        } else {
            format!("{marker} {label} {path}:{line}{mark}")
        };
        writer.line(
            &text,
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str(if *reversed { "in" } else { "out" })),
                ("name", Value::Str(label)),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(line))),
                (
                    "conf",
                    Value::Str(if inferred { "inferred" } else { "exact" }),
                ),
            ],
        );
    }
    Some(writer.finish())
}
