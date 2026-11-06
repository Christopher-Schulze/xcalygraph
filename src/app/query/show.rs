//! `show`: the card of one symbol or file (`docs/spec.md` section 7.4).

use std::collections::{HashMap, HashSet};

use super::output::{Value, Writer};
use super::{Filter, Index, Node};
use crate::domain::graph::EdgeKind;
use crate::infra::store::derived::EdgeRecord;

/// Lines per section unless `--all`.
const SECTION_LINES: usize = 10;
/// Indentation levels of neighbours under a hit of `find`.
const CARD_INDENT: usize = 2;

struct EdgeDisplay<'a> {
    marker: &'a str,
    relation: &'a str,
    depth: usize,
    /// Extra levels of text indentation.
    indent: usize,
    site: bool,
    exact: bool,
}

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
    header(index, filter, &mut writer, node);
    if let Node::Symbol(_) = index.node(node) {
        members(index, filter, &mut writer, node, cap);
        for part in index.derived.interface_parts(node) {
            if part == node || !filter.node(index, part) {
                continue;
            }
            header(index, filter, &mut writer, part);
            members(index, filter, &mut writer, part, cap);
        }
        let mut supertypes: Vec<_> = index
            .derived
            .interface_parts(node)
            .filter(|&part| filter.node(index, part))
            .flat_map(|part| {
                edges(
                    index,
                    filter,
                    index.derived.outgoing(part),
                    &[EdgeKind::Inherits, EdgeKind::Implements],
                )
            })
            .collect();
        supertypes.sort_by(|a, b| {
            kind_order(a.kind())
                .cmp(&kind_order(b.kind()))
                .then(rank_of(index, b.node).total_cmp(&rank_of(index, a.node)))
                .then(a.node.cmp(&b.node))
                .then(b.is_exact().cmp(&a.is_exact()))
        });
        let mut positions: HashMap<(u32, EdgeKind), usize> = HashMap::new();
        let mut distinct: Vec<&EdgeRecord> = Vec::with_capacity(supertypes.len());
        for edge in supertypes {
            let key = (index.derived.interface_canonical(edge.node), edge.kind());
            if let Some(&position) = positions.get(&key) {
                if edge.is_exact() && !distinct[position].is_exact() {
                    distinct[position] = edge;
                }
            } else {
                positions.insert(key, distinct.len());
                distinct.push(edge);
            }
        }
        relation(index, &mut writer, ("^", "sup"), &distinct, cap, false, 0);
        let subtypes = edges(
            index,
            filter,
            index.derived.incoming(node),
            &[EdgeKind::Inherits, EdgeKind::Implements],
        );
        relation(index, &mut writer, ("v", "sub"), &subtypes, cap, true, 0);
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
    envs(index, filter, &mut writer, node, cap);
    if index.derived.incoming(node).is_empty() {
        let note = super::absence_note(index, filter, node);
        writer.line(
            &note,
            &[("t", Value::Str("note")), ("name", Value::Str(&note))],
        );
    }
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

fn header(index: &Index, filter: &Filter, writer: &mut Writer, node: u32) {
    match index.node(node) {
        Node::Symbol(symbol) => {
            let path = index.path_of(node);
            let qualified = index.qualified(symbol);
            let text = format!(
                "{path}:{}-{} {} {}",
                symbol.line,
                symbol.end_line,
                symbol.kind().id(),
                index.short_label(node)
            );
            let signature = index.string(symbol.signature);
            let doc = index.string(symbol.doc);
            let record = [
                ("t", Value::Str("sym")),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(symbol.line))),
                ("end", Value::Int(u64::from(symbol.end_line))),
                ("kind", Value::Str(symbol.kind().id())),
                ("name", Value::Str(&qualified)),
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
                    ("lines", Value::Int(u64::from(file.lines))),
                    ("symbols", Value::Int(u64::from(file.symbol_count))),
                    ("doc", Value::Str(&doc)),
                ],
            );
            if !writer.is_json() && !doc.is_empty() {
                writer.line(&format!("// {doc}"), &[]);
            }
            let symbols = index.derived.symbols();
            let top: Vec<u32> = (file.first_symbol..file.first_symbol + file.symbol_count)
                .filter(|&s| {
                    symbols[s as usize].parent == u32::MAX
                        && filter.symbol(index, &symbols[s as usize])
                })
                .collect();
            for (shown, symbol) in top.iter().enumerate() {
                if shown >= SECTION_LINES * 3 {
                    writer.skip(top.len() - shown);
                    break;
                }
                index.write_symbol(writer, *symbol, None);
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
        let qualified = index.qualified(symbol);
        writer.location_line(
            &format!("m {}", index.summary(symbol)),
            path,
            symbol.line,
            "",
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("member")),
                ("name", Value::Str(&qualified)),
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
/// Up to `cap` tests of the symbol and one command to run them, so a change can be verified
/// without a second call.
pub fn verification(
    index: &Index,
    filter: &Filter,
    writer: &mut Writer,
    node: u32,
    cap: usize,
    indent: usize,
) {
    let found = tests_of(index, filter, node);
    write_tests(index, writer, &found, cap, indent);
    let reachable = found
        .iter()
        .take(cap)
        .map(|test| (test.0.node, test.2))
        .collect();
    if let Some((command, _)) = super::impact::run_lines(index, &reachable).first() {
        let prefix = "  ".repeat(indent);
        writer.line(
            &format!("{prefix}r {command}"),
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("run")),
                ("name", Value::Str(command)),
            ],
        );
    }
}

