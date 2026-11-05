//! Resolution golden tests: every directory under `tests/fixtures/resolve/` is a small root;
//! its edges are compared with `<case>.edges`. `XCALYGRAPH_BLESS=1` rewrites them; every
//! blessed edge must be checked against the fixture source before commit.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as IoWrite};
use std::path::{Path, PathBuf};

use super::{build, static_modifier, swift_source_identity_matches};
use crate::domain::corpus::FileRecord;
use crate::domain::filetype::FileType;
use crate::domain::graph::{Confidence, Edge, EdgeKind, Graph};
use crate::domain::role::{self, FileFacts};
use crate::extract::extract;
use sha2::{Digest, Sha256};

mod precision_tools;

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, root, out)?;
        } else {
            let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
            out.push((
                relative.to_string_lossy().replace('\\', "/"),
                fs::read(&path)?,
            ));
        }
    }
    Ok(())
}

pub fn graph_of(root: &Path) -> std::io::Result<Graph> {
    let mut sources = Vec::new();
    walk(root, root, &mut sources)?;
    sources.sort();
    let mut files = Vec::new();
    let mut facts = Vec::new();
    for (path, content) in &sources {
        let classification = role::classify(&FileFacts {
            path,
            size: content.len() as u64,
            head: Some(&content[..content.len().min(role::HEAD_LEN)]),
            cpp_sibling: false,
        });
        let record = FileRecord {
            path: path.clone(),
            size: content.len() as u64,
            mtime_ns: 0,
            identity: None,
            hash: 0,
            role: classification.role,
            file_type: classification.file_type,
        };
        facts.push(
            record
                .role
                .is_extracted()
                .then(|| extract(&record, content)),
        );
        files.push(record);
    }
    Ok(build(&files, facts))
}

pub fn render(graph: &Graph) -> String {
    let name = |node: u32| -> String {
        let symbols = graph.symbols.len() as u32;
        if node < symbols {
            graph.symbols[node as usize].qualified.clone()
        } else {
            format!("file:{}", graph.files[(node - symbols) as usize].path)
        }
    };
    let mut lines: Vec<String> = graph
        .edges
        .iter()
        .map(|edge| {
            let mark = if edge.confidence == Confidence::Inferred {
                "?"
            } else {
                ""
            };
            format!(
                "{} -{}{}-> {} @{}",
                name(edge.source),
                edge.kind.id(),
                mark,
                name(edge.target),
                edge.line
            )
        })
        .collect();
    lines.sort();
    let mut out = lines.join("\n");
    out.push('\n');
    for file in &graph.files {
        if file.ambiguous > 0 {
            let _ = writeln!(out, "ambiguous {} {}", file.path, file.ambiguous);
        }
        if file.entry {
            let _ = writeln!(out, "entry {}", file.path);
        }
    }
    for external in &graph.externals {
        let _ = writeln!(
            out,
            "ext {} {} {}",
            graph.modules[external.module as usize].name, external.name, external.uses
        );
    }
    out
}

fn cases() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolve");
    let mut cases: Vec<PathBuf> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    cases
}

