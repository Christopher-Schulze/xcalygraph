//! Resolution golden tests: every directory under `tests/fixtures/resolve/` is a small root;
//! its edges are compared with `<case>.edges`. `XCALYGRAPH_BLESS=1` rewrites them; every
//! blessed edge must be checked against the fixture source before commit.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Input, build, swift_source_identity_matches};
use crate::domain::corpus::FileRecord;
use crate::domain::graph::{Confidence, Graph};
use crate::domain::role::{self, FileFacts};
use crate::extract::extract;

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
    Ok(build(&Input {
        files: &files,
        facts: &facts,
    }))
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

/// Precision sampling (`docs/spec.md` section 12): prints up to 50 edges per kind and
/// confidence from the root in `XCALYGRAPH_SAMPLE_ROOT`, each with the reference line and the
/// target signature, for verification against source. Development only:
/// `XCALYGRAPH_SAMPLE_ROOT=<dir> cargo test --release sample_edges -- --ignored --nocapture`.
#[test]
#[ignore = "development tool; needs XCALYGRAPH_SAMPLE_ROOT"]
fn sample_edges() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::var_os("XCALYGRAPH_SAMPLE_ROOT") else {
        return Ok(());
    };
    let root = PathBuf::from(root).canonicalize()?;
    let per_bucket: usize = std::env::var("XCALYGRAPH_SAMPLE_SIZE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(50);
    let graph = corpus_graph(&root)?;
    let symbols = graph.symbols.len() as u32;
    let location = |node: u32| -> (String, u32, String) {
        if node < symbols {
            let symbol = &graph.symbols[node as usize];
            let path = graph.files[symbol.file as usize].path.clone();
            (path, symbol.line, symbol.qualified.clone())
        } else {
            let path = graph.files[(node - symbols) as usize].path.clone();
            (path.clone(), 1, path)
        }
    };
    let mut buckets: std::collections::BTreeMap<(u8, u8), Vec<&crate::domain::graph::Edge>> =
        Default::default();
    for edge in &graph.edges {
        buckets
            .entry((edge.kind as u8, edge.confidence as u8))
            .or_default()
            .push(edge);
    }
    for ((kind, confidence), edges) in buckets {
        let total = edges.len();
        let step = (total / per_bucket).max(1);
        println!(
            "== {} conf={} total={total}",
            crate::domain::graph::EdgeKind::from_u8(kind).map_or("?", |k| k.id()),
            confidence
        );
        for edge in edges.iter().step_by(step).take(per_bucket) {
            let (source_path, _, source_name) = location(edge.source);
            let (target_path, target_line, target_name) = location(edge.target);
            let line_text = fs::read_to_string(root.join(&source_path))
                .ok()
                .and_then(|text| {
                    text.lines()
                        .nth(edge.line as usize - 1)
                        .map(|line| line.trim().chars().take(110).collect::<String>())
                })
                .unwrap_or_default();
            let signature = if edge.target < symbols {
                graph.symbols[edge.target as usize].signature.clone()
            } else {
                String::new()
            };
            println!(
                "{source_name} @ {source_path}:{}\n    | {line_text}\n    -> {target_name} @ {target_path}:{target_line} | {}",
                edge.line,
                signature.chars().take(100).collect::<String>()
            );
        }
    }
    Ok(())
}

/// Scans, extracts and resolves a whole root, as a full index build does.
fn corpus_graph(root: &Path) -> std::io::Result<Graph> {
    let changes = crate::infra::scan::scan(root, &[], &[], i64::MAX)?;
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
    Ok(build(&Input {
        files: &changes.records,
        facts: &facts,
    }))
}

/// Depth comparison input (`scripts/benchmarks/compare-depth.sh`): writes every edge of the
/// root in `XCALYGRAPH_DUMP_ROOT` to `XCALYGRAPH_DUMP_FILE` as TSV lines `kind confidence
/// source_path line target_path target_line target_name`. Development only.
#[test]
#[ignore = "development tool; needs XCALYGRAPH_DUMP_ROOT and XCALYGRAPH_DUMP_FILE"]
fn dump_edges() -> Result<(), Box<dyn std::error::Error>> {
    let (Some(root), Some(file)) = (
        std::env::var_os("XCALYGRAPH_DUMP_ROOT"),
        std::env::var_os("XCALYGRAPH_DUMP_FILE"),
    ) else {
        return Ok(());
    };
    let graph = corpus_graph(&PathBuf::from(root).canonicalize()?)?;
    let symbols = graph.symbols.len() as u32;
    let location = |node: u32| -> (&str, u32, &str) {
        if node < symbols {
            let symbol = &graph.symbols[node as usize];
            (
                &graph.files[symbol.file as usize].path,
                symbol.line,
                &symbol.name,
            )
        } else {
            let path = &graph.files[(node - symbols) as usize].path;
            (path, 1, path)
        }
    };
    let mut out = String::new();
    for edge in &graph.edges {
        let (source_path, _, _) = location(edge.source);
        let (target_path, target_line, target_name) = location(edge.target);
        writeln!(
            out,
            "{}\t{}\t{source_path}\t{}\t{target_path}\t{target_line}\t{target_name}",
            edge.kind.id(),
            edge.confidence as u8,
            edge.line
        )?;
    }
    fs::write(file, out)?;
    Ok(())
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