/// Callers and callees of a symbol, at most `cap` of each.
pub fn neighbours(index: &Index, filter: &Filter, writer: &mut Writer, node: u32, cap: usize) {
    if !matches!(index.node(node), Node::Symbol(_)) {
        return;
    }
    let incoming = edges(index, filter, index.derived.incoming(node), &INCOMING);
    relation(
        index,
        writer,
        ("<", "in"),
        &incoming,
        cap,
        true,
        CARD_INDENT,
    );
    let outgoing = edges(index, filter, index.derived.outgoing(node), &OUTGOING);
    relation(
        index,
        writer,
        (">", "out"),
        &outgoing,
        cap,
        false,
        CARD_INDENT,
    );
}

fn relation(
    index: &Index,
    writer: &mut Writer,
    (marker, rel): (&str, &str),
    list: &[&EdgeRecord],
    cap: usize,
    site: bool,
    indent: usize,
) {
    let mut positions: HashMap<(u32, EdgeKind), usize> = HashMap::new();
    let mut distinct: Vec<&EdgeRecord> = Vec::with_capacity(list.len());
    for &edge in list {
        let groups = index.derived.interface_groups();
        if groups
            .binary_search_by_key(&edge.node, |pair| pair[0])
            .is_err()
        {
            distinct.push(edge);
            continue;
        }
        let key = (index.derived.interface_canonical(edge.node), edge.kind());
        if let Some(&position) = positions.get(&key) {
            if edge.is_exact() && !distinct[position].is_exact() {
                distinct[position] = edge;
            }
        } else {
            positions.insert(key, distinct.len());
            distinct.push(edge);
        }
    }
    for (shown, edge) in distinct.iter().enumerate() {
        if shown >= cap {
            writer.skip(distinct.len() - shown);
            break;
        }
        write_edge(
            index,
            writer,
            edge,
            EdgeDisplay {
                marker,
                relation: rel,
                depth: 0,
                indent,
                site,
                exact: edge.is_exact(),
            },
        );
    }
}