#[test]
fn golden_edges_match() -> Result<(), Box<dyn std::error::Error>> {
    let bless = std::env::var_os("XCALYGRAPH_BLESS").is_some();
    let mut failures = Vec::new();
    let all = cases();
    assert!(!all.is_empty(), "no resolution fixtures");
    for case in all {
        let actual = render(&graph_of(&case)?);
        let golden = PathBuf::from(format!("{}.edges", case.display()));
        if bless {
            fs::write(&golden, &actual)?;
            continue;
        }
        let expected = fs::read_to_string(&golden).unwrap_or_default();
        if expected != actual {
            failures.push(format!(
                "{}\n--- expected\n{expected}--- actual\n{actual}",
                case.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "golden mismatches:\n{}",
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn derived_bytes_are_identical_across_thread_counts() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolve");
    let build_with = |threads: usize| -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error>> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()?;
        let mut images = Vec::new();
        for case in cases() {
            let graph = pool.install(|| graph_of(&case))?;
            images.push(crate::infra::store::derived::build(&graph));
        }
        Ok(images)
    };
    assert!(root.is_dir());
    let single = build_with(1)?;
    let parallel = build_with(8)?;
    assert_eq!(single.len(), parallel.len());
    for (left, right) in single.iter().zip(&parallel) {
        assert!(
            left == right,
            "derived image differs between 1 and 8 threads"
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct NodeLocation<'a> {
    node: u32,
    file: u32,
    path: &'a str,
    line: u32,
    name: &'a str,
    signature: &'a str,
    language: &'a str,
    node_kind: &'static str,
}

fn node_location(graph: &Graph, node: u32) -> Option<NodeLocation<'_>> {
    let symbol_count = graph.symbols.len() as u32;
    if node < symbol_count {
        let symbol = graph.symbols.get(node as usize)?;
        let file = graph.files.get(symbol.file as usize)?;
        Some(NodeLocation {
            node,
            file: symbol.file,
            path: &file.path,
            line: symbol.line,
            name: &symbol.qualified,
            signature: &symbol.signature,
            language: file.file_type.map_or("unknown", FileType::id),
            node_kind: "symbol",
        })
    } else {
        let file_index = node.checked_sub(symbol_count)?;
        let file = graph.files.get(file_index as usize)?;
        Some(NodeLocation {
            node,
            file: file_index,
            path: &file.path,
            line: 1,
            name: &file.path,
            signature: "",
            language: file.file_type.map_or("unknown", FileType::id),
            node_kind: "file",
        })
    }
}

#[test]
fn javascript_unknown_receiver_reaches_imported_methods_only_as_inferred() -> io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("javascript-unknown-render")?;
    fs::write(
        root.path().join("index.js"),
        "var GithubView = require('./github-view');\n\
         var app = {};\n\
         app.get('/', function (req, res) {\n\
           res.render('page.md');\n\
         });\n",
    )?;
    fs::write(
        root.path().join("github-view.js"),
        "function GithubView(name) {}\n\
         GithubView.prototype.render = function render(options, callback) {};\n\
         module.exports = GithubView;\n",
    )?;
    let graph = graph_of(root.path())?;
    // `res` has no receiver evidence: the unique method among imported modules is a candidate
    // (inferred, `?`), never exact, so `--exact` drops it.
    let render_edges: Vec<Confidence> = graph
        .edges
        .iter()
        .filter(|edge| {
            edge.kind == EdgeKind::Calls
                && node_location(&graph, edge.source)
                    .is_some_and(|source| source.path == "index.js")
                && node_location(&graph, edge.target).is_some_and(|target| {
                    target.path == "github-view.js" && target.name.ends_with(".GithubView.render")
                })
        })
        .map(|edge| edge.confidence)
        .collect();
    assert_eq!(
        render_edges,
        [Confidence::Inferred],
        "expected one inferred candidate edge in fixture:\n{}",
        render(&graph)
    );
    Ok(())
}

#[test]
fn javascript_and_typescript_class_calls_select_static_methods() -> io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("javascript-static-instance-methods")?;
    fs::write(
        root.path().join("index.js"),
        "import { HeaderBag } from './headers.js';\n\
         HeaderBag.concat({}, {});\n",
    )?;
    fs::write(
        root.path().join("headers.js"),
        "export class HeaderBag {\n\
           static concat(first, ...targets) { return first; }\n\
           concat(...targets) { return this.constructor.concat(this, ...targets); }\n\
           callInstance() { this.concat({}); }\n\
         }\n",
    )?;
    fs::write(
        root.path().join("index.ts"),
        "import { TypedHeaderBag } from './typed-headers';\n\
         TypedHeaderBag.concat({}, {});\n\
         function useInstance(headerBag: TypedHeaderBag) {\n\
           headerBag.concat({});\n\
         }\n",
    )?;
    fs::write(
        root.path().join("typed-headers.ts"),
        "export class TypedHeaderBag {\n\
           static concat(first: object, ...targets: object[]) { return first; }\n\
           concat(...targets: object[]) { return this.constructor.concat(this, ...targets); }\n\
           callInstance() { this.concat({}); }\n\
         }\n",
    )?;
    let graph = graph_of(root.path())?;
    let target_lines = |source_path: &str, target_path: &str, source_line: u32| {
        let mut targets: Vec<u32> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls && edge.line == source_line)
            .filter_map(|edge| {
                let source = node_location(&graph, edge.source)?;
                let target = node_location(&graph, edge.target)?;
                (source.path == source_path && target.path == target_path).then_some(target.line)
            })
            .collect();
        targets.sort_unstable();
        targets
    };
    assert_eq!(
        target_lines("index.js", "headers.js", 2),
        vec![2],
        "JavaScript class call must select only the static method:\n{}",
        render(&graph)
    );
    assert_eq!(
        target_lines("index.ts", "typed-headers.ts", 2),
        vec![2],
        "TypeScript class call must select only the static method:\n{}",
        render(&graph)
    );
    assert_eq!(
        target_lines("headers.js", "headers.js", 4),
        vec![3],
        "JavaScript instance call must keep its instance method target:\n{}",
        render(&graph)
    );
    assert_eq!(
        target_lines("typed-headers.ts", "typed-headers.ts", 4),
        vec![3],
        "TypeScript instance call must keep its instance method target:\n{}",
        render(&graph)
    );
    assert_eq!(
        target_lines("index.ts", "typed-headers.ts", 4),
        vec![3],
        "TypeScript typed receiver must select only its instance method:\n{}",
        render(&graph)
    );
    Ok(())
}

#[test]
fn typescript_string_match_does_not_fall_back_to_router_match() -> io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("typescript-string-match")?;
    fs::write(
        root.path().join("index.ts"),
        "import type { Router } from './router';\n\
         import { METHOD_NAME_ALL } from './router';\n\
         declare const router: Router;\n\
         router.match(METHOD_NAME_ALL, '/');\n\
         class Node {\n\
           search(restPathString: string): void {\n\
             const matcher = /x/;\n\
             const m = matcher.exec(restPathString);\n\
             if (m) m[0].match(/\\//g);\n\
           }\n\
         }\n",
    )?;
    fs::write(
        root.path().join("router.ts"),
        "export const METHOD_NAME_ALL = 'ALL' as const;\n\
         export interface Router {\n\
           match(method: string, path: string): boolean;\n\
         }\n",
    )?;
    let graph = graph_of(root.path())?;
    let targets_at = |line| {
        let mut targets: Vec<u32> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls && edge.line == line)
            .filter_map(|edge| {
                let source = node_location(&graph, edge.source)?;
                let target = node_location(&graph, edge.target)?;
                (source.path == "index.ts" && target.path == "router.ts").then_some(target.line)
            })
            .collect();
        targets.sort_unstable();
        targets
    };
    assert_eq!(
        targets_at(4),
        vec![3],
        "typed router call must keep its imported interface target:\n{}",
        render(&graph)
    );
    assert_eq!(
        targets_at(9),
        Vec::<u32>::new(),
        "String.match must not target the unrelated Router method:\n{}",
        render(&graph)
    );
    Ok(())
}

