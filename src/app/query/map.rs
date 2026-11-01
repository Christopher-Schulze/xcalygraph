//! `map`: orientation on an unknown tree (`docs/spec.md` section 7.1).

use std::collections::{BTreeMap, HashMap};

use super::output::{Value, Writer, json_object};
use super::{Filter, Index};
use crate::domain::facts::{EnvSource, FLAG_ENTRY, FLAG_GENERATED, FLAG_TEST, SymKind};
use crate::domain::graph::EdgeKind;
use crate::domain::role::Role;

const MODULE_LINES: usize = 40;
const SECTION_LINES: usize = 10;
const EXTERNAL_LINES: usize = 15;
const MODULE_DEPTH: usize = 3;

struct Section {
    keyword: &'static str,
    lines: Vec<(String, String)>,
    cut: usize,
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
            lines: vec![root_line(index, filter, root_name)],
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
    // Same bound as every writer: the truncation line is reserved, so the output never exceeds
    // the budget; when not even the root line fits, the answer is the truncation line alone.
    let limit =
        (budget * super::output::BYTES_PER_TOKEN).saturating_sub(super::output::footer_len(json));
    let keyword_size = |keyword: &str| -> usize {
        match (keyword.is_empty(), json) {
            (true, _) => 0,
            (false, true) => {
                json_object(&[("t", Value::Str("sec")), ("name", Value::Str(keyword))]).len()
            }
            (false, false) => keyword.len() + 1,
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
                        .map(|(text, record)| if json { record.len() } else { text.len() + 1 })
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
    let mut writer = Writer::new(json, usize::MAX / 4);
    let mut cut = 0;
    for section in &sections {
        if !section.keyword.is_empty() {
            writer.line(
                section.keyword,
                &[
                    ("t", Value::Str("sec")),
                    ("name", Value::Str(section.keyword)),
                ],
            );
        }
        for (text, record) in &section.lines {
            if json {
                writer.raw(record);
            } else {
                writer.line(text, &[]);
            }
        }
        cut += section.cut;
    }
    writer.skip(cut + dropped);
    writer.finish()
}

fn section(keyword: &'static str, mut lines: Vec<(String, String)>, cap: usize) -> Section {
    let cut = lines.len().saturating_sub(cap);
    lines.truncate(cap);
    Section {
        keyword,
        lines,
        cut,
    }
}

fn root_line(index: &Index, filter: &Filter, root_name: &str) -> (String, String) {
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
    let text = format!(
        "{root_name} {files} files {symbols} symbols {}",
        languages.join(" ")
    );
    let record = json_object(&[
        ("t", Value::Str("root")),
        ("name", Value::Str(root_name)),
        ("n", Value::Int(files)),
        ("sig", Value::Str(&languages.join(" "))),
    ]);
    (text.trim_end().to_owned(), record)
}

/// Module tree by path, depth max 3: deeper modules fold into their ancestor at depth 3.
fn modules(index: &Index, filter: &Filter) -> Vec<(String, String)> {
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
    let mut lines: Vec<(String, String)> = largest
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
            let text = format!(
                "{name} {}f {}s {}",
                entry.files,
                entry.symbols,
                top.join(" ")
            );
            let record = json_object(&[
                ("t", Value::Str("mod")),
                ("name", Value::Str(&name)),
                ("n", Value::Int(u64::from(entry.symbols))),
                ("sig", Value::Str(&top.join(" "))),
            ]);
            (text.trim_end().to_owned(), record)
        })
        .collect();
    if hidden > 0 {
        lines.push((
            format!("+{hidden} smaller modules"),
            json_object(&[("t", Value::Str("more")), ("n", Value::Int(hidden as u64))]),
        ));
    }
    lines
}

fn symbol_line(index: &Index, node: u32, rel: &str) -> (String, String) {
    let label = index.label(node);
    let path = index.path_of(node);
    let line = index.line_of(node);
    let text = if label == path {
        path.to_owned()
    } else {
        format!("{label} {path}:{line}")
    };
    let record = json_object(&[
        ("t", Value::Str("sym")),
        ("rel", Value::Str(rel)),
        ("name", Value::Str(&label)),
        ("path", Value::Str(path)),
        ("line", Value::Int(u64::from(line))),
    ]);
    (text, record)
}

fn entrypoints(index: &Index, filter: &Filter) -> Vec<(String, String)> {
    let mut lines: Vec<(String, String)> = Vec::new();
    for (id, symbol) in index.derived.symbols().iter().enumerate() {
        let task = matches!(symbol.kind(), SymKind::Task | SymKind::Svc);
        if symbol.flags & FLAG_ENTRY != 0 && !task && filter.symbol(index, symbol) {
            lines.push(symbol_line(index, id as u32, "entry"));
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
            lines.push(symbol_line(
                index,
                index.symbol_count() + id as u32,
                "entry",
            ));
        }
    }
    lines
}