fn write_edge(
    index: &Index,
    writer: &mut Writer,
    edge: &EdgeRecord,
    display: EdgeDisplay<'_>,
) -> bool {
    let path = index.path_of(edge.node);
    let line = if display.site {
        edge.line
    } else {
        index.line_of(edge.node)
    };
    let label = index.label(edge.node);
    let inferred = if display.exact { "" } else { "?" };
    let indent = "  ".repeat(display.depth + display.indent);
    let kind = match edge.kind() {
        EdgeKind::Calls | EdgeKind::Tests | EdgeKind::Inherits | EdgeKind::Implements => {
            String::new()
        }
        other => format!(" {}", other.id()),
    };
    let prefix = if path == label {
        format!("{indent}{}", display.marker)
    } else {
        format!(
            "{indent}{} {}",
            display.marker,
            index.short_label(edge.node)
        )
    };
    writer.location_line(
        &prefix,
        path,
        line,
        &format!("{kind}{inferred}"),
        &[
            ("t", Value::Str("edge")),
            ("rel", Value::Str(display.relation)),
            ("kind", Value::Str(edge.kind().id())),
            ("name", Value::Str(&label)),
            ("path", Value::Str(path)),
            ("line", Value::Int(u64::from(line))),
            (
                "conf",
                Value::Str(if display.exact { "exact" } else { "inferred" }),
            ),
            ("n", Value::Int(display.depth as u64 + 1)),
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
    let node = index.derived.interface_canonical(node);
    let mut seen: HashSet<u32> = HashSet::from([node]);
    let mut written = 0usize;
    let mut stack: Vec<(u32, usize)> = vec![(node, 0)];
    let mut pending_total = 0usize;
    while let Some((current, level)) = stack.pop() {
        if level >= depth.max(1) {
            continue;
        }
        let found: Vec<_> = index
            .derived
            .interface_parts(current)
            .filter(|&part| filter.includes(index, part))
            .flat_map(|part| {
                let list = match direction {
                    Direction::In => index.derived.incoming(part),
                    Direction::Out => index.derived.outgoing(part),
                };
                edges(index, filter, list, kinds)
            })
            .collect();
        let mut children = Vec::new();
        for edge in found {
            let target = index.derived.interface_canonical(edge.node);
            if !seen.insert(target) {
                continue;
            }
            if written >= cap {
                pending_total += 1;
                continue;
            }
            write_edge(
                index,
                writer,
                edge,
                EdgeDisplay {
                    marker,
                    relation: rel,
                    depth: level,
                    indent: 0,
                    site: direction == Direction::In,
                    exact: edge.is_exact(),
                },
            );
            written += 1;
            children.push(target);
        }
        for child in children.into_iter().rev() {
            stack.push((child, level + 1));
        }
    }
    writer.skip(pending_total);
}

/// Tests reaching the node directly or through one caller (test map, two `calls` hops).
fn tests(index: &Index, filter: &Filter, writer: &mut Writer, node: u32, cap: usize) {
    write_tests(index, writer, &tests_of(index, filter, node), cap, 0);
}

/// Tests of a symbol, direct ones and through a caller: (edge, through a caller, exact).
pub(super) fn tests_of<'a>(
    index: &'a Index,
    filter: &Filter,
    node: u32,
) -> Vec<(&'a EdgeRecord, bool, bool)> {
    let mut found: Vec<(&EdgeRecord, bool, bool)> = Vec::new();
    let mut positions: HashMap<u32, usize> = HashMap::new();
    for edge in index.derived.incoming(node).iter().filter(|edge| {
        edge.kind() == EdgeKind::Tests && filter.edge(edge) && filter.includes(index, edge.node)
    }) {
        if let Some(&position) = positions.get(&edge.node) {
            if edge.is_exact() && !found[position].2 {
                found[position] = (edge, false, true);
            }
        } else {
            positions.insert(edge.node, found.len());
            found.push((edge, false, edge.is_exact()));
        }
    }
    for caller in index
        .derived
        .incoming(node)
        .iter()
        .filter(|edge| edge.kind() == EdgeKind::Calls)
    {
        if !filter.edge(caller) || !filter.includes(index, caller.node) {
            continue;
        }
        for edge in index.derived.incoming(caller.node) {
            if edge.kind() == EdgeKind::Tests
                && filter.edge(edge)
                && filter.includes(index, edge.node)
            {
                let exact = caller.is_exact() && edge.is_exact();
                if let Some(&position) = positions.get(&edge.node) {
                    if exact && !found[position].2 {
                        found[position] = (edge, true, true);
                    }
                } else {
                    positions.insert(edge.node, found.len());
                    found.push((edge, true, exact));
                }
            }
        }
    }
    found
}

fn write_tests(
    index: &Index,
    writer: &mut Writer,
    found: &[(&EdgeRecord, bool, bool)],
    cap: usize,
    indent: usize,
) {
    for (shown, (edge, indirect, exact)) in found.iter().enumerate() {
        if shown >= cap {
            writer.skip(found.len() - shown);
            break;
        }
        let marker = if *indirect { "t2" } else { "t" };
        write_edge(
            index,
            writer,
            edge,
            EdgeDisplay {
                marker,
                relation: "test",
                depth: 0,
                indent,
                site: true,
                exact: *exact,
            },
        );
    }
}

fn envs(index: &Index, filter: &Filter, writer: &mut Writer, node: u32, cap: usize) {
    let reads: Vec<_> = index
        .derived
        .envs()
        .iter()
        .filter(|env| env.source == 0)
        .filter(|env| filter.includes(index, env.node))
        .filter(|env| env.node == node)
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