#[test]
fn python_receivers_flow_through_packages_context_managers_and_properties() -> io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("python-receivers")?;
    fs::create_dir_all(root.path().join("pkg"))?;
    fs::write(root.path().join("pkg/__init__.py"), "from ._m import *\n")?;
    fs::write(
        root.path().join("pkg/_m.py"),
        "class Url:\n    def path(self):\n        return 1\n\n\
         class Foo:\n    def run(self) -> Url:\n        return Url()\n\n\
         \x20   @property\n    def url(self) -> Url:\n        return Url()\n",
    )?;
    fs::write(
        root.path().join("use.py"),
        "import pkg\n\n\
         def with_block():\n    with pkg.Foo() as f:\n        f.run()\n\n\
         def assigned():\n    g = pkg.Foo()\n    g.run()\n\n\
         def chained():\n    g = pkg.Foo()\n    g.run().path()\n\n\
         def through_property():\n    g = pkg.Foo()\n    g.url.path()\n",
    )?;
    let graph = graph_of(root.path())?;
    let text = render(&graph);
    for expected in [
        "use.with_block -> pkg._m.Foo.run",
        "use.assigned -> pkg._m.Foo.run",
        "use.chained -> pkg._m.Url.path",
        "use.through_property -> pkg._m.Url.path",
    ] {
        assert!(
            text.lines().any(|line| {
                let (source, target) = expected.split_once(" -> ").unwrap_or((expected, ""));
                line.contains(source) && line.contains(target)
            }),
            "{expected}\n{text}"
        );
    }
    Ok(())
}

#[test]
fn receivers_flow_through_return_types_getters_and_private_fields() -> io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("receiver-chains")?;
    fs::write(
        root.path().join("lib.rs"),
        "pub struct Builder;\npub struct Walk;\n\
         impl Builder {\n    pub fn new() -> Builder { Builder }\n\
         \x20   pub fn depth(&mut self) -> &mut Builder { self }\n\
         \x20   pub fn build(&self) -> Result<Walk, String> { Ok(Walk) }\n}\n\
         impl Walk {\n    pub fn run(&self) {}\n}\n\
         pub fn chained() {\n    Builder::new().depth().depth().build().unwrap().run();\n}\n",
    )?;
    fs::write(
        root.path().join("a.ts"),
        "export class Item { insert(): void {} }\n\
         export async function make(): Promise<Item> { return new Item() }\n\
         export class Router {\n  #node: Item = new Item()\n  get y(): Item { return this.#node }\n\
         \x20 viaPrivate() { this.#node.insert() }\n  viaGetter() { this.y.insert() }\n}\n\
         export async function awaited() { const res = await make(); res.insert() }\n",
    )?;
    let text = render(&graph_of(root.path())?);
    for (source, target) in [
        ("chained", "Walk::run"),
        ("Router.viaPrivate", "Item.insert"),
        ("Router.viaGetter", "Item.insert"),
        ("awaited", "Item.insert"),
    ] {
        assert!(
            text.lines().any(|line| line.contains(source)
                && line.contains(target)
                && line.contains("calls")),
            "{source} -> {target}\n{text}"
        );
    }
    Ok(())
}

#[test]
fn rust_unwrap_methods_follow_declared_results_and_consume_only_result_layers() -> io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("rust-unwrap-results")?;
    fs::write(
        root.path().join("lib.rs"),
        concat!(
            "pub struct Wrapper;\npub struct Value;\n",
            "impl Wrapper {\n",
            "    pub fn new() -> Self { Self }\n",
            "    pub fn unwrap(self) -> Value { Value }\n",
            "    pub fn expect(self, _: &str) -> Value { Value }\n",
            "    pub fn finish(self) {}\n}\n",
            "impl Value { pub fn finish(self) {} }\n",
            "pub fn optional() -> Option<Wrapper> { Some(Wrapper) }\n",
            "pub fn nested() -> Result<Option<Wrapper>, ()> { Ok(Some(Wrapper)) }\n",
            "pub fn custom_unwrap() { Wrapper::new().unwrap().finish(); }\n",
            "pub fn custom_expect() { Wrapper::new().expect(\"ok\").finish(); }\n",
            "pub fn standard_unwrap() { optional().unwrap().finish(); }\n",
            "pub fn standard_expect() { optional().expect(\"ok\").finish(); }\n",
            "pub fn mixed() { optional().unwrap().unwrap().finish(); }\n",
            "pub fn nested_layers() { nested().unwrap().expect(\"ok\").finish(); }\n",
            "pub fn through_try() -> Option<()> { optional()?.unwrap().finish(); Some(()) }\n",
            "pub fn standard_try() -> Option<()> { optional()?.finish(); Some(()) }\n",
            "pub fn assigned() { let x = optional(); x.unwrap().finish(); }\n",
            "pub struct Factory;\n",
            "impl Factory {\n",
            "    pub fn new() -> Option<Wrapper> { Some(Wrapper) }\n",
            "    pub fn unwrap(self) -> Value { Value }\n}\n",
            "pub fn assigned_constructor() { let x = Factory::new(); x.unwrap().finish(); }\n",
            "pub fn assigned_try() -> Option<()> { let x = optional()?; x.unwrap().finish(); Some(()) }\n",
        ),
    )?;
    let graph = graph_of(root.path())?;
    for (caller, target) in [
        ("custom_unwrap", "Value::finish"),
        ("custom_expect", "Value::finish"),
        ("standard_unwrap", "Wrapper::finish"),
        ("standard_expect", "Wrapper::finish"),
        ("mixed", "Value::finish"),
        ("nested_layers", "Wrapper::finish"),
        ("through_try", "Value::finish"),
        ("standard_try", "Wrapper::finish"),
        ("assigned", "Wrapper::finish"),
        ("assigned_constructor", "Wrapper::finish"),
        ("assigned_try", "Value::finish"),
    ] {
        let targets: Vec<&str> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls)
            .filter_map(|edge| {
                let source = node_location(&graph, edge.source)?;
                let target = node_location(&graph, edge.target)?;
                (source.name.ends_with(&format!("::{caller}")) && target.name.ends_with("::finish"))
                    .then_some(target.name)
            })
            .collect();
        assert_eq!(
            targets.len(),
            1,
            "{caller}: {targets:?}\n{}",
            render(&graph)
        );
        assert!(targets[0].ends_with(target), "{caller}: {targets:?}");
    }
    let custom = crate::infra::testdir::TestDir::new("rust-local-option-result")?;
    fs::write(
        custom.path().join("lib.rs"),
        concat!(
            "pub struct Option<T>(T);\npub struct Wrapper;\npub struct Value;\n",
            "impl Option<Wrapper> { pub fn unwrap(self) -> Value { Value } }\n",
            "impl Wrapper { pub fn finish(self) {} }\n",
            "impl Value { pub fn finish(self) {} }\n",
            "pub fn factory() -> Option<Wrapper> { Option(Wrapper) }\n",
            "pub fn entry() { factory().unwrap().finish(); }\n",
        ),
    )?;
    let graph = graph_of(custom.path())?;
    let text = render(&graph);
    assert!(
        text.contains("entry -calls?-> crate::Value::finish"),
        "{text}"
    );
    assert!(
        !text.contains("entry -calls?-> crate::Wrapper::finish"),
        "{text}"
    );
    Ok(())
}

