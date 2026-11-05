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

#[test]
fn rust_paths_follow_module_reexports_across_crates() -> Result<(), Box<dyn Error>> {
    let fixture = support::exclusive_dir("rust-reexports")?;
    for (path, content) in [
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"a\", \"b\", \"facade\", \"other\"]\n",
        ),
        (
            "a/Cargo.toml",
            "[package]\nname = \"a-core\"\nversion = \"0.1.0\"\n",
        ),
        (
            "a/src/lib.rs",
            "mod inner;\npub mod glob_source;\npub use crate::inner::Thing;\npub use self::glob_source::*;\npub use crate::inner::Thing as Renamed;\npub use external_dep::*;\n",
        ),
        (
            "a/src/inner.rs",
            "pub struct Thing;\nimpl Thing {\n    pub fn new() -> Thing { Thing }\n    pub fn run(&self) -> usize { 1 }\n}\n",
        ),
        (
            "a/src/glob_source.rs",
            "pub struct Globbed;\nimpl Globbed { pub fn make() -> Globbed { Globbed } }\n",
        ),
        (
            "other/Cargo.toml",
            "[package]\nname = \"other\"\nversion = \"0.1.0\"\n",
        ),
        (
            "other/src/lib.rs",
            "pub struct Thing;\nimpl Thing {\n    pub fn new() -> Thing { Thing }\n    pub fn run(&self) -> usize { 2 }\n}\n",
        ),
        (
            "facade/Cargo.toml",
            "[package]\nname = \"facade\"\nversion = \"0.1.0\"\n",
        ),
        (
            "facade/src/lib.rs",
            "pub extern crate a_core as core_api;\n",
        ),
        (
            "b/Cargo.toml",
            "[package]\nname = \"b\"\nversion = \"0.1.0\"\n",
        ),
        (
            "b/src/lib.rs",
            "extern crate a_core as renamed_crate;\nuse a_core::Thing;\n\
            pub fn via_use() -> usize { Thing::new().run() }\n\
            pub fn via_facade() -> usize { facade::core_api::Thing::new().run() }\n\
            pub fn via_extern() -> usize { renamed_crate::Thing::new().run() }\n\
            pub fn via_rename() -> usize { a_core::Renamed::new().run() }\n\
            pub fn via_glob() -> a_core::Globbed { a_core::Globbed::make() }\n\
            pub fn via_external_glob() { a_core::Missing::new(); }\n",
        ),
    ] {
        let file = fixture.join(path);
        fs::create_dir_all(file.parent().ok_or("parent")?)?;
        fs::write(file, content)?;
    }
    for (caller, target, line) in [
        ("via_use", "Thing::new", "a/src/inner.rs:3"),
        ("via_facade", "Thing::new", "a/src/inner.rs:3"),
        ("via_extern", "Thing::new", "a/src/inner.rs:3"),
        ("via_rename", "Thing::new", "a/src/inner.rs:3"),
        ("via_glob", "Globbed::make", "a/src/glob_source.rs:2"),
    ] {
        let output = support::command(BINARY, &fixture)
            .args(["show", caller, "--all", "-C"])
            .arg(&fixture)
            .output()?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{caller}: {text}");
        assert!(
            text.contains(&format!("> {target} {line}")),
            "{caller}: {text}"
        );
        assert!(!text.contains("other/src"), "{caller}: {text}");
    }
    let output = support::command(BINARY, &fixture)
        .args(["show", "via_external_glob", "--all", "-C"])
        .arg(&fixture)
        .output()?;
    let text = String::from_utf8(output.stdout)?;
    assert!(!text.contains("new"), "{text}");
    fs::remove_dir_all(fixture)?;
    Ok(())
}

#[test]
fn rust_pattern_names_shadow_only_their_arm() -> Result<(), Box<dyn Error>> {
    let fixture = support::exclusive_dir("rust-pattern-names")?;
    fs::write(
        fixture.join("lib.rs"),
        "pub struct Item;\nimpl Item { pub fn size(&self) -> usize { 1 } }\n\
        pub enum Value { Str(String), One(Item) }\n\
        impl Value { pub fn size(&self) -> usize { 0 } pub fn twin(&self) -> Value { Value::Str(String::new()) } }\n\
        pub fn in_arm(value: Value) -> usize { match value { Value::Str(value) => value.size(), _ => 0 } }\n\
        pub fn after_match(value: Value) -> usize { match &value { Value::Str(value) => 0, _ => 0 }; value.size() }\n\
        pub fn in_consequence(value: Value) -> usize { if let Value::Str(value) = value.twin() { value.size() } else { 0 } }\n\
        pub fn in_else(value: Value) -> usize { if let Value::Str(inner) = value.twin() { 0 } else { value.size() } }\n\
        pub fn let_else(value: Value) -> usize { let Value::Str(value) = value.twin() else { return value.size() }; value.size() }\n",
    )?;
    for (caller, wanted, wanted_twin) in [
        ("in_arm", false, false),
        ("after_match", true, false),
        ("in_consequence", false, true),
        ("in_else", true, true),
        ("let_else", true, true),
    ] {
        let output = support::command(BINARY, &fixture)
            .args(["show", caller, "--all", "-C"])
            .arg(&fixture)
            .output()?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{caller}: {text}");
        assert_eq!(text.contains("> Value::size"), wanted, "{caller}: {text}");
        assert_eq!(
            text.contains("Value::twin"),
            wanted_twin,
            "{caller}: {text}"
        );
    }
    fs::remove_dir_all(fixture)?;
    Ok(())
}
