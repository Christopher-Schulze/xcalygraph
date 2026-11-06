//! Source-exact receiver visibility across initializers, blocks and closures.

use crate::domain::graph::EdgeKind;
use crate::infra::testdir::TestDir;

#[test]
fn iterable_receivers_use_declared_elements_without_leaking_callback_locals() -> std::io::Result<()>
{
    use crate::domain::graph::Confidence::Exact;
    let cases: [(&str, &str, Option<&str>); 9] = [
        (
            "array",
            "function probe(parts: Part[]) { for (const part of parts) { part.encode(); } }",
            Some("Part"),
        ),
        (
            "generic",
            "function probe(parts: Array<Part>) { for (const part of parts) { part.encode(); } }",
            Some("Part"),
        ),
        (
            "readonly",
            "function probe(parts: readonly Other[]) { for (const part of parts) { part.encode(); } }",
            Some("Other"),
        ),
        (
            "set",
            "function probe(parts: Set<Part>) { for (const part of parts) { part.encode(); } }",
            Some("Part"),
        ),
        (
            "nested",
            "function probe(parts: Part[][]) { for (const part of parts) { part.encode(); } }",
            None,
        ),
        (
            "value-is-no-element",
            "function probe(parts: Part) { for (const part of parts) { part.encode(); } }",
            None,
        ),
        (
            "container-is-no-element",
            "function probe(parts: Part[]) { parts.encode(); }",
            None,
        ),
        (
            "undeclared",
            "function probe() { const parts = Array.from([0]).map(() => new Part()); for (const part of parts) { part.encode(); } }",
            None,
        ),
        (
            "callback-local",
            "function probe(items: number[]) { items.map(() => { const part = new Part(); return part; }); part.encode(); }",
            None,
        ),
    ];
    for (name, body, expected) in cases {
        let root = TestDir::new("iterable-receiver-evidence")?;
        root.write(
            "source.ts",
            &format!(
                "class Part {{ encode() {{ return 1; }} }}\nclass Other {{ encode() {{ return 2; }} }}\n{body}\n"
            ),
        )?;
        let graph = super::graph_of(root.path())?;
        let calls: Vec<_> = graph
            .edges
            .iter()
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                if edge.kind != EdgeKind::Calls || source.name != "probe" || target.name != "encode"
                {
                    return None;
                }
                let owner = graph.symbols.get(target.parent as usize)?;
                Some((owner.name.as_str(), edge.confidence))
            })
            .collect();
        let wanted: Vec<_> = expected.into_iter().map(|owner| (owner, Exact)).collect();
        assert_eq!(calls, wanted, "{name}: {}", super::render(&graph));
    }
    Ok(())
}

#[test]
fn receiver_visibility_preserves_same_line_rhs_closures_and_blocks() -> std::io::Result<()> {
    let cases = [
        (
            "visibility.ts",
            r#"class Outer { run(): string { return 'outer'; } make(): Outer { return this; } }
class Inner { run(): string { return 'inner'; } make(): Inner { return this; } }
function afterBlock(value: Outer) { { const value: Inner = new Inner(); value.run(); } value.run(); }
function beforeBlock(value: Outer) { value.run(); { const value: Inner = new Inner(); value.run(); } }
function afterClosure(value: Outer) { const closure = (value: Inner) => value.run(); value.run(); }
function beforeClosure(value: Outer) { value.run(); const closure = (value: Inner) => value.run(); }
function afterLoop(value: Outer) { for (let value: Inner = new Inner(); false;) { value.run(); } value.run(); }
function functionVar() { { var value: Inner = new Inner(); } value.run(); }
function nestedFactory(value: Outer) { const result = value.make(); function inside(value: Inner) { result.run(); value.run(); } }
"#,
        ),
        (
            "visibility.rs",
            r#"struct Outer;
impl Outer { fn run(&self) -> u8 { 1 } fn make(&self) -> Inner { Inner } }
struct Inner;
impl Inner { fn run(&self) -> u8 { 2 } fn make(&self) -> Outer { Outer } }
fn afterBlock(value: Outer) { { let value: Inner = Inner; value.run(); } value.run(); }
fn beforeBlock(value: Outer) { value.run(); { let value: Inner = Inner; value.run(); } }
fn afterClosure(value: Outer) { let closure = |value: Inner| value.run(); value.run(); }
fn beforeClosure(value: Outer) { value.run(); let closure = |value: Inner| value.run(); }
fn rhs(value: Outer) { let value: Inner = value.make(); value.run(); }
"#,
        ),
    ];
    for (path, source) in cases {
        let root = TestDir::new("binding-byte-visibility")?;
        root.write(path, source)?;
        let graph = super::graph_of(root.path())?;
        let targets = |caller: &str| {
            let mut found: Vec<_> = graph
                .edges
                .iter()
                .filter(|edge| edge.kind == EdgeKind::Calls)
                .filter_map(|edge| {
                    let source = graph.symbols.get(edge.source as usize)?;
                    let target = graph.symbols.get(edge.target as usize)?;
                    (source.name == caller && matches!(target.name.as_str(), "run" | "make"))
                        .then(|| target.qualified.replace("::", "."))
                })
                .collect();
            found.sort();
            found
        };
        for caller in ["afterBlock", "beforeBlock", "afterClosure", "beforeClosure"] {
            let found = targets(caller);
            assert_eq!(found.len(), 2, "{path} {caller}: {found:?}");
            for owner in ["Outer", "Inner"] {
                assert!(
                    found
                        .iter()
                        .any(|target| target.ends_with(&format!(".{owner}.run"))),
                    "{path} {caller}: missing {owner}.run: {found:?}"
                );
            }
        }
        if path.ends_with(".ts") {
            let found = targets("afterLoop");
            assert_eq!(found.len(), 2, "{path} afterLoop: {found:?}");
            assert!(found.iter().any(|target| target.ends_with(".Outer.run")));
            assert!(found.iter().any(|target| target.ends_with(".Inner.run")));
            let found = targets("functionVar");
            assert_eq!(found.len(), 1, "{path} functionVar: {found:?}");
            assert!(found[0].ends_with(".Inner.run"));
            let found = targets("inside");
            assert_eq!(found.len(), 2, "{path} inside: {found:?}");
            assert!(found.iter().any(|target| target.ends_with(".Outer.run")));
            assert!(found.iter().any(|target| target.ends_with(".Inner.run")));
        } else {
            let found = targets("rhs");
            assert_eq!(found.len(), 2, "{path} rhs: {found:?}");
            assert!(found.iter().any(|target| target.ends_with(".Outer.make")));
            assert!(found.iter().any(|target| target.ends_with(".Inner.run")));
        }
    }
    Ok(())
}

