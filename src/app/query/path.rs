//! `path`: shortest dependency path between two nodes (`docs/spec.md` section 7.6).

use std::collections::HashMap;

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

/// One hop of a path: the node reached, whether the hop ran against edge direction and whether
/// it is proven (an exact edge taken in its own direction). The first hop is the start node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hop {
    pub node: u32,
    pub reversed: bool,
    pub exact: bool,
}

/// Shortest path from `from` to `to`; among equally short paths the one with the fewest
/// inferred hops, ties broken by adjacency order. Breadth-first by layers, so every
/// predecessor of a layer is known before the layer is expanded.
pub fn shortest(
    index: &Index,
    filter: &Filter,
    from: u32,
    to: u32,
    undirected: bool,
) -> Option<Vec<Hop>> {
    let requested_from = from;
    let requested_to = to;
    let from = index.derived.interface_canonical(from);
    let to = index.derived.interface_canonical(to);
    // node -> (inferred hops so far, predecessor, reversed, exact)
    let mut best: HashMap<u32, (u32, u32, bool, bool)> = HashMap::new();
    best.insert(from, (0, from, false, true));
    let mut layer = vec![from];
    while !layer.is_empty() && !best.contains_key(&to) {
        let mut next: Vec<u32> = Vec::new();
        let mut found: HashMap<u32, (u32, u32, bool, bool)> = HashMap::new();
        for &node in &layer {
            let inferred_so_far = best[&node].0;
            let parts = || {
                index
                    .derived
                    .interface_parts(node)
                    .filter(|&part| filter.includes(index, part))
            };
            let forward = parts().flat_map(|part| {
                index
                    .derived
                    .outgoing(part)
                    .iter()
                    .map(|edge| (edge, false))
            });
            let backward = parts().flat_map(|part| {
                index
                    .derived
                    .incoming(part)
                    .iter()
                    .filter(|_| undirected)
                    .map(|edge| (edge, true))
            });
            for (edge, reversed) in forward.chain(backward) {
                let target = index.derived.interface_canonical(edge.node);
                if !KINDS.contains(&edge.kind())
                    || !filter.edge(edge)
                    || best.contains_key(&target)
                    || !filter.includes(index, edge.node)
                {
                    continue;
                }
                let exact = edge.is_exact() && !reversed;
                let candidate = (inferred_so_far + u32::from(!exact), node, reversed, exact);
                match found.get(&target) {
                    Some(current) if current.0 <= candidate.0 => {}
                    Some(_) => {
                        found.insert(target, candidate);
                    }
                    None => {
                        found.insert(target, candidate);
                        next.push(target);
                    }
                }
            }
        }
        for node in &next {
            best.insert(*node, found[node]);
        }
        layer = next;
    }
    best.get(&to)?;
    let mut hops = Vec::new();
    let mut current = to;
    while current != from {
        let (_, before, reversed, exact) = best[&current];
        hops.push(Hop {
            node: current,
            reversed,
            exact,
        });
        current = before;
    }
    hops.push(Hop {
        node: from,
        reversed: false,
        exact: true,
    });
    hops.reverse();
    if let Some(first) = hops.first_mut() {
        first.node = requested_from;
    }
    if let Some(last) = hops.last_mut() {
        last.node = requested_to;
    }
    Some(hops)
}

pub fn render(
    index: &Index,
    filter: &Filter,
    from: u32,
    to: u32,
    json: bool,
    budget: usize,
) -> Option<String> {
    // An undirected path is inferred by construction, so `--exact` never falls back to it.
    let hops = shortest(index, filter, from, to, false)
        .or_else(|| (!filter.exact).then(|| shortest(index, filter, from, to, true))?)?;
    let mut writer = Writer::new(json, budget);
    for (position, hop) in hops.iter().enumerate() {
        let marker = if position == 0 {
            "."
        } else if hop.reversed {
            "<"
        } else {
            ">"
        };
        let mark = if hop.exact { "" } else { "?" };
        let label = index.label(hop.node);
        let path = index.path_of(hop.node);
        let line = index.line_of(hop.node);
        let record = [
            ("t", Value::Str("edge")),
            ("rel", Value::Str(if hop.reversed { "in" } else { "out" })),
            ("name", Value::Str(&label)),
            ("path", Value::Str(path)),
            ("line", Value::Int(u64::from(line))),
            (
                "conf",
                Value::Str(if hop.exact { "exact" } else { "inferred" }),
            ),
        ];
        if label == path {
            writer.line(&format!("{marker} {path}{mark}"), &record);
        } else {
            writer.location_line(
                &format!("{marker} {}", index.short_label(hop.node)),
                path,
                line,
                mark,
                &record,
            );
        }
    }
    Some(writer.finish())
}
