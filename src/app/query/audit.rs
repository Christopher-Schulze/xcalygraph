//! `audit`: dead code, unused exports, cycles, stale doc references, env drift, hidden coupling
//! and hotspots (`docs/spec.md` sections 6 and 7.7).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::output::{Value, Writer};
use super::{Filter, Index, Node};
use crate::domain::facts::{
    EnvSource, FLAG_DECLARATION, FLAG_ENTRY, FLAG_GENERATED, FLAG_TEST, SymKind,
};
use crate::domain::graph::EdgeKind;
use crate::domain::history::History;
use crate::domain::role::Role;

const SECTION_LINES: usize = 15;
const HOTSPOTS: usize = 10;

/// Names that frameworks, runtimes or the language call implicitly (Java serialization hooks).
fn implicit(name: &str) -> bool {
    matches!(
        name,
        "main"
            | "init"
            | "new"
            | "default"
            | "constructor"
            | "setup"
            | "teardown"
            | "readObject"
            | "writeObject"
            | "readResolve"
            | "writeReplace"
            | "readObjectNoData"
    ) || (name.starts_with("__") && name.ends_with("__"))
}

/// Methods called through a supertype outside the root, by construction or by a body without
/// a named type: overrides (`@Override`, `override`), constructors, and methods of enum constant
/// or anonymous class bodies.
fn called_implicitly(index: &Index, node: u32) -> bool {
    let symbols = index.derived.symbols();
    let symbol = &symbols[node as usize];
    if symbol.kind() != SymKind::Method {
        return false;
    }
    let signature = index.string(symbol.signature);
    if signature.contains("@Override")
        || signature.split_whitespace().any(|word| word == "override")
    {
        return true;
    }
    if symbol.parent == u32::MAX {
        return false;
    }
    let parent = &symbols[symbol.parent as usize];
    !parent.kind().is_type() || parent.name == symbol.name
}

pub fn render(
    index: &Index,
    filter: &Filter,
    root: &std::path::Path,
    history: Option<&History>,
    json: bool,
    budget: usize,
) -> String {
    let mut writer = Writer::new(json, budget);
    let owners = member_owners(index);
    let (dead, unused) = unreferenced(index, filter, &owners);
    section(
        &mut writer,
        "dead",
        dead.iter().map(|&node| symbol_line(index, node)).collect(),
    );
    section(
        &mut writer,
        "unused",
        unused
            .iter()
            .map(|&node| symbol_line(index, node))
            .collect(),
    );
    section(&mut writer, "cycles", cycles(index, filter));
    section(&mut writer, "stale", stale(index, filter, root));
    section(&mut writer, "drift", drift(index, filter, root));
    section(&mut writer, "assets", unreferenced_assets(index, filter));
    match history {
        Some(history) => {
            section(&mut writer, "coupling", coupling(index, filter, history));
            section(&mut writer, "hotspots", hotspots(index, filter, history));
        }
        None => {
            writer.line(
                "! git unavailable",
                &[
                    ("t", Value::Str("note")),
                    ("name", Value::Str("git unavailable")),
                ],
            );
        }
    }
    writer.finish()
}

fn section(writer: &mut Writer, keyword: &str, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    writer.line(
        keyword,
        &[("t", Value::Str("sec")), ("name", Value::Str(keyword))],
    );
    for (shown, line) in lines.iter().enumerate() {
        if shown >= SECTION_LINES {
            writer.line(
                &format!("+{} more", lines.len() - shown),
                &[
                    ("t", Value::Str("more")),
                    ("name", Value::Str(keyword)),
                    ("n", Value::Int((lines.len() - shown) as u64)),
                ],
            );
            break;
        }
        writer.line(
            line,
            &[
                ("t", Value::Str("note")),
                ("rel", Value::Str(keyword)),
                ("name", Value::Str(line)),
            ],
        );
    }
}

fn symbol_line(index: &Index, node: u32) -> String {
    format!(
        "{} {}:{}",
        index.label(node),
        index.path_of(node),
        index.line_of(node)
    )
}

/// Owning type of each member symbol (methods declared outside their type included).
fn member_owners(index: &Index) -> HashMap<u32, u32> {
    let mut owners = HashMap::new();
    for (owner, symbol) in index.derived.symbols().iter().enumerate() {
        if symbol.kind().is_type() {
            for member in index.derived.members(owner as u32) {
                owners.insert(member, owner as u32);
            }
        }
    }
    owners
}

