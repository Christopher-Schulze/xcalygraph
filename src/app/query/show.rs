//! `show`: the card of one symbol or file (`docs/spec.md` section 7.4).

use std::collections::HashSet;

use super::output::{Value, Writer};
use super::{Filter, Index, Node};
use crate::domain::graph::EdgeKind;
use crate::infra::store::derived::EdgeRecord;

/// Lines per section unless `--all`.
const SECTION_LINES: usize = 10;

pub struct ShowOptions {
    pub depth: usize,
    pub all: bool,
    pub json: bool,
    pub budget: usize,
}

const INCOMING: [EdgeKind; 5] = [
    EdgeKind::Calls,
    EdgeKind::Uses,
    EdgeKind::References,
    EdgeKind::Runs,
    EdgeKind::Queries,
];
const OUTGOING: [EdgeKind; 6] = [
    EdgeKind::Calls,
    EdgeKind::Uses,
    EdgeKind::References,
    EdgeKind::Runs,
    EdgeKind::Queries,
    EdgeKind::Imports,
];

pub fn render(index: &Index, filter: &Filter, node: u32, options: &ShowOptions) -> String {
    let mut writer = Writer::new(options.json, options.budget);
    let cap = if options.all {
        usize::MAX
    } else {
        SECTION_LINES
    };
    header(index, &mut writer, node);
    if let Node::Symbol(_) = index.node(node) {
        members(index, filter, &mut writer, node, cap);
        let supertypes = edges(
            index,
            filter,
            index.derived.outgoing(node),
            &[EdgeKind::Inherits, EdgeKind::Implements],
        );
        relation(index, &mut writer, "^", "sup", &supertypes, cap, false);
        let subtypes = edges(
            index,
            filter,
            index.derived.incoming(node),
            &[EdgeKind::Inherits, EdgeKind::Implements],
        );
        relation(index, &mut writer, "v", "sub", &subtypes, cap, true);
    }
    traverse(
        index,
        filter,
        &mut writer,
        node,
        options.depth,
        cap,
        Direction::In,
    );
    traverse(
        index,
        filter,
        &mut writer,
        node,
        options.depth,
        cap,
        Direction::Out,
    );
    tests(index, filter, &mut writer, node, cap);
    envs(index, &mut writer, node, cap);
    let ambiguous = index.file_of(node).ambiguous;
    if ambiguous > 0 {
        let text = format!(
            "! {ambiguous} ambiguous references in {}",
            index.path_of(node)
        );
        writer.line(
            &text,
            &[("t", Value::Str("note")), ("name", Value::Str(&text))],
        );
    }
    writer.finish()
}

fn header(index: &Index, writer: &mut Writer, node: u32) {
    match index.node(node) {
        Node::Symbol(symbol) => {
            let path = index.path_of(node);
            let qualified = index.string(symbol.qualified);
            let text = format!(
                "{path}:{}-{} {} {qualified}",
                symbol.line,
                symbol.end_line,
                symbol.kind().id()
            );
            let signature = index.string(symbol.signature);
            let doc = index.string(symbol.doc);
            let record = [
                ("t", Value::Str("sym")),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(symbol.line))),
                ("end", Value::Int(u64::from(symbol.end_line))),
                ("kind", Value::Str(symbol.kind().id())),
                ("name", Value::Str(qualified)),
                ("sig", Value::Str(signature)),
                ("doc", Value::Str(doc)),
            ];
            writer.line(&text, &record);
            if !writer.is_json() {
                if !signature.is_empty() {
                    writer.line(signature, &[]);
                }
                if !doc.is_empty() {
                    writer.line(&format!("// {doc}"), &[]);
                }
            }
        }
        Node::File(file) => {
            let path = index.string(file.path);
            let kind = file
                .file_type()
                .map_or(file.role().id(), |file_type| file_type.id());
            let text = format!(
                "{path} {kind} {} lines {} symbols",
                file.lines, file.symbol_count
            );
            let doc = crate::domain::facts::first_sentence(index.string(file.doc));
            writer.line(
                &text,
                &[
                    ("t", Value::Str("file")),
                    ("path", Value::Str(path)),
                    ("kind", Value::Str(kind)),
                    ("doc", Value::Str(&doc)),
                ],
            );
            if !writer.is_json() && !doc.is_empty() {
                writer.line(&format!("// {doc}"), &[]);
            }
            let symbols = index.derived.symbols();
            let top: Vec<u32> = (file.first_symbol..file.first_symbol + file.symbol_count)
                .filter(|&s| symbols[s as usize].parent == u32::MAX)
                .collect();
            for (shown, symbol) in top.iter().enumerate() {
                if shown >= SECTION_LINES * 3 {
                    writer.skip(top.len() - shown);
                    break;
                }
                index.write_symbol(writer, *symbol, None, false);
            }
        }
    }
}

