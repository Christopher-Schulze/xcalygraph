//! `map`: orientation on an unknown tree (`docs/spec.md` section 7.1).

use std::collections::{BTreeMap, HashMap};

use super::output::{OutputFact, Value, Writer, json_object};
use super::{Filter, Index};
use crate::domain::facts::{EnvSource, FLAG_ENTRY, FLAG_GENERATED, FLAG_TEST, SymKind};
use crate::domain::graph::{EdgeKind, ModuleEdge};
use crate::domain::role::Role;

const MODULE_LINES: usize = 40;
const SECTION_LINES: usize = 10;
const EXTERNAL_LINES: usize = 15;
const MODULE_DEPTH: usize = 3;

struct Section {
    keyword: &'static str,
    lines: Vec<RenderedFact>,
    cut: usize,
}

struct RenderedFact {
    text: String,
    json: String,
}

impl RenderedFact {
    fn from_fact(fact: &impl OutputFact) -> Self {
        Self {
            text: fact.text(),
            json: fact.json_line(),
        }
    }
}

struct SymbolFact {
    relation: &'static str,
    name: String,
    path: String,
    line: u32,
}

enum MapFact {
    Section(String),
    Root {
        name: String,
        files: u64,
        symbols: u64,
        languages: Vec<String>,
    },
    Module {
        name: String,
        files: u32,
        symbols: u32,
        top: Vec<String>,
    },
    Symbol(SymbolFact),
    Task {
        symbol: SymbolFact,
        runs: Vec<String>,
    },
    Api {
        symbol: SymbolFact,
        signature: Option<String>,
        handlers: Vec<String>,
    },
    Cycle(Vec<String>),
    Layer {
        depth: u32,
        members: Vec<String>,
    },
    External {
        name: String,
        uses: u32,
    },
    Env {
        name: String,
        reads: u32,
        sources: Vec<String>,
        first: Option<(String, u32)>,
    },
    More {
        text: String,
        count: u64,
    },
}

impl OutputFact for MapFact {
    fn text(&self) -> String {
        match self {
            Self::Section(name) => name.clone(),
            Self::Root {
                name,
                files,
                symbols,
                languages,
            } => format!(
                "{name} {files} files {symbols} symbols {}",
                languages.join(" ")
            )
            .trim_end()
            .to_owned(),
            Self::Module {
                name,
                files,
                symbols,
                top,
            } => format!("{name} {files}f {symbols}s {}", top.join(" "))
                .trim_end()
                .to_owned(),
            Self::Symbol(symbol) => symbol.text(),
            Self::Task { symbol, runs } => {
                let mut text = symbol.text();
                if !runs.is_empty() {
                    text.push_str(" > ");
                    text.push_str(&runs.join(", "));
                }
                text
            }
            Self::Api {
                symbol,
                signature,
                handlers,
            } => {
                let mut text = if let Some(signature) = signature {
                    format!(
                        "{} {signature} {}:{}",
                        symbol.name, symbol.path, symbol.line
                    )
                } else {
                    symbol.text()
                };
                if !handlers.is_empty() {
                    text.push_str(" < ");
                    text.push_str(&handlers.join(", "));
                }
                text
            }
            Self::Cycle(members) => members.join(" <-> "),
            Self::Layer { depth, members } => format!("{depth} {}", members.join(" ")),
            Self::External { name, uses } => format!("{name} {uses}"),
            Self::Env {
                name,
                reads,
                sources,
                first,
            } => env_text(name, *reads, sources, first.as_ref()),
            Self::More { text, .. } => text.clone(),
        }
    }

