//! `impact`: changed and affected symbols, tests and run commands (`docs/spec.md` section 7.5).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use super::output::{Value, Writer};
use super::{Filter, Index, Node};
use crate::app::error::AppError;
use crate::domain::corpus::FileRecord as CorpusRecord;
use crate::domain::facts::{FLAG_TEST, Facts, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::graph::EdgeKind;
use crate::extract;
use crate::infra::git;

const AFFECTING: [EdgeKind; 6] = [
    EdgeKind::Calls,
    EdgeKind::Uses,
    EdgeKind::Inherits,
    EdgeKind::Implements,
    EdgeKind::References,
    EdgeKind::Queries,
];

/// Lines per section; tests and run commands are never displaced by long change lists.
const CHANGED_LINES: usize = 15;
const AFFECTED_LINES: usize = 25;
const TEST_LINES: usize = 15;
/// Test names per run command before it runs the whole package or crate instead.
const RUN_NAMES: usize = 20;
/// Run commands shown; a change reaching more packages needs a full test run anyway.
const RUN_LINES: usize = 8;

/// A changed symbol or file and what changed about it.
struct Target {
    node: u32,
    /// `sig` for an exported signature change, `new` for a symbol absent at the revision,
    /// `new N` for a new file with N symbols.
    note: String,
    /// Further BFS seeds summarized by this line (the symbols of a new file).
    seeds: Vec<u32>,
}

pub struct ImpactOptions<'a> {
    pub root: &'a Path,
    pub depth: usize,
    pub json: bool,
    pub budget: usize,
}

/// Targets from NAME arguments.
pub fn named(
    index: &Index,
    filter: &Filter,
    names: &[String],
    json: bool,
) -> Result<Vec<u32>, AppError> {
    names
        .iter()
        .map(|name| super::names::resolve_one(index, filter, name, json))
        .collect()
}

pub fn render_names(
    index: &Index,
    filter: &Filter,
    targets: Vec<u32>,
    options: &ImpactOptions,
) -> String {
    let targets: Vec<Target> = targets
        .into_iter()
        .map(|node| Target {
            node,
            note: String::new(),
            seeds: Vec::new(),
        })
        .collect();
    render(index, filter, &targets, &[], options)
}

/// Targets from `git diff REV` (work tree included) with exported signature changes.
pub fn render_diff(
    index: &Index,
    filter: &Filter,
    rev: &str,
    options: &ImpactOptions,
) -> Result<String, AppError> {
    let diffs = git::diff(options.root, rev)
        .ok_or_else(|| AppError::Invalid(format!("git diff against '{rev}' unavailable")))?;
    let mut targets: Vec<Target> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    for diff in &diffs {
        let Some(file) = index
            .derived
            .files()
            .iter()
            .position(|file| index.string(file.path) == diff.path)
            .map(|position| position as u32)
        else {
            continue;
        };
        let record = &index.derived.files()[file as usize];
        if !filter.file(index, record) {
            continue;
        }
        let symbols: Vec<u32> =
            (record.first_symbol..record.first_symbol + record.symbol_count).collect();
        let file_node = index.symbol_count() + file;
        let old = (!diff.untracked)
            .then(|| old_signatures(options.root, rev, &diff.path, record.file_type()))
            .flatten();
        if diff.untracked || old.is_none() {
            // A file absent at the revision is one line; its symbols still seed the
            // dependents search.
            targets.push(Target {
                node: file_node,
                note: format!("new {}", symbols.len()),
                seeds: symbols,
            });
            continue;
        }
        let hit = innermost_hits(index, &symbols, &diff.ranges);
        for &node in &hit {
            let note = match (&old, index.node(node)) {
                (Some(old), Node::Symbol(symbol)) => match old.get(&symbol_key(index, node)) {
                    None => "new",
                    Some(signature)
                        if symbol.exported != 0 && signature != index.string(symbol.signature) =>
                    {
                        "sig"
                    }
                    Some(_) => "",
                },
                _ => "",
            };
            targets.push(Target {
                node,
                note: note.to_owned(),
                seeds: Vec::new(),
            });
        }
        if hit.is_empty() {
            targets.push(Target {
                node: file_node,
                note: String::new(),
                seeds: Vec::new(),
            });
        }
        if let Some(old) = &old {
            let current: BTreeSet<String> = symbols.iter().map(|&s| symbol_key(index, s)).collect();
            for (key, _) in old.iter().filter(|(key, _)| !current.contains(*key)) {
                removed.push(format!("{} {}", key.replace('\u{0}', "."), diff.path));
            }
        }
    }
    Ok(render(index, filter, &targets, &removed, options))
}