/// Private symbols without any incoming edge (dead candidates) and exported symbols without
/// incoming edges from other files (unused exports).
fn unreferenced(
    index: &Index,
    filter: &Filter,
    owners: &HashMap<u32, u32>,
) -> (Vec<u32>, Vec<u32>) {
    let symbols = index.derived.symbols();
    let mut dead = Vec::new();
    let mut unused = Vec::new();
    for (id, symbol) in symbols.iter().enumerate() {
        let node = id as u32;
        let kind = symbol.kind();
        // Declarations (C prototypes, in-class C++ declarations) hand their uses to the definition.
        if symbol.flags & (FLAG_TEST | FLAG_ENTRY | FLAG_GENERATED | FLAG_DECLARATION) != 0
            || !matches!(
                kind,
                SymKind::Fn
                    | SymKind::Method
                    | SymKind::Struct
                    | SymKind::Class
                    | SymKind::Enum
                    | SymKind::Iface
                    | SymKind::Trait
                    | SymKind::Alias
                    | SymKind::Macro
            )
            || index.file_of(node).role() != Role::Src
            // SQL functions are called from SQL text, which yields no call edges.
            || index.file_of(node).file_type() == Some(crate::domain::filetype::FileType::Sql)
            || implicit(index.string(symbol.name))
            || !filter.symbol(index, symbol)
            || implements_supertype(index, node, owners)
            || called_implicitly(index, node)
            || declared_by_abstract_type(index, node)
        {
            continue;
        }
        let incoming = index.derived.incoming(node);
        if symbol.exported == 0 {
            // A name with ambiguous references may be one of their targets; a type whose members
            // are used from outside (`Type.CONSTANT`, static imports) is alive.
            if incoming.is_empty()
                && !index.derived.is_ambiguous_name(index.string(symbol.name))
                && !(kind.is_type()
                    && (members_used_outside(index, node) || has_entry_member(index, node)))
            {
                dead.push(node);
            }
        } else if kind != SymKind::Method
            && export_semantics(index.file_of(node).file_type())
            // A framework route (`page /x`, `GET /api/x`) using it is its outside caller.
            && incoming.iter().all(|edge| {
                index.file_of(edge.node).path == index.file_of(node).path
                    && !matches!(index.node(edge.node), Node::Symbol(caller) if caller.kind() == SymKind::Op)
            })
        {
            unused.push(node);
        }
    }
    let by_path = |nodes: &mut Vec<u32>| {
        nodes.sort_by(|a, b| {
            index
                .path_of(*a)
                .cmp(index.path_of(*b))
                .then(index.line_of(*a).cmp(&index.line_of(*b)))
        })
    };
    by_path(&mut dead);
    by_path(&mut unused);
    (dead, unused)
}

/// Whether a member of the type is an entrypoint or test (`Program` with `static Main`).
fn has_entry_member(index: &Index, owner: u32) -> bool {
    index.derived.members(owner).any(|member| {
        index.derived.symbols()[member as usize].flags & (FLAG_ENTRY | FLAG_TEST) != 0
    })
}

/// Whether a member of the type `owner` has an incoming edge from outside the type.
fn members_used_outside(index: &Index, owner: u32) -> bool {
    let members: Vec<u32> = index.derived.members(owner).collect();
    members.iter().any(|&member| {
        index
            .derived
            .incoming(member)
            .iter()
            .any(|edge| edge.node != owner && !members.contains(&edge.node))
    })
}

/// Methods declared by an interface or trait are contracts, not code that can be dead.
fn declared_by_abstract_type(index: &Index, node: u32) -> bool {
    let parent = index.derived.symbols()[node as usize].parent;
    parent != u32::MAX
        && index.derived.symbols()[parent as usize]
            .kind()
            .is_abstract()
}

/// Languages whose `exported` marks a deliberate public API (unused exports make sense).
fn export_semantics(file_type: Option<crate::domain::filetype::FileType>) -> bool {
    use crate::domain::filetype::FileType;
    matches!(
        file_type,
        Some(
            FileType::Go
                | FileType::Rust
                | FileType::TypeScript
                | FileType::JavaScript
                | FileType::Python
        )
    )
}

/// A method whose type has a supertype (interface, trait, base class) with a member of the same
/// name: called through dynamic dispatch, never dead by missing edges.
fn implements_supertype(index: &Index, node: u32, owners: &HashMap<u32, u32>) -> bool {
    let Some(&owner) = owners.get(&node) else {
        return false;
    };
    let name = index.string(index.derived.symbols()[node as usize].name);
    index
        .derived
        .outgoing(owner)
        .iter()
        .filter(|edge| matches!(edge.kind(), EdgeKind::Implements | EdgeKind::Inherits))
        .any(|edge| {
            index
                .derived
                .members(edge.node)
                .any(|member| index.string(index.derived.symbols()[member as usize].name) == name)
        })
}

