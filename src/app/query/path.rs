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

/// Edge kinds that connect ranked hits: what runs what, what extends what.
const CHAIN_KINDS: [EdgeKind; 4] = [
    EdgeKind::Calls,
    EdgeKind::Tests,
    EdgeKind::Inherits,
    EdgeKind::Implements,
];
const CHAIN_DEPTH: usize = 3;
/// Nodes one search may visit: a query stays within its latency target on any hub.
const CHAIN_VISITS: usize = 512;

/// One link of a chain: the node reached and the line of the relation in the node before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Link {
    pub node: u32,
    pub site: u32,
    pub exact: bool,
}

/// Directed chains of at most [`CHAIN_DEPTH`] edges between `hits`, the ones joining most hits
/// first, at most `limit`. Every link is an emitted edge; nothing connects by name or rank.
pub fn chains(index: &Index, filter: &Filter, hits: &[u32], limit: usize) -> Vec<(u32, Vec<Link>)> {
    let mut found: Vec<(u32, Vec<Link>)> = Vec::new();
    for &start in hits {
        let mut visited = 1;
        let mut seen = vec![start];
        let mut layer: Vec<(u32, Vec<Link>)> = vec![(start, Vec::new())];
        for _ in 0..CHAIN_DEPTH {
            let mut next = Vec::new();
            for (node, links) in &layer {
                for edge in index.derived.outgoing(*node) {
                    if !CHAIN_KINDS.contains(&edge.kind())
                        || !filter.edge(edge)
                        || !filter.includes(index, edge.node)
                        || seen.contains(&edge.node)
                        || visited >= CHAIN_VISITS
                    {
                        continue;
                    }
                    visited += 1;
                    seen.push(edge.node);
                    let mut extended = links.clone();
                    extended.push(Link {
                        node: edge.node,
                        site: edge.line,
                        exact: edge.is_exact(),
                    });
                    if hits.contains(&edge.node) {
                        found.push((start, extended.clone()));
                    }
                    next.push((edge.node, extended));
                }
            }
            layer = next;
        }
    }
    let joined = |chain: &(u32, Vec<Link>)| {
        1 + chain
            .1
            .iter()
            .filter(|link| hits.contains(&link.node))
            .count()
    };
    let rank = |node: u32| {
        hits.iter()
            .position(|&hit| hit == node)
            .unwrap_or(hits.len())
    };
    found.sort_by_key(|chain| {
        (
            std::cmp::Reverse(joined(chain)),
            chain.1.len(),
            chain.1.iter().filter(|link| !link.exact).count(),
            rank(chain.0),
            chain.1.iter().map(|link| link.node).collect::<Vec<_>>(),
        )
    });
    let mut chosen: Vec<(u32, Vec<Link>)> = Vec::new();
    for chain in found {
        // A chain whose hits another chain already joins adds nothing.
        let members = |chain: &(u32, Vec<Link>)| -> Vec<u32> {
            std::iter::once(chain.0)
                .chain(chain.1.iter().map(|link| link.node))
                .filter(|node| hits.contains(node))
                .collect()
        };
        let own = members(&chain);
        if chosen.iter().any(|other| {
            let shown = members(other);
            own.iter().all(|node| shown.contains(node))
        }) {
            continue;
        }
        chosen.push(chain);
        if chosen.len() == limit {
            break;
        }
    }
    chosen
}

/// Writes `~ A path:line >site B path:line ...`: `>site` is the relation's line in the node before.
pub fn write_chain(index: &Index, writer: &mut Writer, start: u32, links: &[Link]) {
    let name = |node: u32| index.short_label(node).into_owned();
    let mut last_path = index.path_of(start).to_owned();
    let mut text = format!("~ {} {last_path}:{}", name(start), index.line_of(start));
    let mut hops = vec![format!(
        "{last_path}:{} {}",
        index.line_of(start),
        name(start)
    )];
    for link in links {
        let path = index.path_of(link.node);
        let mark = if link.exact { "" } else { "?" };
        let location = if path == last_path {
            format!(":{}", index.line_of(link.node))
        } else {
            format!("{path}:{}", index.line_of(link.node))
        };
        text.push_str(&format!(
            " >{}{mark} {} {location}",
            link.site,
            name(link.node)
        ));
        hops.push(format!(
            "{path}:{} {} site {} {}",
            index.line_of(link.node),
            name(link.node),
            link.site,
            if link.exact { "exact" } else { "inferred" },
        ));
        path.clone_into(&mut last_path);
    }
    writer.line(
        &text,
        &[("t", Value::Str("chain")), ("hops", Value::Strings(&hops))],
    );
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