/// Symbols whose range intersects a changed range; an enclosing symbol is dropped when one of
/// its children is hit.
fn innermost_hits(index: &Index, symbols: &[u32], ranges: &[(u32, u32)]) -> Vec<u32> {
    let all = index.derived.symbols();
    let hit: Vec<u32> = symbols
        .iter()
        .copied()
        .filter(|&s| {
            let symbol = &all[s as usize];
            symbol.kind() != SymKind::Mod
                && ranges
                    .iter()
                    .any(|&(start, end)| symbol.line <= end && start <= symbol.end_line)
        })
        .collect();
    hit.iter()
        .copied()
        .filter(|&s| {
            !hit.iter()
                .any(|&other| other != s && all[other as usize].parent == s)
        })
        .collect()
}

/// Name path of a symbol inside its file, for matching with the revision's facts.
fn symbol_key(index: &Index, node: u32) -> String {
    let symbols = index.derived.symbols();
    let mut parts = Vec::new();
    let mut current = node;
    loop {
        let symbol = &symbols[current as usize];
        parts.push(index.string(symbol.name).to_owned());
        if symbol.parent == u32::MAX {
            break;
        }
        current = symbol.parent;
    }
    parts.reverse();
    parts.join("\u{0}")
}

/// Signatures of every symbol of the file at `rev`, keyed like [`symbol_key`].
fn old_signatures(
    root: &Path,
    rev: &str,
    path: &str,
    file_type: Option<FileType>,
) -> Option<BTreeMap<String, String>> {
    let content = git::show(root, rev, path)?;
    let record = CorpusRecord {
        path: path.to_owned(),
        size: content.len() as u64,
        mtime_ns: 0,
        hash: 0,
        role: crate::domain::role::Role::Src,
        file_type,
    };
    let facts: Facts = extract::extract(&record, &content);
    let mut map = BTreeMap::new();
    for symbol in &facts.symbols {
        let mut parts = vec![symbol.name.clone()];
        let mut current = symbol.parent;
        while let Some(parent) = current {
            let enclosing = &facts.symbols[parent as usize];
            parts.push(enclosing.name.clone());
            current = enclosing.parent;
        }
        parts.reverse();
        map.insert(parts.join("\u{0}"), symbol.signature.clone());
    }
    Some(map)
}