fn cycles(index: &Index, filter: &Filter) -> Vec<String> {
    let mut groups: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    let included = super::map::included_modules(index, filter);
    for (id, module) in index.derived.modules().iter().enumerate() {
        if !included[id] {
            continue;
        }
        groups
            .entry(module.cycle)
            .or_default()
            .push(index.string(module.name));
    }
    let mut lines: Vec<String> = groups
        .into_values()
        .filter(|members| members.len() > 1)
        .map(|mut members| {
            members.sort_unstable();
            members.join(" <-> ")
        })
        .collect();
    lines.sort();
    lines
}

fn stale(index: &Index, filter: &Filter, root: &std::path::Path) -> Vec<String> {
    let mut lines: Vec<(String, u32, String)> = index
        .derived
        .stale()
        .iter()
        .filter(|record| filter.file(index, index.file_of(record[0])))
        .map(|record| {
            (
                index.path_of(record[0]).to_owned(),
                record[1],
                index.string(record[2]).to_owned(),
            )
        })
        // Ignored or untracked files exist on disk without being indexed.
        .filter(|(path, _, text)| {
            // Paths count only below an existing top-level directory: examples like `a/b.lua`
            // in a convention table are no repository references.
            let first = text.split('/').next().unwrap_or_default();
            if text.contains('/') && !root.join(first).is_dir() {
                return false;
            }
            let text = text
                .rsplit_once(':')
                .filter(|(_, lines)| lines.chars().all(|ch| ch.is_ascii_digit() || ch == '-'))
                .map_or(text.as_str(), |(path, _)| path);
            let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
            !(text.contains('/') || text.contains('.'))
                || !(root.join(dir).join(text).exists() || root.join(text).exists())
        })
        .collect();
    lines.sort();
    lines.dedup();
    lines
        .into_iter()
        .map(|(path, line, text)| format!("{text} {path}:{line}"))
        .collect()
}

/// Asset files that no code, document, template or config references or imports.
fn unreferenced_assets(index: &Index, filter: &Filter) -> Vec<String> {
    let symbols = index.symbol_count();
    index
        .derived
        .files()
        .iter()
        .enumerate()
        .filter(|(_, file)| file.role() == Role::Asset && filter.file(index, file))
        .filter(|(position, _)| {
            index
                .derived
                .incoming(symbols + *position as u32)
                .iter()
                .all(|edge| {
                    !matches!(
                        EdgeKind::from_u8(edge.kind),
                        Some(EdgeKind::References | EdgeKind::Imports)
                    )
                })
        })
        .map(|(_, file)| index.string(file.path).to_owned())
        .collect()
}

/// Variables read without a definition site and defined without a read site; only when the
/// root defines any environment variable (otherwise every read would be listed).
fn drift(index: &Index, filter: &Filter, root: &std::path::Path) -> Vec<String> {
    let envs = index.derived.envs();
    let defined: BTreeSet<&str> = envs
        .iter()
        .filter(|env| env.source != 0 && filter.includes(index, env.node))
        .map(|env| index.string(env.name))
        .collect();
    if defined.is_empty() {
        return Vec::new();
    }
    let read: BTreeSet<&str> = envs
        .iter()
        .filter(|env| env.source == 0 && filter.includes(index, env.node))
        .map(|env| index.string(env.name))
        .collect();
    let mut lines = Vec::new();
    for env in envs
        .iter()
        .filter(|env| env.source == 0 && filter.includes(index, env.node))
    {
        let name = index.string(env.name);
        if !defined.contains(name) {
            lines.push(format!(
                "read {name} {}:{} undefined",
                index.path_of(env.node),
                env.line
            ));
        }
    }
    // Unread definitions count only for runtime configuration: CI and Dockerfile variables are
    // often read by tools and base images, not by the code.
    let runtime = |source: u32| {
        u8::try_from(source)
            .ok()
            .and_then(EnvSource::from_u8)
            .is_some_and(|source| {
                matches!(
                    source,
                    EnvSource::EnvExample | EnvSource::Compose | EnvSource::Kubernetes
                )
            })
    };
    let unread: BTreeSet<&str> = envs
        .iter()
        .filter(|env| runtime(env.source) && filter.includes(index, env.node))
        .map(|env| index.string(env.name))
        .filter(|name| !read.contains(name))
        .collect();
    let wrapped = quoted_in_code(index, filter, root, &unread);
    for env in envs
        .iter()
        .filter(|env| runtime(env.source) && filter.includes(index, env.node))
    {
        let name = index.string(env.name);
        if unread.contains(name) && !wrapped.contains(name) {
            lines.push(format!(
                "defined {name} {}:{} unread",
                index.path_of(env.node),
                env.line
            ));
        }
    }
    lines.sort();
    lines.dedup_by(|a, b| {
        a.split_whitespace()
            .take(2)
            .eq(b.split_whitespace().take(2))
    });
    lines
}