/// Makefile targets, package scripts, CI jobs and services with what they run:
/// `<name> <path>:<line> > <target>, ...`; tasks running most first.
fn tasks(index: &Index, filter: &Filter) -> Vec<(String, String)> {
    let mut found: Vec<(usize, String, String)> = Vec::new();
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
            .map(|edge| index.label(edge.node))
            .collect();
        let (mut text, record) = symbol_line(index, node, "task");
        if !runs.is_empty() {
            text.push_str(" > ");
            text.push_str(&runs.join(", "));
        }
        found.push((runs.len(), text, record));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found
        .into_iter()
        .map(|(_, text, record)| (text, record))
        .collect()
}

/// API operations (OpenAPI `op`, Protobuf rpcs, GraphQL root fields) with their handlers:
/// `<name> [METHOD /path] <path>:<line> < <handler>, ...`; implemented operations first.
fn api(index: &Index, filter: &Filter) -> Vec<(String, String)> {
    use crate::domain::filetype::FileType;
    let symbols = index.derived.symbols();
    let mut found: Vec<(bool, String, String)> = Vec::new();
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
            .map(|edge| index.label(edge.node))
            .collect();
        let (mut text, record) = symbol_line(index, node, "api");
        if symbol.kind() == SymKind::Op {
            let signature = index.string(symbol.signature);
            if !signature.is_empty() && signature != index.string(symbol.name) {
                text = text.replacen(' ', &format!(" {signature} "), 1);
            }
        }
        if !handlers.is_empty() {
            text.push_str(" < ");
            text.push_str(&handlers.join(", "));
        }
        found.push((handlers.is_empty(), text, record));
    }
    found.sort();
    found
        .into_iter()
        .map(|(_, text, record)| (text, record))
        .collect()
}

fn hubs(index: &Index, filter: &Filter) -> Vec<(String, String)> {
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
        .map(|(_, node)| symbol_line(index, node, "hub"))
        .collect()
}

/// Per module: whether any of its files passes the filter. Module-level sections (cycles,
/// layers, externals) show only these modules, so a path restriction never exposes others.
fn included_modules(index: &Index, filter: &Filter) -> Vec<bool> {
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

fn cycles(index: &Index, included: &[bool]) -> Vec<(String, String)> {
    let mut groups: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for (id, module) in index.derived.modules().iter().enumerate() {
        if !included[id] {
            continue;
        }
        groups
            .entry(module.cycle)
            .or_default()
            .push(index.string(module.name));
    }
    let mut lines: Vec<(String, String)> = groups
        .into_values()
        .filter(|members| members.len() > 1)
        .map(|mut members| {
            members.sort_unstable();
            let text = members.join(" <-> ");
            let record = json_object(&[
                ("t", Value::Str("cycle")),
                ("name", Value::Str(&text)),
                ("n", Value::Int(members.len() as u64)),
            ]);
            (text, record)
        })
        .collect();
    lines.sort();
    lines
}

/// Modules per layer, top layer first; layer 0 has no dependencies inside the root.
fn layers(index: &Index, included: &[bool]) -> Vec<(String, String)> {
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
            let text = format!("{layer} {}", members.join(" "));
            let record = json_object(&[
                ("t", Value::Str("layer")),
                ("n", Value::Int(u64::from(layer))),
                ("name", Value::Str(&members.join(" "))),
            ]);
            (text, record)
        })
        .collect()
}

fn externals(index: &Index, included: &[bool]) -> Vec<(String, String)> {
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
            let record = json_object(&[
                ("t", Value::Str("ext")),
                ("name", Value::Str(name)),
                ("n", Value::Int(u64::from(count))),
            ]);
            (format!("{name} {count}"), record)
        })
        .collect()
}

/// Per variable: read count, definition sources and the first read location.
fn envs(index: &Index, filter: &Filter) -> Vec<(String, String)> {
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
        .map(|(name, usage)| {
            let sources = if usage.sources.is_empty() {
                "-".to_owned()
            } else {
                usage.sources.join(",")
            };
            let first = usage
                .first
                .map_or_else(String::new, |(path, line)| format!(" {path}:{line}"));
            let text = format!("{name} {}r {sources}{first}", usage.reads);
            let record = json_object(&[
                ("t", Value::Str("env")),
                ("name", Value::Str(name)),
                ("n", Value::Int(u64::from(usage.reads))),
                ("sig", Value::Str(&sources)),
            ]);
            (text, record)
        })
        .collect()
}
