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