fn render(
    index: &Index,
    filter: &Filter,
    targets: &[Target],
    removed: &[String],
    options: &ImpactOptions,
) -> String {
    let mut writer = Writer::new(options.json, options.budget);
    let target_nodes: BTreeSet<u32> = targets
        .iter()
        .flat_map(|target| std::iter::once(target.node).chain(target.seeds.iter().copied()))
        .collect();
    // Code changes first; config, docs and other files after.
    let mut ordered: Vec<&Target> = targets.iter().collect();
    ordered.sort_by_key(|target| !matches!(index.node(target.node), Node::Symbol(_)));
    for (shown, target) in ordered.iter().enumerate() {
        if shown >= CHANGED_LINES {
            more(&mut writer, ordered.len() - shown, "changed");
            break;
        }
        let label = index.label(target.node);
        let path = index.path_of(target.node);
        let line = index.line_of(target.node);
        let mut text = format!("~ {label} {path}:{line}");
        if !target.note.is_empty() {
            text.push(' ');
            text.push_str(&target.note);
        }
        writer.line(
            &text,
            &[
                ("t", Value::Str("sym")),
                ("rel", Value::Str("chg")),
                ("name", Value::Str(&label)),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(line))),
                ("kind", Value::Str(&target.note)),
            ],
        );
    }
    for gone in removed {
        writer.line(
            &format!("~ {gone} removed"),
            &[
                ("t", Value::Str("sym")),
                ("rel", Value::Str("chg")),
                ("name", Value::Str(gone)),
                ("kind", Value::Str("removed")),
            ],
        );
    }
    // Reverse BFS over dependent edges, ordered by depth, then rank. A dependent is exact only
    // when some shortest chain to a target consists of exact edges; within one depth an exact
    // chain wins over an inferred one found earlier.
    let mut depth_of: HashMap<u32, usize> = target_nodes.iter().map(|&node| (node, 0)).collect();
    let mut exact_of: HashMap<u32, bool> = target_nodes.iter().map(|&node| (node, true)).collect();
    let mut frontier: Vec<u32> = target_nodes.iter().copied().collect();
    let mut affected: Vec<(usize, u32, bool)> = Vec::new();
    let mut tests: BTreeSet<u32> = BTreeSet::new();
    for depth in 1..=options.depth.max(1) {
        let mut layer = Vec::new();
        for &node in &frontier {
            let chain_exact = exact_of[&node];
            for edge in index.derived.incoming(node) {
                if edge.kind() == EdgeKind::Tests && filter.edge(edge) {
                    tests.insert(edge.node);
                    continue;
                }
                if !AFFECTING.contains(&edge.kind()) || !filter.edge(edge) {
                    continue;
                }
                let exact = chain_exact && edge.is_exact();
                match depth_of.get(&edge.node) {
                    Some(&seen) if seen < depth => {}
                    Some(_) => {
                        if exact {
                            exact_of.insert(edge.node, true);
                        }
                    }
                    None => {
                        depth_of.insert(edge.node, depth);
                        exact_of.insert(edge.node, exact);
                        layer.push(edge.node);
                    }
                }
            }
        }
        let mut next = Vec::new();
        for node in layer {
            if is_test(index, node) {
                tests.insert(node);
                continue;
            }
            if filter.node(index, node) {
                affected.push((depth, node, exact_of[&node]));
            }
            next.push(node);
        }
        frontier = next;
    }
    let rank = |node: u32| match index.node(node) {
        Node::Symbol(symbol) => symbol.rank,
        Node::File(_) => 0.0,
    };
    affected.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(rank(b.1).total_cmp(&rank(a.1)))
            .then(a.1.cmp(&b.1))
    });
    for (shown, (depth, node, exact)) in affected.iter().enumerate() {
        if shown >= AFFECTED_LINES {
            more(&mut writer, affected.len() - shown, "affected");
            break;
        }
        let label = index.label(*node);
        let path = index.path_of(*node);
        let line = index.line_of(*node);
        let mark = if *exact { "" } else { "?" };
        writer.line(
            &format!("< {depth} {label} {path}:{line}{mark}"),
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("in")),
                ("name", Value::Str(&label)),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(line))),
                ("n", Value::Int(*depth as u64)),
                (
                    "conf",
                    Value::Str(if *exact { "exact" } else { "inferred" }),
                ),
            ],
        );
    }
    for (shown, &test) in tests.iter().enumerate() {
        if shown >= TEST_LINES {
            more(&mut writer, tests.len() - shown, "tests");
            break;
        }
        let label = index.label(test);
        let path = index.path_of(test);
        let line = index.line_of(test);
        writer.line(
            &format!("t {label} {path}:{line}"),
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("test")),
                ("name", Value::Str(&label)),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(line))),
            ],
        );
    }
    let commands = run_lines(index, options.root, &tests);
    for (shown, command) in commands.iter().enumerate() {
        if shown >= RUN_LINES {
            more(&mut writer, commands.len() - shown, "run commands");
            break;
        }
        writer.force_line(
            &format!("r {command}"),
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("run")),
                ("name", Value::Str(command)),
            ],
        );
    }
    writer.finish()
}

fn more(writer: &mut Writer, count: usize, what: &str) {
    writer.line(
        &format!("+{count} more {what}"),
        &[
            ("t", Value::Str("more")),
            ("name", Value::Str(what)),
            ("n", Value::Int(count as u64)),
        ],
    );
}

fn is_test(index: &Index, node: u32) -> bool {
    match index.node(node) {
        Node::Symbol(symbol) => symbol.flags & FLAG_TEST != 0,
        Node::File(file) => file.role() == crate::domain::role::Role::Test,
    }
}

