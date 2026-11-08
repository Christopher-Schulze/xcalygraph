//! A receiver whose value another write of a different type can replace on some path (a branch
//! after an earlier write, a later write in a loop) has no proven type: its calls are inferred.
//! Straight-line writes keep the last value exact.

mod support;

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

type TestResult = Result<(), Box<dyn Error>>;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `name conf` of the outgoing `send` calls of `symbol`.
fn calls(root: &Path, symbol: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), root)
        .args(["show", symbol, "--json", "--all", "-C"])
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
        if field("kind") == "calls" && field("name").ends_with(".send") {
            found.push(format!("{} {}", field("name"), field("conf")));
        }
    }
    found.sort();
    Ok(found)
}

#[test]
fn javascript_reassigned_receivers_are_not_exact() -> TestResult {
    let fixture = Fixture(support::exclusive_dir("js-reassigned-receivers")?);
    let root = &fixture.0;
    fs::write(
        root.join("app.js"),
        "class Client { send() {} }\nclass Other { send() {} }\n\
function straight() { let c = new Client(); c = new Other(); c.send() }\n\
function branch(flag) { let c = new Client(); if (flag) { c = new Other() } c.send() }\n\
function single() { const c = new Client(); c.send() }\n",
    )?;
    assert_eq!(calls(root, "straight")?, ["app.Other.send exact"]);
    assert_eq!(calls(root, "branch")?, ["app.Other.send inferred"]);
    assert_eq!(calls(root, "single")?, ["app.Client.send exact"]);
    Ok(())
}

#[test]
fn python_reassigned_receivers_are_not_exact() -> TestResult {
    let fixture = Fixture(support::exclusive_dir("py-reassigned-receivers")?);
    let root = &fixture.0;
    fs::write(
        root.join("app.py"),
        "class Client:\n    def send(self): pass\nclass Other:\n    def send(self): pass\n\
def branch(flag):\n    c = Client()\n    if flag:\n        c = Other()\n    c.send()\n\
def loop(items):\n    c = Client()\n    for item in items:\n        c.send()\n        c = Other()\n\
def single():\n    c = Client()\n    c.send()\n\
def convert(c: Other):\n    c = Client()\n    c.send()\n",
    )?;
    assert_eq!(calls(root, "branch")?, ["app.Other.send inferred"]);
    assert_eq!(calls(root, "loop")?, ["app.Client.send inferred"]);
    assert_eq!(calls(root, "single")?, ["app.Client.send exact"]);
    assert_eq!(calls(root, "convert")?, ["app.Client.send exact"]);
    Ok(())
}