/// Names of `candidates` written as string literals (`"NAME"`, `'NAME'`) in source files: read
/// through a wrapper (`mustMapEnv(&x, "NAME")`, `config["NAME"]`) the extractor cannot see.
fn quoted_in_code<'a>(
    index: &Index,
    filter: &Filter,
    root: &std::path::Path,
    candidates: &BTreeSet<&'a str>,
) -> BTreeSet<&'a str> {
    use rayon::prelude::*;
    if candidates.is_empty() {
        return BTreeSet::new();
    }
    let paths: Vec<&str> = index
        .derived
        .files()
        .iter()
        .filter(|file| matches!(file.role(), Role::Src | Role::Test) && filter.file(index, file))
        .map(|file| index.string(file.path))
        .collect();
    paths
        .par_iter()
        .flat_map_iter(|path| {
            let text = std::fs::read(root.join(path)).unwrap_or_default();
            let text = String::from_utf8_lossy(&text).into_owned();
            candidates
                .iter()
                .filter(|name| {
                    text.contains(&format!("\"{name}\"")) || text.contains(&format!("'{name}'"))
                })
                .copied()
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Co-change pairs with at least one source file and without any static edge between the two.
fn coupling(index: &Index, filter: &Filter, history: &History) -> Vec<String> {
    let files = index.derived.files();
    let file_index: HashMap<&str, u32> = files
        .iter()
        .enumerate()
        .map(|(id, file)| (index.string(file.path), id as u32))
        .collect();
    let mut linked: HashSet<(u32, u32)> = HashSet::new();
    for node in 0..index.derived.node_count() as u32 {
        let from = index.file_index_of(node);
        for edge in index.derived.outgoing(node) {
            let to = index.file_index_of(edge.node);
            if from != to {
                linked.insert((from.min(to), from.max(to)));
            }
        }
    }
    let mut pairs: Vec<(u32, String)> = history
        .co_change
        .iter()
        .filter_map(|(left, right, count)| {
            let (a, b) = (
                *file_index.get(left.as_str())?,
                *file_index.get(right.as_str())?,
            );
            let code = |file: u32| {
                matches!(
                    files[file as usize].role(),
                    Role::Src | Role::Config | Role::Manifest
                )
            };
            // Config and manifest pairs alone are mostly artifacts committed together.
            let source = |file: u32| files[file as usize].role() == Role::Src;
            let wanted = code(a)
                && code(b)
                && (source(a) || source(b))
                && filter.file(index, &files[a as usize])
                && filter.file(index, &files[b as usize]);
            (wanted && !linked.contains(&(a.min(b), a.max(b))))
                .then(|| (*count, format!("{left} {right} {count}")))
        })
        .collect();
    pairs.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    pairs.into_iter().map(|(_, line)| line).collect()
}

/// Churn x (1 + highest symbol rank in the file, normalized), top 10.
fn hotspots(index: &Index, filter: &Filter, history: &History) -> Vec<String> {
    let symbols = index.derived.symbols();
    let max_rank = symbols
        .iter()
        .map(|symbol| symbol.rank)
        .fold(0.0f32, f32::max)
        .max(f32::EPSILON);
    let mut scored: Vec<(f32, String)> = index
        .derived
        .files()
        .iter()
        .filter(|file| file.role() == Role::Src && filter.file(index, file))
        .filter_map(|file| {
            let path = index.string(file.path);
            let churn = *history.churn.get(path)?;
            let rank = (file.first_symbol..file.first_symbol + file.symbol_count)
                .map(|symbol| symbols[symbol as usize].rank / max_rank)
                .fold(0.0f32, f32::max);
            Some((
                churn as f32 * (1.0 + rank),
                format!("{path} {churn} commits"),
            ))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(HOTSPOTS)
        .map(|(_, line)| line)
        .collect()
}
