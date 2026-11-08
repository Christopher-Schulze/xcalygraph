//! A declared type proves a receiver only when it names one value type.

mod support;

use std::error::Error;
use std::fs;
use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

type TestResult = Result<(), Box<dyn Error>>;

/// `name conf` of the outgoing edges of `symbol`, `conf` empty when exact.
fn calls(root: &Path, symbol: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let output = support::command(BINARY, root)
        .args(["show", symbol, "--json", "-C"])
        .arg(root)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    let mut found = Vec::new();
    for line in String::from_utf8(output.stdout)?.lines() {
        if !line.contains("\"t\":\"edge\"") || !line.contains("\"rel\":\"out\"") {
            continue;
        }
        let field = |key: &str| {
            line.split(&format!("\"{key}\":\""))
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_default()
                .to_owned()
        };
        if field("kind") == "calls" {
            found.push(format!("{} {}", field("name"), field("conf")));
        }
    }
    found.sort();
    Ok(found)
}

const TS: &str = "export class Left { run() { return 1 } }\n\
export class Right { run() { return 2 } }\n\
export class Only { solo() { return 3 } }\n\
export function direct(x: Left | Right) { x.run(); }\n\
export function nullable(x: Left | null) { x.run(); }\n\
export function leading(x: undefined | Right) { x?.run(); }\n\
export function both(x: Left & Only) { x.solo(); }\n\
export function partial(x: Left | Only) { x.solo(); }\n\
export function elements(items: (Left | Right)[]) { for (const x of items) { x.run(); } }\n";

#[test]
fn typescript_unions_reach_every_alternative() -> TestResult {
    let root = support::exclusive_dir("declared-ts")?;
    fs::write(root.join("a.ts"), TS)?;
    assert_eq!(
        calls(&root, "direct")?,
        ["a.Left.run exact", "a.Right.run exact"]
    );
    assert_eq!(calls(&root, "nullable")?, ["a.Left.run exact"]);
    assert_eq!(calls(&root, "leading")?, ["a.Right.run exact"]);
    assert_eq!(calls(&root, "both")?, ["a.Only.solo exact"]);
    assert!(
        !calls(&root, "partial")?
            .iter()
            .any(|edge| edge.ends_with("exact")),
        "an alternative without the member proves nothing"
    );
    assert!(
        !calls(&root, "elements")?
            .iter()
            .any(|edge| edge.ends_with("exact")),
        "a composed element proves nothing"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_and_php_unions_are_never_exact() -> TestResult {
    let root = support::exclusive_dir("declared-py-php")?;
    fs::write(
        root.join("a.py"),
        "class Left:\n    def run(self):\n        return 1\n\nclass Right:\n    def run(self):\n        return 2\n\ndef direct(x: Left | Right):\n    x.run()\n\ndef leading(x: None | Right):\n    x.run()\n",
    )?;
    fs::write(
        root.join("b.php"),
        "<?php\nclass PLeft { function go() { return 1; } }\nclass PRight { function go() { return 2; } }\nfunction pdirect(PLeft|PRight $x) { $x->go(); }\nfunction pleading(null|PRight $x) { $x->go(); }\n",
    )?;
    assert!(
        !calls(&root, "direct")?
            .iter()
            .any(|edge| edge.ends_with("exact")),
        "python union"
    );
    assert_eq!(calls(&root, "leading")?, ["a.Right.run exact"]);
    assert!(
        !calls(&root, "pdirect")?
            .iter()
            .any(|edge| edge.ends_with("exact")),
        "php union"
    );
    assert_eq!(calls(&root, "pleading")?, ["PRight.go exact"]);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn python_awaited_async_result_keeps_its_declared_receiver_type() -> TestResult {
    let root = support::exclusive_dir("python-await-result")?;
    fs::write(
        root.join("a.py"),
        "class Response:\n    def finish(self): pass\n\nclass Decoy:\n    def finish(self): pass\n\nclass Awaitable:\n    def finish(self): pass\n    def __await__(self):\n        if False: yield\n        return Response()\n\ndef sync_factory() -> Awaitable:\n    return Awaitable()\n\nasync def wrong():\n    result = await sync_factory()\n    result.finish()\n\ndef replace(fn):\n    async def replacement():\n        return Decoy()\n    return replacement\n\n@replace\nasync def decorated() -> Response:\n    return Response()\n\nasync def decorated_caller():\n    result = await decorated()\n    result.finish()\n\nclass Client:\n    async def __aenter__(self) -> Client:\n        return self\n    async def __aexit__(self, typ, value, traceback):\n        pass\n    async def send(self) -> Response:\n        return Response()\n    async def run(self):\n        result = await self.send()\n        result.finish()\n    def missing_await(self):\n        result = self.send()\n        result.finish()\n\nasync def context():\n    async with Client() as client:\n        await client.send()\n",
    )?;
    let found = calls(&root, "run")?;
    assert!(
        found
            .iter()
            .any(|edge| edge == "a.Response.finish inferred"),
        "{found:?}"
    );
    let wrong = calls(&root, "wrong")?;
    assert!(
        !wrong
            .iter()
            .any(|edge| edge.starts_with("a.Awaitable.finish ")),
        "{wrong:?}"
    );
    let decorated = calls(&root, "decorated_caller")?;
    assert!(
        !decorated
            .iter()
            .any(|edge| edge.starts_with("a.Response.finish ")),
        "{decorated:?}"
    );
    let unawaited = calls(&root, "missing_await")?;
    assert!(
        !unawaited
            .iter()
            .any(|edge| edge.starts_with("a.Response.finish ")),
        "{unawaited:?}"
    );
    let context = calls(&root, "context")?;
    assert!(
        context.iter().any(|edge| edge == "a.Client.send inferred"),
        "{context:?}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn an_own_array_type_is_no_standard_container() -> TestResult {
    let root = support::exclusive_dir("declared-array")?;
    fs::write(
        root.join("a.ts"),
        "class Array<T> { first!: Other; }\nclass Other { go() { return 1 } }\nclass Item { go() { return 2 } }\nexport function own(items: Array<Item>) { for (const x of items as any) { x.go(); } }\n",
    )?;
    assert!(
        !calls(&root, "own")?
            .iter()
            .any(|edge| edge == "a.Item.go exact"),
        "own Array"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