fn members(index: &Index, filter: &Filter, writer: &mut Writer, node: u32, cap: usize) {
    let members: Vec<u32> = index
        .derived
        .members(node)
        .filter(|&member| filter.node(index, member))
        .collect();
    for (shown, member) in members.iter().enumerate() {
        if shown >= cap {
            writer.skip(members.len() - shown);
            break;
        }
        let Node::Symbol(symbol) = index.node(*member) else {
            continue;
        };
        let path = index.path_of(*member);
        let text = format!("m {} {}:{}", index.summary(symbol), path, symbol.line);
        writer.line(
            &text,
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("member")),
                ("name", Value::Str(index.string(symbol.qualified))),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(symbol.line))),
                ("sig", Value::Str(index.string(symbol.signature))),
            ],
        );
    }
    if let Some(fields) = index
        .derived
        .fields()
        .iter()
        .find(|fields| fields.symbol == node)
    {
        let names = index.string(fields.names);
        writer.line(
            &format!("f {names}"),
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("fields")),
                ("name", Value::Str(names)),
            ],
        );
    }
}

fn edges<'a>(
    index: &Index,
    filter: &Filter,
    list: &'a [EdgeRecord],
    kinds: &[EdgeKind],
) -> Vec<&'a EdgeRecord> {
    let mut found: Vec<&EdgeRecord> = list
        .iter()
        .filter(|edge| {
            kinds.contains(&edge.kind()) && filter.edge(edge) && filter.node(index, edge.node)
        })
        .collect();
    // Calls before type uses and references; within a kind by rank.
    found.sort_by(|a, b| {
        kind_order(a.kind())
            .cmp(&kind_order(b.kind()))
            .then(rank_of(index, b.node).total_cmp(&rank_of(index, a.node)))
            .then(a.node.cmp(&b.node))
    });
    found
}

fn kind_order(kind: EdgeKind) -> u8 {
    match kind {
        EdgeKind::Calls | EdgeKind::Tests => 0,
        EdgeKind::Inherits | EdgeKind::Implements => 1,
        EdgeKind::Runs | EdgeKind::Queries => 2,
        EdgeKind::Uses => 3,
        _ => 4,
    }
}

fn rank_of(index: &Index, node: u32) -> f32 {
    match index.node(node) {
        Node::Symbol(symbol) => symbol.rank,
        Node::File(_) => 0.0,
    }
}

/// One related node: `<marker> <label> <path>:<line>[?]`; `site` prints the reference line
/// instead of the definition line (incoming edges).
fn relation(
    index: &Index,
    writer: &mut Writer,
    marker: &str,
    rel: &str,
    list: &[&EdgeRecord],
    cap: usize,
    site: bool,
) {
    for (shown, edge) in list.iter().enumerate() {
        if shown >= cap {
            writer.skip(list.len() - shown);
            break;
        }
        write_edge(index, writer, marker, rel, edge, 0, site);
    }
}

