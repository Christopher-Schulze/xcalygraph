//! Rust macro invocations and function calls occupy separate source namespaces.

mod support;

use std::error::Error;
use std::fs;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

#[test]
fn rust_calls_preserve_their_macro_or_value_namespace() -> Result<(), Box<dyn Error>> {
    let builtin = "fn write() {}\nfn helper() {}\n\
        fn entry(output: &mut String) -> std::fmt::Result {\n\
        use std::fmt::Write as _;\nhelper();\nwrite!(output, \"x\")\n}\n";
    let shared = "fn route() -> u8 { 1 }\n\
        macro_rules! route { () => { 2 } }\n\
        fn macro_entry() -> u8 { route!() }\n\
        fn function_entry() -> u8 { route() }\n";
    let local = "macro_rules! route { () => { 2 } }\n\
        fn entry() -> u8 { let route = || 1; route!() }\n";
    for (label, source, caller, target, line) in [
        ("builtin", builtin, "entry", "helper", 2),
        ("macro", shared, "macro_entry", "route", 2),
        ("function", shared, "function_entry", "route", 1),
        ("local", local, "entry", "route", 1),
    ] {
        let fixture = support::exclusive_dir("rust-call-namespace")?;
        fs::write(fixture.join("lib.rs"), source)?;
        let output = support::command(BINARY, &fixture)
            .args(["show", caller, "--all", "-C"])
            .arg(&fixture)
            .output()?;
        let text = String::from_utf8(output.stdout)?;
        assert!(
            output.status.success(),
            "{label}: {text}, {:?}",
            output.stderr
        );
        let edges: Vec<_> = text.lines().filter(|line| line.starts_with("> ")).collect();
        assert_eq!(edges.len(), 1, "{label}: {text}");
        assert!(
            edges[0].starts_with(&format!("> {target} ")),
            "{label}: {text}"
        );
        assert!(edges[0].ends_with(&format!(":{line}")), "{label}: {text}");
        fs::remove_dir_all(fixture)?;
    }
    Ok(())
}