    fn json_line(&self) -> String {
        let summary = match self {
            Self::Root { languages, .. } => Some(languages.join(" ")),
            Self::Module { top, .. } => Some(top.join(" ")),
            Self::Cycle(members) => Some(members.join(" <-> ")),
            Self::Layer { members, .. } => Some(members.join(" ")),
            Self::Env { sources, .. } => Some(env_sources(sources)),
            _ => None,
        };
        let mut fields = Vec::new();
        match self {
            Self::Section(name) => {
                fields.extend([("t", Value::Str("sec")), ("name", Value::Str(name))])
            }
            Self::Root {
                name,
                files,
                symbols,
                ..
            } => fields.extend([
                ("t", Value::Str("root")),
                ("name", Value::Str(name)),
                ("n", Value::Int(*files)),
                ("sig", Value::Str(summary.as_deref().unwrap_or_default())),
                ("symbols", Value::Int(*symbols)),
            ]),
            Self::Module {
                name,
                files,
                symbols,
                ..
            } => fields.extend([
                ("t", Value::Str("mod")),
                ("name", Value::Str(name)),
                ("n", Value::Int(u64::from(*symbols))),
                ("sig", Value::Str(summary.as_deref().unwrap_or_default())),
                ("files", Value::Int(u64::from(*files))),
            ]),
            Self::Symbol(symbol) => symbol_fields(&mut fields, symbol),
            Self::Task { symbol, runs } => {
                symbol_fields(&mut fields, symbol);
                if !runs.is_empty() {
                    fields.push(("runs", Value::Strings(runs)));
                }
            }
            Self::Api {
                symbol,
                signature,
                handlers,
            } => {
                symbol_fields(&mut fields, symbol);
                if let Some(signature) = signature {
                    fields.push(("sig", Value::Str(signature)));
                }
                if !handlers.is_empty() {
                    fields.push(("handlers", Value::Strings(handlers)));
                }
            }
            Self::Cycle(members) => fields.extend([
                ("t", Value::Str("cycle")),
                ("name", Value::Str(summary.as_deref().unwrap_or_default())),
                ("n", Value::Int(members.len() as u64)),
                ("members", Value::Strings(members)),
            ]),
            Self::Layer { depth, members } => fields.extend([
                ("t", Value::Str("layer")),
                ("n", Value::Int(u64::from(*depth))),
                ("name", Value::Str(summary.as_deref().unwrap_or_default())),
                ("members", Value::Strings(members)),
            ]),
            Self::External { name, uses } => fields.extend([
                ("t", Value::Str("ext")),
                ("name", Value::Str(name)),
                ("n", Value::Int(u64::from(*uses))),
            ]),
            Self::Env {
                name, reads, first, ..
            } => {
                fields.extend([
                    ("t", Value::Str("env")),
                    ("name", Value::Str(name)),
                    ("n", Value::Int(u64::from(*reads))),
                    ("sig", Value::Str(summary.as_deref().unwrap_or_default())),
                ]);
                if let Some((path, line)) = first {
                    fields.push(("path", Value::Str(path)));
                    fields.push(("line", Value::Int(u64::from(*line))));
                }
            }
            Self::More { count, .. } => {
                fields.extend([("t", Value::Str("more")), ("n", Value::Int(*count))])
            }
        }
        json_object(&fields)
    }
}

impl SymbolFact {
    fn text(&self) -> String {
        if self.name == self.path {
            self.path.clone()
        } else {
            format!("{} {}:{}", self.name, self.path, self.line)
        }
    }
}

fn symbol_fields<'a>(fields: &mut Vec<(&'static str, Value<'a>)>, symbol: &'a SymbolFact) {
    fields.extend([
        ("t", Value::Str("sym")),
        ("rel", Value::Str(symbol.relation)),
        ("name", Value::Str(&symbol.name)),
        ("path", Value::Str(&symbol.path)),
        ("line", Value::Int(u64::from(symbol.line))),
    ]);
}

fn env_sources(sources: &[String]) -> String {
    if sources.is_empty() {
        "-".to_owned()
    } else {
        sources.join(",")
    }
}

fn env_text(name: &str, reads: u32, sources: &[String], first: Option<&(String, u32)>) -> String {
    let sources = env_sources(sources);
    let first = first.map_or_else(String::new, |(path, line)| format!(" {path}:{line}"));
    format!("{name} {reads}r {sources}{first}")
}

