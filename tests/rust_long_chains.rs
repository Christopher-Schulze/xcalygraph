//! Receiver chains longer than the qualifier text budget still resolve: extraction records
//! the chain steps instead of clipped source text.

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

/// `name conf` of the outgoing `finish` calls of `symbol`.
fn finish_calls(root: &Path, symbol: &str) -> Result<Vec<String>, Box<dyn Error>> {
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
        if field("kind") == "calls" && field("name").ends_with("finish") {
            found.push(format!("{} {}", field("name"), field("conf")));
        }
    }
    found.sort();
    Ok(found)
}

fn eighteen_step_chain() -> String {
    let mut out = String::new();
    for i in 0..18 {
        out.push_str(&format!("struct S{i};\n"));
    }
    for i in 0..17 {
        out.push_str(&format!(
            "impl S{i} {{ fn step{}(self) -> S{} {{ S{} }} }}\n",
            i + 1,
            i + 1,
            i + 1
        ));
    }
    out.push_str("impl S17 { fn finish(self) {} }\n");
    out.push_str("struct Decoy;\nimpl Decoy { fn finish(self) {} }\n");
    out.push_str("fn run() {\n    S0");
    for i in 1..18 {
        out.push_str(&format!(".step{i}()"));
    }
    out.push_str(".finish();\n}\n");
    out
}

#[test]
fn rust_long_type_changing_chain_resolves_final_call() -> TestResult {
    let fixture = Fixture(support::exclusive_dir("rs-long-chain")?);
    let root = &fixture.0;
    fs::write(root.join("main.rs"), eighteen_step_chain())?;
    assert_eq!(finish_calls(root, "run")?, ["crate::S17::finish inferred"]);
    Ok(())
}

#[test]
fn rust_long_argument_does_not_hide_final_call() -> TestResult {
    let fixture = Fixture(support::exclusive_dir("rs-long-chain-arg")?);
    let root = &fixture.0;
    let padding = "a".repeat(100);
    fs::write(
        root.join("main.rs"),
        format!(
            "struct Builder;\nstruct Built;\nstruct Decoy;\nimpl Builder {{ fn configure(self, name: &str) -> Built {{ Built }} }}\nimpl Built {{ fn finish(self) {{}} }}\nimpl Decoy {{ fn finish(self) {{}} }}\nfn run() {{\n    Builder.configure(\"{padding}\").finish();\n}}\n"
        ),
    )?;
    assert_eq!(
        finish_calls(root, "run")?,
        ["crate::Built::finish inferred"]
    );
    Ok(())
}