#[test]
fn es_local_export_aliases_reach_their_declaration() -> std::io::Result<()> {
    let root = TestDir::new("es-local-export-alias")?;
    root.write(
        "base.ts",
        "class Inner { route() { return 1; } }\nclass Unrelated { route() { return 2; } }\nexport { Inner as Base };\n",
    )?;
    root.write(
        "app.ts",
        "import { Base } from './base';\nexport class App extends Base {}\n",
    )?;
    root.write(
        "use.ts",
        "import { App } from './app';\nimport { Base } from './base';\nexport function caller() { new App().route(); new Base().route(); }\n",
    )?;
    let graph = super::graph_of(root.path())?;
    let owners: Vec<_> = graph
        .edges
        .iter()
        .filter_map(|edge| {
            let source = graph.symbols.get(edge.source as usize)?;
            let target = graph.symbols.get(edge.target as usize)?;
            (edge.kind == EdgeKind::Calls && source.name == "caller" && target.name == "route")
                .then(|| {
                    graph
                        .symbols
                        .get(target.parent as usize)
                        .map(|owner| owner.name.as_str())
                })
                .flatten()
        })
        .collect();
    assert_eq!(owners, ["Inner"], "{}", super::render(&graph));
    Ok(())
}

#[test]
fn es_export_alias_base_may_share_the_subclass_name() -> std::io::Result<()> {
    let root = TestDir::new("es-local-export-same-name")?;
    root.write(
        "base.ts",
        "class Hono { route() { return 1; } }\nexport { Hono as HonoBase }\n",
    )?;
    root.write(
        "hono.ts",
        "import { HonoBase } from './base'\nexport class Hono<E extends Env = Blank, P extends string = '/',> extends HonoBase<E> {}\n",
    )?;
    root.write(
        "use.ts",
        "import { Hono } from './hono'\nexport function caller() { const app = new Hono(); app.route(); }\n",
    )?;
    let graph = super::graph_of(root.path())?;
    let routes = graph
        .edges
        .iter()
        .filter(|edge| {
            edge.kind == EdgeKind::Calls
                && graph
                    .symbols
                    .get(edge.source as usize)
                    .is_some_and(|s| s.name == "caller")
                && graph
                    .symbols
                    .get(edge.target as usize)
                    .is_some_and(|s| s.name == "route")
        })
        .count();
    assert_eq!(routes, 1, "{}", super::render(&graph));
    Ok(())
}

#[test]
fn typescript_generic_parameters_shadow_only_by_their_own_name() -> std::io::Result<()> {
    for (signature, shadowed) in [
        ("class C<T> extends B<T>", "T"),
        ("function f<const T, in out U>(x: T)", "U"),
    ] {
        let root = TestDir::new("generic-parameter-shadow")?;
        root.write(
            "a.ts",
            &format!("export class {shadowed} {{ run() {{ return 1; }} }}\nexport {signature} {{ const v: {shadowed} = null as any; v.run(); }}\n"),
        )?;
        let graph = super::graph_of(root.path())?;
        let calls = graph
            .edges
            .iter()
            .filter(|edge| {
                edge.kind == EdgeKind::Calls
                    && graph
                        .symbols
                        .get(edge.target as usize)
                        .is_some_and(|s| s.name == "run")
            })
            .count();
        assert_eq!(calls, 0, "{signature}: {}", super::render(&graph));
    }
    let root = TestDir::new("generic-parameter-constraint")?;
    root.write(
        "a.ts",
        "export class Base { run() { return 1; } }\nexport class Other { run() { return 2; } }\nexport class C<T extends Base = Base, P = '>'> extends Base {}\nexport function f() { new C().run(); }\n",
    )?;
    let graph = super::graph_of(root.path())?;
    let targets: Vec<_> = graph
        .edges
        .iter()
        .filter_map(|edge| {
            let target = graph.symbols.get(edge.target as usize)?;
            (edge.kind == EdgeKind::Calls && target.name == "run").then_some(target.qualified)
        })
        .collect();
    assert_eq!(targets, ["a.Base.run"], "{}", super::render(&graph));
    Ok(())
}