pub fn render(
    index: &Index,
    filter: &Filter,
    root_name: &str,
    json: bool,
    budget: usize,
) -> String {
    let included = included_modules(index, filter);
    let mut sections = vec![
        Section {
            keyword: "",
            lines: vec![RenderedFact::from_fact(&root_line(
                index, filter, root_name,
            ))],
            cut: 0,
        },
        section("modules", modules(index, filter), MODULE_LINES),
        section("entry", entrypoints(index, filter), SECTION_LINES),
        section("tasks", tasks(index, filter), SECTION_LINES),
        section("hubs", hubs(index, filter), SECTION_LINES),
        section("api", api(index, filter), SECTION_LINES),
        section("cycles", cycles(index, &included), SECTION_LINES),
        section("layers", layers(index, &included), SECTION_LINES),
        section("ext", externals(index, &included), EXTERNAL_LINES),
        section("env", envs(index, filter), SECTION_LINES),
    ];
    sections.retain(|section| !section.lines.is_empty());
    // The budget cuts from the lowest-priority section upward.
    // Reserve truncation space under the same bound as every other writer. Tiny budgets can
    // return a compact notice or the minimal JSON truncation object when needed.
    let limit = budget
        .saturating_mul(super::output::BYTES_PER_TOKEN)
        .saturating_sub(super::output::footer_len(json));
    let keyword_size = |keyword: &str| -> usize {
        match (keyword.is_empty(), json) {
            (true, _) => 0,
            (false, true) => {
                json_object(&[("t", Value::Str("sec")), ("name", Value::Str(keyword))]).len()
            }
            (false, false) => super::output::escaped_text_len(keyword) + 1,
        }
    };
    let size = |sections: &[Section]| -> usize {
        sections
            .iter()
            .map(|section| {
                keyword_size(section.keyword)
                    + section
                        .lines
                        .iter()
                        .map(|fact| {
                            if json {
                                fact.json.len()
                            } else {
                                super::output::escaped_text_len(&fact.text) + 1
                            }
                        })
                        .sum::<usize>()
            })
            .sum()
    };
    let mut dropped = 0;
    while size(&sections) > limit && !sections.is_empty() {
        if sections.len() == 1 {
            dropped += sections[0].lines.len() + sections[0].cut;
            sections.clear();
            break;
        }
        let Some(last) = sections.last_mut() else {
            break;
        };
        if last.lines.pop().is_some() {
            last.cut += 1;
        }
        if last.lines.is_empty() {
            let cut = last.cut;
            sections.pop();
            if let Some(previous) = sections.last_mut() {
                previous.cut += cut + 1;
            }
        }
    }
    let mut writer = Writer::new(json, budget);
    let mut cut = 0;
    for section in &sections {
        if !section.keyword.is_empty() {
            writer.fact(&MapFact::Section(section.keyword.to_owned()));
        }
        for fact in &section.lines {
            writer.rendered_fact(&fact.text, &fact.json);
        }
        cut += section.cut;
    }
    writer.skip(cut + dropped);
    writer.finish()
}

fn section(keyword: &'static str, mut lines: Vec<MapFact>, cap: usize) -> Section {
    let cut = lines.len().saturating_sub(cap);
    lines.truncate(cap);
    Section {
        keyword,
        lines: lines.iter().map(RenderedFact::from_fact).collect(),
        cut,
    }
}

fn root_line(index: &Index, filter: &Filter, root_name: &str) -> MapFact {
    let mut languages: BTreeMap<&str, u32> = BTreeMap::new();
    let mut files = 0;
    for file in index.derived.files() {
        if !filter.file(index, file) {
            continue;
        }
        files += 1;
        if matches!(file.role(), Role::Src | Role::Test | Role::Gen)
            && let Some(file_type) = file.file_type()
        {
            *languages.entry(file_type.id()).or_default() += 1;
        }
    }
    let mut ordered: Vec<(&str, u32)> = languages.into_iter().collect();
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let symbols = index
        .derived
        .symbols()
        .iter()
        .filter(|symbol| filter.symbol(index, symbol))
        .count();
    let languages: Vec<String> = ordered
        .iter()
        .map(|(id, count)| format!("{id}:{count}"))
        .collect();
    MapFact::Root {
        name: root_name.to_owned(),
        files,
        symbols: symbols as u64,
        languages,
    }
}