#[test]
fn rust_external_receiver_chains_do_not_fall_back_to_local_methods() -> io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("rust-external-chains")?;
    fs::write(
        root.path().join("foreign.py"),
        "class Command:\n    def __init__(self):\n        self.ready = True\n",
    )?;
    fs::write(
        root.path().join("lib.rs"),
        concat!(
            "use std::process::{self, Command};\n",
            "pub struct Local;\n",
            "impl Local {\n",
            "    pub fn arg(&mut self, _: &str) -> &mut Self { self }\n",
            "    pub fn output(&mut self) {}\n",
            "    pub fn finish(&mut self) {}\n}\n",
            "pub fn external_command() { let mut cmd = std::process::Command::new(\"git\"); cmd.arg(\"--version\").output(); }\n",
            "pub fn external_import() { Command::new(\"git\").arg(\"--version\").output(); }\n",
            "pub fn external_formatter(f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_struct(\"Example\").field(\"x\", &1).finish() }\n",
            "pub fn local_command(cmd: &mut Local) { cmd.arg(\"x\").output(); }\n",
        ),
    )?;
    let text = render(&graph_of(root.path())?);
    assert!(
        text.contains("local_command -calls?-> crate::Local::output"),
        "{text}"
    );
    for caller in ["external_command", "external_import", "external_formatter"] {
        assert!(
            !text.lines().any(|line| line.contains(caller)
                && line.contains("-calls")
                && line.contains("crate::Local::")),
            "{text}"
        );
    }
    Ok(())
}

#[test]
fn python_calls_resolve_to_last_same_module_definition() -> io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("python-module-redefinition")?;
    // Local scope IDs must not read unrelated global symbols: invert both scope kinds.
    fs::write(
        root.path().join("a_unrelated.py"),
        "class First:\n    pass\nclass Second:\n    pass\ndef unrelated():\n    pass\n",
    )?;
    fs::write(
        root.path().join("index.py"),
        concat!(
            "def normalize(value):\n",
            "    return value\n",
            "normalize('before')\n",
            "def caller(value):\n",
            "    return normalize(value)\n",
            "class ImportTime:\n",
            "    value = normalize('class')\n",
            "def normalize(value):\n",
            "    return value\n",
            "def local_owner():\n",
            "    def normalize(value):\n",
            "        return value\n",
            "    return normalize('nested')\n",
        ),
    )?;
    let graph = graph_of(root.path())?;
    let mut caller_targets: Vec<(u32, Confidence)> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Calls && edge.line == 5)
        .filter_map(|edge| {
            let source = node_location(&graph, edge.source)?;
            let target = node_location(&graph, edge.target)?;
            (source.path == "index.py"
                && source.name.ends_with(".caller")
                && target.path == "index.py"
                && target.name.ends_with(".normalize"))
            .then_some((target.line, edge.confidence))
        })
        .collect();
    caller_targets.sort_unstable();
    assert_eq!(
        caller_targets,
        vec![(8, Confidence::Inferred)],
        "function-body calls must select the final runtime binding as inferred:\n{}",
        render(&graph)
    );
    let mut module_targets: Vec<(u32, Confidence)> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Calls && edge.line == 3)
        .filter_map(|edge| {
            let source = node_location(&graph, edge.source)?;
            let target = node_location(&graph, edge.target)?;
            (source.path == "index.py"
                && target.path == "index.py"
                && target.name.ends_with(".normalize"))
            .then_some((target.line, edge.confidence))
        })
        .collect();
    module_targets.sort_unstable();
    assert_eq!(
        module_targets,
        vec![(1, Confidence::Exact)],
        "top-level calls must use the latest definition already executed:\n{}",
        render(&graph)
    );
    let line = 7;
    let mut targets: Vec<(u32, Confidence)> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Calls && edge.line == line)
        .filter_map(|edge| {
            let source = node_location(&graph, edge.source)?;
            let target = node_location(&graph, edge.target)?;
            (source.path == "index.py"
                && target.path == "index.py"
                && target.name.ends_with(".normalize"))
            .then_some((target.line, edge.confidence))
        })
        .collect();
    targets.sort_unstable();
    assert_eq!(
        targets,
        vec![(1, Confidence::Exact)],
        "definition-time call at line {line} must use the latest prior binding:\n{}",
        render(&graph)
    );
    let local_targets: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Calls && edge.line == 13)
        .filter_map(|edge| {
            let source = node_location(&graph, edge.source)?;
            let target = node_location(&graph, edge.target)?;
            (source.path == "index.py" && source.name.ends_with(".local_owner"))
                .then_some((target.line, edge.confidence))
        })
        .collect();
    assert_eq!(
        local_targets,
        vec![(11, Confidence::Exact)],
        "a unique local definition must shadow repeated module definitions:\n{}",
        render(&graph)
    );
    Ok(())
}

