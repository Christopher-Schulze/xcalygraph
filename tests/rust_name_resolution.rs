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

fn rust_root(label: &str, files: &[(&str, &str)]) -> std::io::Result<std::path::PathBuf> {
    let root = support::exclusive_dir(label)?;
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )?;
    for (path, content) in files {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    Ok(root)
}

fn outgoing(root: &std::path::Path, name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let output = support::command(BINARY, root)
        .args(["show", name, "-C"])
        .arg(root)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn self_super_and_crate_calls_resolve() -> Result<(), Box<dyn std::error::Error>> {
    let root = rust_root(
        "rust-relative",
        &[
            (
                "src/lib.rs",
                "pub mod a;\npub fn root_fn() -> i32 { 1 }\npub fn sup_inline() -> i32 { 2 }\npub fn calls_self() -> i32 { self::root_fn() }\npub fn calls_crate() -> i32 { crate::root_fn() }\nmod inner {\n    pub fn own() -> i32 { 3 }\n    pub fn calls_inline_self() -> i32 { self::own() }\n    pub fn calls_inline_super() -> i32 { super::sup_inline() }\n}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {\n        super::root_fn();\n    }\n}\n",
            ),
            (
                "src/a/mod.rs",
                "pub mod b;\npub fn parent_fn() -> i32 { 1 }\n",
            ),
            (
                "src/a/b.rs",
                "pub fn calls_parent() -> i32 { super::parent_fn() }\n",
            ),
        ],
    )?;
    for (name, want) in [
        ("calls_self", "> root_fn src/lib.rs"),
        ("calls_crate", "> root_fn src/lib.rs"),
        ("calls_inline_self", "> inner::own src/lib.rs"),
        ("calls_inline_super", "> sup_inline src/lib.rs"),
        ("calls_parent", "> parent_fn src/a/mod.rs"),
    ] {
        let text = outgoing(&root, name)?;
        assert!(text.contains(want), "{name}: {text}");
    }
    let text = outgoing(&root, "root_fn")?;
    assert!(text.contains("tests::t"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn one_segment_modules_follow_their_reexports() -> Result<(), Box<dyn std::error::Error>> {
    let root = rust_root(
        "rust-one-segment",
        &[
            (
                "src/lib.rs",
                "pub mod a;\nuse crate::a as m;\npub fn direct() -> i32 { a::pick() }\npub fn imported() -> i32 { m::pick() }\n",
            ),
            ("src/a/mod.rs", "pub mod two;\npub use two::pick;\n"),
            ("src/a/two.rs", "pub fn pick() -> i32 { 1 }\n"),
        ],
    )?;
    for name in ["direct", "imported"] {
        let text = outgoing(&root, name)?;
        assert!(text.contains("pick src/a/two.rs"), "{name}: {text}");
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn an_explicit_reexport_shadows_a_glob() -> Result<(), Box<dyn std::error::Error>> {
    let root = rust_root(
        "rust-explicit-glob",
        &[
            (
                "src/lib.rs",
                "pub mod a;\npub mod b;\npub mod m;\npub fn check() -> i32 { m::pick() }\n",
            ),
            ("src/a.rs", "pub fn pick() -> i32 { 1 }\n"),
            ("src/b.rs", "pub fn pick() -> i32 { 2 }\n"),
            (
                "src/m.rs",
                "pub use crate::a::*;\npub use crate::b::pick;\n",
            ),
        ],
    )?;
    let text = outgoing(&root, "check")?;
    assert!(text.contains("pick src/b.rs"), "{text}");
    assert!(!text.contains("pick src/a.rs"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_self_method_is_no_self_path() -> Result<(), Box<dyn std::error::Error>> {
    let root = rust_root(
        "rust-self-method",
        &[(
            "src/lib.rs",
            "pub struct S;\nimpl S {\n    pub fn entry(&self) -> i32 { self.helper() }\n    fn helper(&self) -> i32 { 1 }\n}\npub fn helper() -> i32 { 2 }\npub fn free_entry() -> i32 { self::helper() }\n",
        )],
    )?;
    let text = outgoing(&root, "S.entry")?;
    assert!(text.contains("> S::helper src/lib.rs:4"), "{text}");
    let text = outgoing(&root, "free_entry")?;
    assert!(text.contains("> helper src/lib.rs:6"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}