/// Module tree by path, depth max 3: deeper modules fold into their ancestor at depth 3.
fn modules(index: &Index, filter: &Filter) -> Vec<MapFact> {
    struct Entry {
        files: u32,
        symbols: u32,
        top: Vec<(f32, u32)>,
    }
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut module_key: HashMap<u32, String> = HashMap::new();
    for (id, module) in index.derived.modules().iter().enumerate() {
        let name = index.string(module.name);
        let separator = if name.contains("::") { "::" } else { "/" };
        let folded: Vec<&str> = name.split(separator).take(MODULE_DEPTH).collect();
        module_key.insert(id as u32, folded.join(separator));
    }
    for file in index.derived.files() {
        if !filter.file(index, file) || !matches!(file.role(), Role::Src | Role::Gen) {
            continue;
        }
        let Some(key) = module_key.get(&file.module) else {
            continue;
        };
        let entry = entries.entry(key.clone()).or_insert(Entry {
            files: 0,
            symbols: 0,
            top: Vec::new(),
        });
        entry.files += 1;
        for symbol_id in file.first_symbol..file.first_symbol + file.symbol_count {
            let symbol = &index.derived.symbols()[symbol_id as usize];
            if !filter.symbol(index, symbol) || symbol.flags & (FLAG_TEST | FLAG_GENERATED) != 0 {
                continue;
            }
            entry.symbols += 1;
            entry.top.push((symbol.rank, symbol_id));
        }
    }
    // The largest modules, shown in path order.
    let mut largest: Vec<(String, Entry)> = entries
        .into_iter()
        .filter(|(_, entry)| entry.symbols > 0)
        .collect();
    largest.sort_by(|a, b| b.1.symbols.cmp(&a.1.symbols).then(a.0.cmp(&b.0)));
    let hidden = largest.len().saturating_sub(MODULE_LINES);
    largest.truncate(MODULE_LINES);
    largest.sort_by(|a, b| a.0.cmp(&b.0));
    let mut lines: Vec<MapFact> = largest
        .into_iter()
        .map(|(name, mut entry)| {
            entry
                .top
                .sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            let top: Vec<&str> = entry
                .top
                .iter()
                .take(3)
                .map(|&(_, symbol)| index.string(index.derived.symbols()[symbol as usize].name))
                .collect();
            MapFact::Module {
                name,
                files: entry.files,
                symbols: entry.symbols,
                top: top.into_iter().map(str::to_owned).collect(),
            }
        })
        .collect();
    if hidden > 0 {
        lines.push(MapFact::More {
            text: format!("+{hidden} smaller modules"),
            count: hidden as u64,
        });
    }
    lines
}

fn symbol_line(index: &Index, node: u32, rel: &'static str) -> SymbolFact {
    let label = index.label(node);
    let path = index.path_of(node);
    let line = index.line_of(node);
    SymbolFact {
        relation: rel,
        name: label.into_owned(),
        path: path.to_owned(),
        line,
    }
}

fn entrypoints(index: &Index, filter: &Filter) -> Vec<MapFact> {
    let mut lines: Vec<MapFact> = Vec::new();
    for (id, symbol) in index.derived.symbols().iter().enumerate() {
        let task = matches!(symbol.kind(), SymKind::Task | SymKind::Svc);
        if symbol.flags & FLAG_ENTRY != 0 && !task && filter.symbol(index, symbol) {
            lines.push(MapFact::Symbol(symbol_line(index, id as u32, "entry")));
        }
    }
    let symbols = index.derived.symbols();
    for (id, file) in index.derived.files().iter().enumerate() {
        // A file is the entry when none of its symbols is (a manifest `bin` with helpers).
        let first = file.first_symbol as usize;
        let entry_symbol = symbols
            .get(first..first + file.symbol_count as usize)
            .is_some_and(|own| own.iter().any(|symbol| symbol.flags & FLAG_ENTRY != 0));
        if file.entry != 0 && !entry_symbol && filter.file(index, file) {
            lines.push(MapFact::Symbol(symbol_line(
                index,
                index.symbol_count() + id as u32,
                "entry",
            )));
        }
    }
    lines
}