fn write_edge(
    index: &Index,
    writer: &mut Writer,
    marker: &str,
    rel: &str,
    edge: &EdgeRecord,
    depth: usize,
    site: bool,
) -> bool {
    let path = index.path_of(edge.node);
    let line = if site {
        edge.line
    } else {
        index.line_of(edge.node)
    };
    let label = index.label(edge.node);
    let inferred = if edge.is_exact() { "" } else { "?" };
    let indent = "  ".repeat(depth);
    let kind = match edge.kind() {
        EdgeKind::Calls | EdgeKind::Tests | EdgeKind::Inherits | EdgeKind::Implements => {
            String::new()
        }
        other => format!(" {}", other.id()),
    };
    let text = if index.path_of(edge.node) == label {
        format!("{indent}{marker} {path}:{line}{kind}{inferred}")
    } else {
        format!("{indent}{marker} {label} {path}:{line}{kind}{inferred}")
    };
    writer.line(
        &text,
        &[
            ("t", Value::Str("edge")),
            ("rel", Value::Str(rel)),
            ("kind", Value::Str(edge.kind().id())),
            ("name", Value::Str(label)),
            ("path", Value::Str(path)),
            ("line", Value::Int(u64::from(line))),
            (
                "conf",
                Value::Str(if edge.is_exact() { "exact" } else { "inferred" }),
            ),
            ("n", Value::Int(depth as u64 + 1)),
        ],
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    In,
    Out,
}

/// Incoming or outgoing edges, `depth` hops, indented per hop; the cap counts all lines.
fn traverse(
    index: &Index,
    filter: &Filter,
    writer: &mut Writer,
    node: u32,
    depth: usize,
    cap: usize,
    direction: Direction,
) {
    let (marker, rel, kinds): (&str, &str, &[EdgeKind]) = match direction {
        Direction::In => ("<", "in", &INCOMING),
        Direction::Out => (">", "out", &OUTGOING),
    };
    let mut seen: HashSet<u32> = HashSet::from([node]);
    let mut written = 0usize;
    let mut stack: Vec<(u32, usize)> = vec![(node, 0)];
    let mut pending_total = 0usize;
    while let Some((current, level)) = stack.pop() {
        if level >= depth.max(1) {
            continue;
        }
        let list = match direction {
            Direction::In => index.derived.incoming(current),
            Direction::Out => index.derived.outgoing(current),
        };
        let found = edges(index, filter, list, kinds);
        let mut children = Vec::new();
        for edge in found {
            if !seen.insert(edge.node) {
                continue;
            }
            if written >= cap {
                pending_total += 1;
                continue;
            }
            write_edge(
                index,
                writer,
                marker,
                rel,
                edge,
                level,
                direction == Direction::In,
            );
            written += 1;
            children.push(edge.node);
        }
        for child in children.into_iter().rev() {
            stack.push((child, level + 1));
        }
    }
    writer.skip(pending_total);
}

/// Tests reaching the node directly or through one caller (test map, two `calls` hops).
fn tests(index: &Index, filter: &Filter, writer: &mut Writer, node: u32, cap: usize) {
    let mut found: Vec<(&EdgeRecord, bool)> = index
        .derived
        .incoming(node)
        .iter()
        .filter(|edge| edge.kind() == EdgeKind::Tests && filter.edge(edge))
        .map(|edge| (edge, false))
        .collect();
    let mut seen: HashSet<u32> = found.iter().map(|(edge, _)| edge.node).collect();
    for caller in index
        .derived
        .incoming(node)
        .iter()
        .filter(|edge| edge.kind() == EdgeKind::Calls)
    {
        for edge in index.derived.incoming(caller.node) {
            if edge.kind() == EdgeKind::Tests && filter.edge(edge) && seen.insert(edge.node) {
                found.push((edge, true));
            }
        }
    }
    for (shown, (edge, indirect)) in found.iter().enumerate() {
        if shown >= cap {
            writer.skip(found.len() - shown);
            break;
        }
        let marker = if *indirect { "t2" } else { "t" };
        write_edge(index, writer, marker, "test", edge, 0, true);
    }
}

fn envs(index: &Index, writer: &mut Writer, node: u32, cap: usize) {
    let symbol_end = match index.node(node) {
        Node::Symbol(symbol) => Some((symbol.file, symbol.line, symbol.end_line)),
        Node::File(_) => None,
    };
    let reads: Vec<_> = index
        .derived
        .envs()
        .iter()
        .filter(|env| env.source == 0)
        .filter(|env| {
            env.node == node
                || symbol_end.is_some_and(|(file, start, end)| {
                    index.file_index_of(env.node) == file && start <= env.line && env.line <= end
                })
        })
        .collect();
    for (shown, env) in reads.iter().enumerate() {
        if shown >= cap {
            writer.skip(reads.len() - shown);
            break;
        }
        let name = index.string(env.name);
        let path = index.path_of(env.node);
        writer.line(
            &format!("e {name} {path}:{}", env.line),
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("env")),
                ("name", Value::Str(name)),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(env.line))),
            ],
        );
    }
}