/// Commands that run exactly the selected tests: Go per package, Rust per crate, JVM test classes
/// per Gradle or Maven project, test files otherwise (section 7.5).
fn run_lines(index: &Index, root: &Path, tests: &BTreeSet<u32>) -> Vec<String> {
    let mut go: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut rust: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // (build tool, project directory) -> test classes.
    let mut jvm: BTreeMap<(JvmBuild, String), BTreeSet<String>> = BTreeMap::new();
    // .NET project directory -> fully qualified test names.
    let mut dotnet: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // SwiftPM package directory -> test classes.
    let mut swift: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // Runner prefix -> test file paths (RSpec, Minitest, ExUnit, Dart), PHPUnit test names.
    let mut by_path: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut phpunit: BTreeSet<String> = BTreeSet::new();
    let mut zig = false;
    let mut files: BTreeSet<String> = BTreeSet::new();
    let paths: Vec<&str> = index
        .derived
        .files()
        .iter()
        .map(|file| index.string(file.path))
        .collect();
    for &test in tests {
        let path = index.path_of(test);
        let runner = if path.ends_with("_spec.rb") {
            Some("bundle exec rspec")
        } else if path.ends_with(".rb") {
            Some("bundle exec ruby -Itest")
        } else if path.ends_with(".exs") {
            Some("mix test")
        } else if path.ends_with(".dart") {
            Some("dart test")
        } else {
            None
        };
        if let Some(runner) = runner {
            by_path.entry(runner).or_default().insert(path.to_owned());
            continue;
        }
        if path.ends_with(".zig") {
            zig = true;
            continue;
        }
        if path.ends_with(".php")
            && let Node::Symbol(symbol) = index.node(test)
            && index.string(symbol.name).starts_with("test")
        {
            phpunit.insert(index.string(symbol.name).to_owned());
            continue;
        }
        let Node::Symbol(symbol) = index.node(test) else {
            files.insert(index.path_of(test).to_owned());
            continue;
        };
        let path = index.path_of(test);
        let name = index.string(symbol.name).to_owned();
        match index.file_of(test).file_type() {
            Some(FileType::Go) if symbol.kind() == SymKind::Fn && name.starts_with("Test") => {
                let dir = parent(path);
                let module = nearest(&paths, dir, "go.mod").unwrap_or_default();
                let relative = dir
                    .strip_prefix(module.as_str())
                    .unwrap_or(dir)
                    .trim_start_matches('/');
                go.entry((module, relative.to_owned()))
                    .or_default()
                    .insert(name);
            }
            Some(FileType::Rust) if symbol.kind() == SymKind::Fn => {
                let manifest = nearest(&paths, parent(path), "Cargo.toml");
                let crate_name = manifest
                    .and_then(|dir| {
                        let file = if dir.is_empty() {
                            "Cargo.toml".to_owned()
                        } else {
                            format!("{dir}/Cargo.toml")
                        };
                        let text = std::fs::read_to_string(root.join(file)).ok()?;
                        extract::manifest::parse("Cargo.toml", &text)?.module
                    })
                    .unwrap_or_default();
                rust.entry(crate_name).or_default().insert(name);
            }
            Some(FileType::Java | FileType::Kotlin | FileType::Scala | FileType::Groovy)
                if symbol.parent != u32::MAX =>
            {
                let class = index.qualified(&index.derived.symbols()[symbol.parent as usize]);
                let dir = parent(path);
                let build = ["build.gradle", "build.gradle.kts"]
                    .iter()
                    .find_map(|manifest| nearest(&paths, dir, manifest))
                    .map(|project| (JvmBuild::Gradle, project))
                    .or_else(|| {
                        nearest(&paths, dir, "pom.xml").map(|project| (JvmBuild::Maven, project))
                    });
                match build {
                    Some(key) => {
                        jvm.entry(key).or_default().insert(class.into_owned());
                    }
                    None => {
                        files.insert(path.to_owned());
                    }
                }
            }
            Some(FileType::Swift) if symbol.parent != u32::MAX => {
                let class = index
                    .string(index.derived.symbols()[symbol.parent as usize].name)
                    .to_owned();
                match nearest(&paths, parent(path), "Package.swift") {
                    Some(package) => {
                        swift.entry(package).or_default().insert(class);
                    }
                    None => {
                        files.insert(path.to_owned());
                    }
                }
            }
            Some(FileType::CSharp | FileType::VbNet) => {
                let qualified = index.qualified(symbol).into_owned();
                match nearest_project(&paths, parent(path)) {
                    Some(project) => {
                        dotnet.entry(project).or_default().insert(qualified);
                    }
                    None => {
                        files.insert(path.to_owned());
                    }
                }
            }
            _ => {
                files.insert(path.to_owned());
            }
        }
    }
    let mut lines = Vec::new();
    for ((module, package), names) in go {
        let names: Vec<String> = names.into_iter().collect();
        let package = if package.is_empty() {
            ".".to_owned()
        } else {
            format!("./{package}")
        };
        let prefix = if module.is_empty() {
            "go".to_owned()
        } else {
            format!("go -C {module}")
        };
        // Beyond a handful of names the whole package is the shorter and safer command.
        if names.len() > RUN_NAMES {
            lines.push(format!("{prefix} test {package}"));
        } else {
            lines.push(format!(
                "{prefix} test {package} -run '^({})$'",
                names.join("|")
            ));
        }
    }
    for (crate_name, names) in rust {
        let names: Vec<String> = names.into_iter().collect();
        let package = if crate_name.is_empty() {
            String::new()
        } else {
            format!(" -p {crate_name}")
        };
        if names.len() > RUN_NAMES {
            lines.push(format!("cargo test{package}"));
        } else {
            lines.push(format!("cargo test{package} -- {}", names.join(" ")));
        }
    }
    for ((build, project), classes) in jvm {
        let classes: Vec<String> = classes.into_iter().collect();
        lines.push(match build {
            JvmBuild::Gradle => {
                let project = if project.is_empty() {
                    String::new()
                } else {
                    format!(" -p {project}")
                };
                let filters: Vec<String> = classes
                    .iter()
                    .map(|class| format!("--tests {class}"))
                    .collect();
                format!("./gradlew{project} test {}", filters.join(" "))
            }
            JvmBuild::Maven => {
                let project = if project.is_empty() {
                    String::new()
                } else {
                    format!(" -f {project}/pom.xml")
                };
                format!("mvn{project} test -Dtest={}", classes.join(","))
            }
        });
    }
    for (package, classes) in swift {
        let package = if package.is_empty() {
            String::new()
        } else {
            format!(" --package-path {package}")
        };
        let classes: Vec<String> = classes.into_iter().collect();
        lines.push(format!(
            "swift test{package} --filter '{}'",
            classes.join("|")
        ));
    }
    for (project, names) in dotnet {
        let project = if project.is_empty() {
            String::new()
        } else {
            format!(" {project}")
        };
        let filter: Vec<String> = names
            .iter()
            .map(|name| format!("FullyQualifiedName~{name}"))
            .collect();
        lines.push(format!(
            "dotnet test{project} --filter \"{}\"",
            filter.join("|")
        ));
    }
    for (runner, paths) in by_path {
        let paths: Vec<String> = paths.into_iter().collect();
        if runner == "bundle exec ruby -Itest" {
            // Minitest runs one file per process.
            lines.extend(paths.iter().map(|path| format!("{runner} {path}")));
        } else {
            lines.push(format!("{runner} {}", paths.join(" ")));
        }
    }
    if !phpunit.is_empty() {
        let names: Vec<String> = phpunit.into_iter().collect();
        lines.push(format!("vendor/bin/phpunit --filter '{}'", names.join("|")));
    }
    if zig {
        lines.push("zig build test".to_owned());
    }
    lines.extend(files);
    lines
}

