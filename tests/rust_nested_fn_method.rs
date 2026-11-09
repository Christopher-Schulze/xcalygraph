//! A function nested in a method body is a local function, not a method: calls to it
//! resolve lexically (the glob.rs:1042 shape: unique same-file nested fn).

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

/// `name conf` of the outgoing `calls` edges of `symbol`.
fn calls_of(root: &Path, symbol: &str) -> Result<Vec<String>, Box<dyn Error>> {
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
        if field("kind") == "calls" {
            found.push(format!("{} {}", field("name"), field("conf")));
        }
    }
    found.sort();
    Ok(found)
}

#[test]
fn rust_nested_fn_call_inside_method_resolves() -> TestResult {
    let fixture = Fixture(support::exclusive_dir("rs-nested-fn")?);
    let root = &fixture.0;
    fs::write(
        root.join("main.rs"),
        "struct Parser;\n\
         impl Parser {\n\
         \x20   fn parse(&mut self) -> i32 {\n\
         \x20       fn helper(x: i32) -> i32 {\n\
         \x20           x + 1\n\
         \x20       }\n\
         \x20       helper(41)\n\
         \x20   }\n\
         }\n",
    )?;
    assert_eq!(
        calls_of(root, "parse")?,
        ["crate::Parser::parse::helper exact"]
    );
    Ok(())
}