/// Makefile targets, package scripts, CI jobs and services with what they run:
/// `<name> <path>:<line> > <target>, ...`; tasks running most first.
fn tasks(index: &Index, filter: &Filter) -> Vec<MapFact> {
    let mut found: Vec<(usize, String, MapFact)> = Vec::new();
    for (id, symbol) in index.derived.symbols().iter().enumerate() {
        if !matches!(symbol.kind(), SymKind::Task | SymKind::Svc) || !filter.symbol(index, symbol) {
            continue;
        }
        let node = id as u32;
        let runs: Vec<_> = index
            .derived
            .outgoing(node)
            .iter()
            .filter(|edge| edge.kind() == EdgeKind::Runs)
            .map(|edge| index.label(edge.node).into_owned())
            .collect();
        let run_count = runs.len();
        let fact = MapFact::Task {
            symbol: symbol_line(index, node, "task"),
            runs,
        };
        found.push((run_count, fact.text(), fact));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, _, fact)| fact).collect()
}

/// API operations (OpenAPI `op`, Protobuf rpcs, GraphQL root fields) with their handlers:
/// `<name> [METHOD /path] <path>:<line> < <handler>, ...`; implemented operations first.
fn api(index: &Index, filter: &Filter) -> Vec<MapFact> {
    use crate::domain::filetype::FileType;
    let symbols = index.derived.symbols();
    let mut found: Vec<(bool, String, MapFact)> = Vec::new();
    for (id, symbol) in symbols.iter().enumerate() {
        let file_type = index.file_of(id as u32).file_type();
        let root_field = symbol.parent != u32::MAX
            && matches!(
                index.string(symbols[symbol.parent as usize].name),
                "Query" | "Mutation" | "Subscription"
            );
        let contract = symbol.kind() == SymKind::Op
            || (symbol.kind() == SymKind::Method && file_type == Some(FileType::Proto))
            || (symbol.kind() == SymKind::Method
                && file_type == Some(FileType::GraphQl)
                && root_field);
        if !contract || !filter.symbol(index, symbol) {
            continue;
        }
        let node = id as u32;
        let handlers: Vec<_> = index
            .derived
            .incoming(node)
            .iter()
            .filter(|edge| edge.kind() == EdgeKind::Implements)
            .map(|edge| index.label(edge.node).into_owned())
            .collect();
        let signature = (symbol.kind() == SymKind::Op)
            .then(|| index.string(symbol.signature))
            .filter(|signature| !signature.is_empty() && *signature != index.string(symbol.name))
            .map(str::to_owned);
        let has_no_handlers = handlers.is_empty();
        let fact = MapFact::Api {
            symbol: symbol_line(index, node, "api"),
            signature,
            handlers,
        };
        found.push((has_no_handlers, fact.text(), fact));
    }
    found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, _, fact)| fact).collect()
}

fn hubs(index: &Index, filter: &Filter) -> Vec<MapFact> {
    let mut ranked: Vec<(f32, u32)> = index
        .derived
        .symbols()
        .iter()
        .enumerate()
        .filter(|(_, symbol)| {
            filter.symbol(index, symbol) && symbol.flags & (FLAG_TEST | FLAG_GENERATED) == 0
        })
        .map(|(id, symbol)| (symbol.rank, id as u32))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked
        .into_iter()
        .take(SECTION_LINES)
        .map(|(_, node)| MapFact::Symbol(symbol_line(index, node, "hub")))
        .collect()
}

/// Per module: whether any of its files passes the filter. Module-level sections (cycles,
/// layers, externals) show only these modules, so a path restriction never exposes others.
pub(super) fn included_modules(index: &Index, filter: &Filter) -> Vec<bool> {
    let mut included = vec![false; index.derived.modules().len()];
    for file in index.derived.files() {
        if let Some(flag) = included.get_mut(file.module as usize)
            && filter.file(index, file)
        {
            *flag = true;
        }
    }
    included
}

fn cycles(index: &Index, included: &[bool]) -> Vec<MapFact> {
    let mut lines: Vec<MapFact> = cycle_groups(index, included)
        .into_iter()
        .map(|members| MapFact::Cycle(members.into_iter().map(str::to_owned).collect()))
        .collect();
    lines.sort_by_key(OutputFact::text);
    lines
}