/// Directory of the nearest `.csproj`, `.vbproj` or `.fsproj` at or above `dir`.
fn nearest_project(paths: &[&str], dir: &str) -> Option<String> {
    let mut current = dir.to_owned();
    loop {
        let prefix = if current.is_empty() {
            String::new()
        } else {
            format!("{current}/")
        };
        let start = paths.partition_point(|path| *path < prefix.as_str());
        let found = paths[start..]
            .iter()
            .take_while(|path| path.starts_with(&prefix))
            .any(|path| {
                let rest = &path[prefix.len()..];
                !rest.contains('/')
                    && [".csproj", ".vbproj", ".fsproj"]
                        .iter()
                        .any(|suffix| rest.ends_with(suffix))
            });
        if found {
            return Some(current);
        }
        if current.is_empty() {
            return None;
        }
        current = parent(&current).to_owned();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum JvmBuild {
    Gradle,
    Maven,
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Directory of the nearest `manifest` at or above `dir`.
fn nearest(paths: &[&str], dir: &str, manifest: &str) -> Option<String> {
    let mut current = dir.to_owned();
    loop {
        let candidate = if current.is_empty() {
            manifest.to_owned()
        } else {
            format!("{current}/{manifest}")
        };
        if paths.binary_search(&candidate.as_str()).is_ok() {
            return Some(current);
        }
        if current.is_empty() {
            return None;
        }
        current = parent(&current).to_owned();
    }
}