/// Scans, extracts and resolves a whole root, as a full index build does.
fn corpus_graph(root: &Path) -> std::io::Result<Graph> {
    let changes = crate::infra::scan::scan(root, &[], &[], i64::MAX, None)?;
    let facts: Vec<Option<crate::domain::facts::Facts>> = changes
        .records
        .iter()
        .map(|record| -> std::io::Result<_> {
            if !record.role.is_extracted() {
                return Ok(None);
            }
            let content = fs::read(root.join(&record.path))?;
            Ok(Some(extract(record, &content)))
        })
        .collect::<std::io::Result<_>>()?;
    Ok(build(&changes.records, facts))
}

#[test]
fn static_modifiers_are_read_before_the_method_name() {
    assert!(static_modifier("static concat(...targets)", "concat"));
    assert!(static_modifier(
        "public static async *items<T>(x: T)",
        "items"
    ));
    assert!(static_modifier(
        "@render() static render(options)",
        "render"
    ));
    assert!(!static_modifier(
        "render(options) { return static_render }",
        "render"
    ));
    assert!(!static_modifier(
        "@Inject(static) handle(request)",
        "handle"
    ));
    assert!(!static_modifier("concat(...targets)", "concat"));
}

#[test]
fn parameter_counts_come_from_the_declaration_list() {
    let count = super::parameter_count;
    assert_eq!(
        count("@GetMapping(PREFIX) public List<User> list()", "list"),
        Some((0, 0, false))
    );
    assert_eq!(
        count(
            "public JsonWriter value(Map<String, Long> v, int x)",
            "value"
        ),
        Some((2, 2, false))
    );
    assert_eq!(
        count(
            "public static <T> T fromJson(String json, Class<T> classOfT)",
            "fromJson"
        ),
        Some((2, 2, false))
    );
    assert_eq!(
        count("void log(String format, Object... args)", "log"),
        Some((2, 2, true))
    );
    assert_eq!(
        count("fun <T> run(block: () -> T): T", "run"),
        Some((1, 1, false))
    );
    assert_eq!(
        count(
            "internal inline fun build(action: Writer.() -> Unit): String",
            "build"
        ),
        Some((1, 1, false))
    );
    assert_eq!(
        count(
            "func join(_ parts: [String], separator: String = \", \", open: String = \"(\")",
            "join"
        ),
        Some((1, 3, false))
    );
    assert_eq!(
        count(
            "Task Send(object request, CancellationToken ct = default)",
            "Send"
        ),
        Some((1, 2, false))
    );
    assert_eq!(
        count(
            "fun all(limit: Int = Registry.LIMIT, x: Map<A, B> = mapOf())",
            "all"
        ),
        Some((0, 2, false))
    );
    assert_eq!(count("const value = 1", "value"), None);
}

#[test]
fn julia_dispatch_reads_only_supported_nominal_parameter_types() {
    let signature = "function Base.convert(value::Tuple{Queue, Other}, item::Queue = nothing)";
    assert_eq!(
        super::julia_parameter_type(signature, "convert", 1),
        Some("Queue")
    );
    assert_eq!(
        super::julia_method_accepts(signature, "convert", 1),
        Some(true)
    );
    assert_eq!(
        super::julia_parameter_type(signature, "convert", 0),
        None,
        "generic tuple annotations stay unknown"
    );
    assert_eq!(
        super::julia_parameter_name("Base.length(value = …)", "length", 0),
        Some("value"),
        "compacted signatures retain parameter names for facts-backed types"
    );
    assert_eq!(
        super::julia_parameter_name("Base.find(value::Queue; strict::Bool=false)", "find", 0),
        None,
        "keyword signatures stay unknown"
    );
    assert_eq!(
        super::julia_method_accepts("Base.find(value::Queue; strict::Bool=false)", "find", 2),
        Some(false),
        "Julia keyword parameters do not add positional arity"
    );
    assert_eq!(
        super::julia_parameter_name("Base.convert(value::T) where T", "convert", 0),
        None,
        "type variables stay unknown"
    );
    assert_eq!(
        super::julia_parameter_name("Base.length(value) …", "length", 0),
        None,
        "clipped signatures stay unknown"
    );

    assert_eq!(
        super::julia_parameter_type("Base.join(values::Queue...)", "join", 2),
        Some("Queue")
    );
    assert_eq!(
        super::julia_parameter_type("Base.find(value::Queue; strict::Bool=false)", "find", 0),
        None,
        "keyword signatures stay unknown"
    );
    assert_eq!(
        super::julia_parameter_type("Base.convert(value::T) where T", "convert", 0),
        None,
        "type variables stay unknown"
    );
}

fn targets_at(
    graph: &Graph,
    caller_suffix: &str,
    line: u32,
    kind: crate::domain::graph::EdgeKind,
) -> Vec<(Confidence, String)> {
    let mut targets: Vec<(Confidence, String)> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == kind && edge.line == line)
        .filter(|edge| {
            graph
                .symbols
                .get(edge.source as usize)
                .is_some_and(|source| source.qualified.ends_with(caller_suffix))
        })
        .filter_map(|edge| {
            graph
                .symbols
                .get(edge.target as usize)
                .map(|target| (edge.confidence, target.signature.clone()))
        })
        .collect();
    targets.sort();
    targets
}