/// Strongly connected groups in the selected module subgraph, ordered within each group.
pub(super) fn cycle_groups<'a>(index: &'a Index, included: &[bool]) -> Vec<Vec<&'a str>> {
    let modules = index.derived.modules();
    let components = if included.iter().all(|&selected| selected) {
        modules.iter().map(|module| module.cycle).collect()
    } else {
        let edges: Vec<ModuleEdge> = index
            .derived
            .module_edges()
            .iter()
            .filter(|edge| included[edge.source as usize] && included[edge.target as usize])
            .map(|edge| ModuleEdge {
                source: edge.source,
                target: edge.target,
                count: edge.count,
            })
            .collect();
        crate::resolve::derive::strongly_connected(modules.len(), &edges)
    };
    let mut groups: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for (id, module) in modules.iter().enumerate() {
        if included[id] {
            groups
                .entry(components[id])
                .or_default()
                .push(index.string(module.name));
        }
    }
    groups
        .into_values()
        .filter(|members| members.len() > 1)
        .map(|mut members| {
            members.sort_unstable();
            members
        })
        .collect()
}

/// Modules per layer, top layer first; layer 0 has no dependencies inside the root.
fn layers(index: &Index, included: &[bool]) -> Vec<MapFact> {
    let modules = index.derived.modules();
    let has_edges: Vec<bool> = {
        let mut flags = vec![false; modules.len()];
        for edge in index.derived.module_edges() {
            flags[edge.source as usize] = true;
            flags[edge.target as usize] = true;
        }
        flags
    };
    let mut by_layer: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for (id, module) in modules.iter().enumerate() {
        if has_edges[id] && included[id] {
            by_layer
                .entry(module.layer)
                .or_default()
                .push(index.string(module.name));
        }
    }
    if by_layer.len() < 2 {
        return Vec::new();
    }
    by_layer
        .into_iter()
        .rev()
        .map(|(layer, mut members)| {
            members.sort_unstable();
            MapFact::Layer {
                depth: layer,
                members: members.into_iter().map(str::to_owned).collect(),
            }
        })
        .collect()
}

fn externals(index: &Index, included: &[bool]) -> Vec<MapFact> {
    let mut uses: BTreeMap<&str, u32> = BTreeMap::new();
    for external in index.derived.externals() {
        if !included
            .get(external.module as usize)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        *uses.entry(index.string(external.name)).or_default() += external.uses;
    }
    let mut ordered: Vec<(&str, u32)> = uses.into_iter().collect();
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    ordered
        .into_iter()
        .map(|(name, count)| {
            let name = name.trim_start_matches("ext:");
            MapFact::External {
                name: name.to_owned(),
                uses: count,
            }
        })
        .collect()
}

/// Per variable: read count, definition sources and the first read location.
fn envs(index: &Index, filter: &Filter) -> Vec<MapFact> {
    struct Usage<'a> {
        reads: u32,
        sources: Vec<&'static str>,
        first: Option<(&'a str, u32)>,
    }
    let mut usages: BTreeMap<&str, Usage> = BTreeMap::new();
    for env in index.derived.envs() {
        if !filter.includes(index, env.node) {
            continue;
        }
        let usage = usages.entry(index.string(env.name)).or_insert(Usage {
            reads: 0,
            sources: Vec::new(),
            first: None,
        });
        if env.source == 0 {
            usage.reads += 1;
            let site = (index.path_of(env.node), env.line);
            if usage.first.is_none_or(|first| site < first) {
                usage.first = Some(site);
            }
        } else if let Some(source) = u8::try_from(env.source).ok().and_then(EnvSource::from_u8)
            && !usage.sources.contains(&source.id())
        {
            usage.sources.push(source.id());
        }
    }
    let mut ordered: Vec<(&str, Usage)> = usages.into_iter().collect();
    ordered.sort_by(|a, b| b.1.reads.cmp(&a.1.reads).then(a.0.cmp(b.0)));
    ordered
        .into_iter()
        .map(|(name, usage)| MapFact::Env {
            name: name.to_owned(),
            reads: usage.reads,
            sources: usage.sources.into_iter().map(str::to_owned).collect(),
            first: usage.first.map(|(path, line)| (path.to_owned(), line)),
        })
        .collect()
}
