//! Calls on enum-variant field bindings resolve through the declared variant field
//! type: `Tool::Hammer(h) => h.hit()` reaches `Hammer::hit`, never a same-named decoy.

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

fn tool_source() -> String {
    "enum Tool { Hammer(Hammer), Wrench(Wrench) }\n\
     struct Hammer;\n\
     struct Wrench;\n\
     struct Decoy;\n\
     impl Hammer { fn hit(self) {} }\n\
     impl Wrench { fn turn(self) {} }\n\
     impl Decoy { fn hit(self) {} }\n\
     impl Tool {\n\
     \x20   fn use_tool(self) {\n\
     \x20       match self {\n\
     \x20           Tool::Hammer(h) => h.hit(),\n\
     \x20           Tool::Wrench(w) => w.turn(),\n\
     \x20       }\n\
     \x20   }\n\
     }\n"
    .to_owned()
}

#[test]
fn rust_variant_field_calls_resolve_to_declared_field_type() -> TestResult {
    let fixture = Fixture(support::exclusive_dir("rs-variant-field")?);
    let root = &fixture.0;
    fs::write(root.join("main.rs"), tool_source())?;
    assert_eq!(
        calls_of(root, "use_tool")?,
        ["crate::Hammer::hit exact", "crate::Wrench::turn exact",]
    );
    Ok(())
}

#[test]
fn rust_unqualified_variant_resolves_through_self_scrutinee() -> TestResult {
    let fixture = Fixture(support::exclusive_dir("rs-variant-self")?);
    let root = &fixture.0;
    fs::write(
        root.join("main.rs"),
        "use Tool::*;\n\
         enum Tool { Hammer(Hammer) }\n\
         struct Hammer;\n\
         struct Decoy;\n\
         impl Hammer { fn hit(self) {} }\n\
         impl Decoy { fn hit(self) {} }\n\
         impl Tool {\n\
         \x20   fn use_tool(self) {\n\
         \x20       match self {\n\
         \x20           Hammer(h) => h.hit(),\n\
         \x20       }\n\
         \x20   }\n\
         }\n",
    )?;
    assert_eq!(calls_of(root, "use_tool")?, ["crate::Hammer::hit exact"]);
    Ok(())
}

#[test]
fn rust_variant_field_resolves_across_files() -> TestResult {
    let fixture = Fixture(support::exclusive_dir("rs-variant-xfile")?);
    let root = &fixture.0;
    fs::create_dir_all(root.join("src"))?;
    for (path, content) in [
        (
            "Cargo.toml",
            "[package]\nname = \"vprobe\"\nversion = \"0.1.0\"\n",
        ),
        (
            "src/tool.rs",
            "pub enum Tool { Hammer(Hammer) }\n\
             pub struct Hammer;\n\
             pub struct Decoy;\n\
             impl Hammer { pub fn hit(self) {} }\n\
             impl Decoy { pub fn hit(self) {} }\n",
        ),
        (
            "src/main.rs",
            "mod tool;\n\
             use tool::Tool;\n\
             fn run(t: Tool) {\n\
             \x20   match t {\n\
             \x20       tool::Tool::Hammer(h) => h.hit(),\n\
             \x20   }\n\
             }\n\
             fn run2(t: Tool) {\n\
             \x20   match t {\n\
             \x20       Tool::Hammer(h) => h.hit(),\n\
             \x20   }\n\
             }\n",
        ),
    ] {
        fs::write(root.join(path), content)?;
    }
    assert_eq!(calls_of(root, "run")?, ["tool::Hammer::hit exact"]);
    assert_eq!(calls_of(root, "run2")?, ["tool::Hammer::hit exact"]);
    Ok(())
}