#[test]
fn swift_calls_select_equal_arity_overloads_by_external_labels() -> std::io::Result<()> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolve/swift-labels-app");
    let graph = graph_of(&root)?;

    let matching = targets_at(
        &graph,
        "Codec.run",
        5,
        crate::domain::graph::EdgeKind::Calls,
    );
    assert_eq!(matching.len(), 1, "matching label must select one overload");
    assert_eq!(matching[0].0, Confidence::Exact);
    assert!(
        matching[0].1.contains("base radix: Int"),
        "selected: {matching:?}"
    );

    let mismatched = targets_at(
        &graph,
        "Codec.run",
        6,
        crate::domain::graph::EdgeKind::Calls,
    );
    assert!(
        mismatched.is_empty(),
        "wrong labels must not resolve: {mismatched:?}"
    );
    Ok(())
}

#[test]
fn swift_calls_skip_only_defaulted_parameters() {
    assert!(swift_source_identity_matches(
        "render(_:columns?:root:header:separator?:)",
        "render(_:root:header:)"
    ));
    assert!(swift_source_identity_matches(
        "render(_:root:header:separator?:)",
        "render(_:root:header:)"
    ));
    assert!(swift_source_identity_matches("render(_?:_:)", "render(_:)"));
    assert!(!swift_source_identity_matches(
        "render(_:columns:root:header:)",
        "render(_:root:header:)"
    ));
    assert!(!swift_source_identity_matches(
        "render(_:root:header?:)",
        "render(_:header:root:)"
    ));
}

#[test]
fn swift_call_resolves_overload_with_omitted_default_parameters() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolve/swift-app");
    let graph = graph_of(&root)?;
    let targets = targets_at(&graph, "main", 9, crate::domain::graph::EdgeKind::Calls);

    assert_eq!(
        targets.len(),
        1,
        "defaulted labels select one overload: {targets:?}"
    );
    assert!(
        targets[0].1.contains("U: CustomStringConvertible"),
        "selected: {targets:?}"
    );
    Ok(())
}

#[test]
fn objective_c_message_sends_keep_full_selectors() -> std::io::Result<()> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolve/objc-selectors-app");
    let graph = graph_of(&root)?;
    let calls = crate::domain::graph::EdgeKind::Calls;

    let to = targets_at(&graph, "Sink.run", 6, calls);
    assert_eq!(to.len(), 1, "to selector must select one method: {to:?}");
    assert!(to[0].1.contains("to:"), "selected: {to:?}");
    assert_eq!(to[0].0, Confidence::Exact);

    let on = targets_at(&graph, "Sink.run", 7, calls);
    assert_eq!(on.len(), 1, "on selector must select one method: {on:?}");
    assert!(on[0].1.contains("on:"), "selected: {on:?}");
    assert_eq!(on[0].0, Confidence::Exact);

    let mismatched = targets_at(&graph, "Sink.run", 8, calls);
    assert!(
        mismatched.is_empty(),
        "unknown selector pieces must not resolve: {mismatched:?}"
    );

    Ok(())
}

#[test]
fn objective_c_selector_expression_resolves_its_full_selector() -> std::io::Result<()> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolve/objc-selectors-app");
    let graph = graph_of(&root)?;
    let selector = targets_at(&graph, "Sink.run", 9, crate::domain::graph::EdgeKind::Uses);
    assert_eq!(
        selector.len(),
        1,
        "@selector must resolve to its method: {selector:?}"
    );
    assert_eq!(selector[0].0, Confidence::Inferred);
    assert!(selector[0].1.contains("on:"), "selected: {selector:?}");

    let mismatched = targets_at(&graph, "Sink.run", 10, crate::domain::graph::EdgeKind::Uses);
    assert!(
        mismatched.is_empty(),
        "unknown selector pieces must not resolve: {mismatched:?}"
    );

    let reset_call = targets_at(
        &graph,
        "Sink.run",
        11,
        crate::domain::graph::EdgeKind::Calls,
    );
    assert_eq!(
        reset_call.len(),
        1,
        "zero-argument selector must resolve: {reset_call:?}"
    );
    assert_eq!(reset_call[0].0, Confidence::Exact);

    let reset_selector = targets_at(&graph, "Sink.run", 12, crate::domain::graph::EdgeKind::Uses);
    assert_eq!(
        reset_selector.len(),
        1,
        "zero-argument @selector must resolve: {reset_selector:?}"
    );
    assert_eq!(reset_selector[0].0, Confidence::Inferred);
    assert!(reset_selector[0].1.contains("reset"));
    Ok(())
}

#[test]
fn julia_base_extensions_dispatch_on_typed_arguments() -> std::io::Result<()> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolve/julia-dispatch-app");
    let graph = graph_of(&root)?;
    let calls = crate::domain::graph::EdgeKind::Calls;

    let queue = targets_at(&graph, "App.queue_size", 6, calls);
    assert_eq!(
        queue.len(),
        1,
        "typed Queue call must select only its most-specific extension: {queue:?}"
    );
    assert_eq!(queue[0].0, Confidence::Inferred);
    assert!(queue[0].1.contains("Base.length"));
    assert!(!queue[0].1.contains("Other"));
    let queue_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == calls && edge.line == 6)
        .filter(|edge| {
            graph
                .symbols
                .get(edge.source as usize)
                .is_some_and(|source| source.qualified.ends_with("App.queue_size"))
        })
        .collect();
    assert_eq!(
        queue_edges.len(),
        1,
        "Queue source targets must stay unique"
    );
    let queue_target = &graph.symbols[queue_edges[0].target as usize];
    assert_eq!(graph.files[queue_target.file as usize].path, "src/types.jl");
    assert_eq!(queue_target.line, 8);
    assert!(
        !queue_target.signature.contains("::"),
        "long method signature must exercise the facts-backed annotation fallback: {}",
        queue_target.signature
    );

    let other = targets_at(&graph, "App.other_size", 10, calls);
    assert_eq!(
        other.len(),
        1,
        "typed Other call must select one extension: {other:?}"
    );
    assert_eq!(other[0].0, Confidence::Inferred);
    assert!(other[0].1.contains("Base.length") && other[0].1.contains("Other"));
    assert!(!other[0].1.contains("Queue"));
    let other_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == calls && edge.line == 10)
        .filter(|edge| {
            graph
                .symbols
                .get(edge.source as usize)
                .is_some_and(|source| source.qualified.ends_with("App.other_size"))
        })
        .collect();
    assert_eq!(
        other_edges.len(),
        1,
        "Other source targets must stay unique"
    );
    let other_target = &graph.symbols[other_edges[0].target as usize];
    assert_eq!(graph.files[other_target.file as usize].path, "src/types.jl");
    assert_eq!(other_target.line, 9);

    let unknown = targets_at(&graph, "App.unknown_size", 14, calls);
    assert!(
        unknown.is_empty(),
        "untyped call must remain unresolved: {unknown:?}"
    );
    Ok(())
}

#[test]
fn rust_closure_calls_follow_generic_trait_bounds_and_shadowing() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/resolve/rust-closure-trait-bounds");
    let graph = graph_of(&root)?;
    let calls = crate::domain::graph::EdgeKind::Calls;
    let targets = |caller_suffix: &str, line: u32| {
        graph
            .edges
            .iter()
            .filter(|edge| edge.kind == calls && edge.line == line)
            .filter(|edge| {
                graph
                    .symbols
                    .get(edge.source as usize)
                    .is_some_and(|source| source.qualified.ends_with(caller_suffix))
            })
            .filter_map(|edge| {
                graph
                    .symbols
                    .get(edge.target as usize)
                    .map(|target| (edge.confidence, target))
            })
            .collect::<Vec<_>>()
    };

    let bounded = targets("bounded", 20);
    assert_eq!(bounded.len(), 2, "bounded closure targets: {bounded:?}");
    assert!(bounded.iter().any(|(confidence, target)| {
        *confidence == Confidence::Exact
            && target.qualified.ends_with("Readable::value")
            && target.line == 2
    }));
    assert!(bounded.iter().any(|(confidence, target)| {
        *confidence == Confidence::Inferred
            && target.qualified.ends_with("Concrete::value")
            && target.line == 58
    }));
    assert!(
        bounded
            .iter()
            .all(|(_, target)| !target.qualified.ends_with("Unrelated::value"))
    );

    let where_bounded = targets("where_bounded", 39);
    assert_eq!(
        where_bounded.len(),
        2,
        "where-bounded closure target: {where_bounded:?}"
    );
    assert!(where_bounded.iter().any(|(confidence, target)| {
        *confidence == Confidence::Exact
            && target.qualified.ends_with("Readable::value")
            && target.line == 2
    }));
    assert!(where_bounded.iter().any(|(confidence, target)| {
        *confidence == Confidence::Inferred
            && target.qualified.ends_with("Concrete::value")
            && target.line == 58
    }));

    let impl_bounded = targets("Wrapper::read", 67);
    assert_eq!(
        impl_bounded.len(),
        2,
        "impl-bounded closure targets: {impl_bounded:?}"
    );
    assert!(impl_bounded.iter().any(|(confidence, target)| {
        *confidence == Confidence::Exact
            && target.qualified.ends_with("Readable::value")
            && target.line == 2
    }));
    assert!(impl_bounded.iter().any(|(confidence, target)| {
        *confidence == Confidence::Inferred
            && target.qualified.ends_with("Concrete::value")
            && target.line == 58
    }));

    let impl_where_bounded = targets("WhereWrapper::read", 79);
    assert_eq!(
        impl_where_bounded.len(),
        2,
        "impl where-bounded closure targets: {impl_where_bounded:?}"
    );
    assert!(impl_where_bounded.iter().any(|(confidence, target)| {
        *confidence == Confidence::Exact
            && target.qualified.ends_with("Readable::value")
            && target.line == 2
    }));
    assert!(impl_where_bounded.iter().any(|(confidence, target)| {
        *confidence == Confidence::Inferred
            && target.qualified.ends_with("Concrete::value")
            && target.line == 58
    }));

    let shadowed = targets("shadowed", 26);
    assert_eq!(shadowed.len(), 1, "shadowed closure target: {shadowed:?}");
    assert_eq!(shadowed[0].0, Confidence::Exact);
    assert!(shadowed[0].1.qualified.ends_with("Other::value"));
    assert_eq!(shadowed[0].1.line, 14);

    let unbounded = targets("unbounded", 31);
    assert!(
        unbounded.is_empty(),
        "unbounded closure target: {unbounded:?}"
    );
    Ok(())
}

#[test]
fn python_not_implemented_bases_dispatch_to_local_overrides() -> std::io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("python-not-implemented-dispatch")?;
    fs::write(
        root.path().join("contracts.py"),
        concat!(
            "class Decoder:\n",
            "    def decode(self, data):\n",
            "        raise NotImplementedError()\n",
            "    def flush(self):\n",
            "        raise NotImplementedError\n",
            "class TextDecoder(Decoder):\n",
            "    def decode(self, data):\n",
            "        return data.decode()\n",
            "class PassthroughDecoder(Decoder):\n",
            "    def decode(self, data):\n",
            "        return data\n",
            "    def flush(self):\n",
            "        return b''\n",
            "class InheritedDecoder(PassthroughDecoder):\n",
            "    pass\n",
        ),
    )?;
    fs::write(
        root.path().join("consumer.py"),
        concat!(
            "from contracts import Decoder\n",
            "def decode(decoder: Decoder, data: bytes):\n",
            "    return decoder.decode(data)\n",
            "def flush(decoder: Decoder):\n",
            "    return decoder.flush()\n",
        ),
    )?;
    let graph = graph_of(root.path())?;
    let target_lines = |caller: &str, line: u32| {
        let mut targets: Vec<(u32, Confidence)> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls && edge.line == line)
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                (graph.files.get(source.file as usize)?.path == "consumer.py"
                    && source.qualified.ends_with(caller)
                    && graph.files.get(target.file as usize)?.path == "contracts.py")
                    .then_some((target.line, edge.confidence))
            })
            .collect();
        targets.sort_unstable();
        targets
    };
    assert_eq!(
        target_lines(".decode", 3),
        vec![(7, Confidence::Inferred), (10, Confidence::Inferred)],
        "Python contract stubs must dispatch to each local override, not the raising base:\n{}",
        render(&graph)
    );
    assert_eq!(
        target_lines(".flush", 5),
        vec![(12, Confidence::Inferred)],
        "Python contract stubs must dispatch through inherited overrides:\n{}",
        render(&graph)
    );
    Ok(())
}

#[test]
fn rust_methods_do_not_cross_cargo_target_roots() -> std::io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("rust-cargo-target-method-scope")?;
    fs::create_dir_all(root.path().join("src"))?;
    fs::create_dir_all(root.path().join("examples"))?;
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"target-scope\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    fs::write(
        root.path().join("src/lib.rs"),
        concat!(
            "pub struct DirEntry;\n",
            "impl DirEntry {\n",
            "    pub fn path(&self) {}\n",
            "}\n",
            "pub struct ChildEntry;\n",
            "impl ChildEntry {\n",
            "    pub fn value(&self) {}\n",
            "}\n",
            "pub fn library_call(entry: &DirEntry) {\n",
            "    entry.path();\n",
            "}\n",
            "pub fn library_child_call(entry: &ChildEntry) {\n",
            "    entry.value();\n",
            "}\n",
        ),
    )?;
    fs::write(
        root.path().join("examples/walk.rs"),
        concat!(
            "mod inner;\n",
            "struct DirEntry;\n",
            "impl DirEntry {\n",
            "    fn path(&self) {}\n",
            "}\n",
            "fn example_call(entry: &DirEntry) {\n",
            "    entry.path();\n",
            "}\n",
            "fn child_call(entry: &inner::ChildEntry) {\n",
            "    entry.value();\n",
            "}\n",
        ),
    )?;
    fs::create_dir_all(root.path().join("examples/walk"))?;
    fs::write(
        root.path().join("examples/walk/inner.rs"),
        concat!(
            "pub struct ChildEntry;\n",
            "impl ChildEntry {\n",
            "    pub fn value(&self) {}\n",
            "}\n",
        ),
    )?;

    let graph = graph_of(root.path())?;
    let targets = |caller: &str, line: u32| {
        let mut found: Vec<(String, u32)> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls && edge.line == line)
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                source
                    .qualified
                    .ends_with(caller)
                    .then(|| (graph.files[target.file as usize].path.clone(), target.line))
            })
            .collect();
        found.sort_by(|left, right| (left.0.as_str(), left.1).cmp(&(right.0.as_str(), right.1)));
        found
    };

    assert_eq!(
        targets("library_call", 10),
        vec![("src/lib.rs".to_owned(), 3)],
        "library calls must not bind to example target methods:\n{}",
        render(&graph)
    );
    assert_eq!(
        targets("example_call", 7),
        vec![("examples/walk.rs".to_owned(), 4)],
        "example calls must stay within their own Cargo target:\n{}",
        render(&graph)
    );
    assert_eq!(
        targets("library_child_call", 13),
        vec![("src/lib.rs".to_owned(), 7)],
        "library submodule candidates must not leak into example modules:\n{}",
        render(&graph)
    );
    assert_eq!(
        targets("child_call", 10),
        vec![("examples/walk/inner.rs".to_owned(), 3)],
        "target module files must inherit their Cargo root method scope:\n{}",
        render(&graph)
    );
    Ok(())
}

#[test]
fn rust_calls_respect_mutually_exclusive_unix_cfg_gates() -> std::io::Result<()> {
    let root = crate::infra::testdir::TestDir::new("rust-cfg-platform-calls")?;
    fs::create_dir_all(root.path().join("src"))?;
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"cfg-platform\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    fs::write(
        root.path().join("src/lib.rs"),
        concat!(
            "mod api {\n",
            "    #[cfg(unix)]\n",
            "    pub fn open_existing_regular() {}\n",
            "    #[cfg(not(unix))]\n",
            "    pub fn open_existing_regular() {}\n",
            "}\n",
            "#[cfg(unix)]\n",
            "fn touch_stamp() {\n",
            "    api::open_existing_regular();\n",
            "}\n",
            "fn acquire() {\n",
            "    api::open_existing_regular();\n",
            "}\n",
        ),
    )?;
    let graph = graph_of(root.path())?;
    let targets = |caller: &str, line: u32| {
        let mut lines: Vec<u32> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls && edge.line == line)
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                (source.qualified.ends_with(caller)
                    && target.qualified.ends_with("api::open_existing_regular"))
                .then_some(target.line)
            })
            .collect();
        lines.sort_unstable();
        lines
    };
    assert_eq!(
        targets("touch_stamp", 9),
        vec![3],
        "a Unix-only caller must exclude the not(unix) implementation:\n{}",
        render(&graph)
    );
    assert_eq!(
        targets("acquire", 12),
        vec![3, 5],
        "an unguarded caller must retain both platform alternatives:\n{}",
        render(&graph)
    );
    Ok(())
}
