//! False facts and false absences found by source review, one reproduction each.

mod support;

use std::error::Error;
use std::fs;
use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

type TestResult = Result<(), Box<dyn Error>>;

fn write(root: &Path, files: &[(&str, &[u8])]) -> TestResult {
    for (path, content) in files {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    Ok(())
}

fn query(root: &Path, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = support::command(BINARY, root)
        .args(args)
        .arg("-C")
        .arg(root)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn an_export_alias_finds_its_declaration() -> TestResult {
    let root = support::exclusive_dir("export-alias")?;
    write(
        &root,
        &[(
            "a.ts",
            b"function hidden() { return 1; }\nexport { hidden as Public };\n",
        )],
    )?;
    let text = query(&root, &["show", "Public"])?;
    assert!(
        text.contains("fn hidden") || text.contains("hidden"),
        "{text}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn exported_aliases_report_ambiguity_and_respect_file_qualification() -> TestResult {
    let root = support::exclusive_dir("export-alias-ambiguity")?;
    write(
        &root,
        &[
            (
                "a.ts",
                b"function first() {}\nexport { first as Shared };\n",
            ),
            (
                "b.ts",
                b"function second() {}\nexport { second as Shared };\n",
            ),
        ],
    )?;
    let ambiguous = support::command(BINARY, &root)
        .args(["show", "Shared", "-C"])
        .arg(&root)
        .output()?;
    let candidates = String::from_utf8(ambiguous.stdout)?;
    assert_eq!(ambiguous.status.code(), Some(3), "{candidates}");
    assert!(
        candidates.contains("first") && candidates.contains("second"),
        "{candidates}"
    );
    let first = query(&root, &["show", "a.ts::Shared"])?;
    assert!(
        first.contains("first") && !first.contains("second"),
        "{first}"
    );
    let second = query(&root, &["show", "Shared", "-p", "b.ts"])?;
    assert!(
        second.contains("second") && !second.contains("first"),
        "{second}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_damaged_alias_search_shard_is_not_reported_as_absence() -> TestResult {
    let work = support::exclusive_dir("export-alias-damaged")?;
    let root = work.join("source");
    let index = work.join("index");
    fs::create_dir(&root)?;
    write(
        &root,
        &[(
            "a.ts",
            b"function first() {}\nexport { first as Shared };\n",
        )],
    )?;
    let run = |args: &[&str]| {
        support::command(BINARY, &work)
            .args(args)
            .arg("-C")
            .arg(&root)
            .arg("--index-dir")
            .arg(&index)
            .output()
    };
    assert!(run(&["index"])?.status.success());
    let store = fs::read_dir(&index)?.next().ok_or("missing index")??.path();
    let shard = xxhash_rust::xxh3::xxh3_64(b"a.ts") % 64;
    let damaged = if shard == 0 { 1 } else { 0 };
    fs::write(store.join(format!("facts/{damaged:02}")), b"corrupt")?;
    let output = run(&["show", "MissingAlias", "--read-only"])?;
    assert_ne!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stderr).contains("fact shard"));
    fs::remove_dir_all(work)?;
    Ok(())
}

const SHADOW: &[u8] = b"<script>\nclass Outer { run() { return 1 } }\nclass Inner { run() { return 2 } }\nfunction check() { const value = new Outer(); { const value = new Inner(); value.run(); } value.run(); }\n</script>";

#[test]
fn html_scripts_keep_block_scopes() -> TestResult {
    let root = support::exclusive_dir("html-scope")?;
    write(&root, &[("page.html", SHADOW)])?;
    let text = query(&root, &["show", "check"])?;
    assert!(text.contains("Outer.run"), "{text}");
    assert!(text.contains("Inner.run"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_lossy_html_script_proves_no_receiver() -> TestResult {
    let root = support::exclusive_dir("html-lossy")?;
    let mut page = vec![0xff];
    page.extend_from_slice(SHADOW);
    write(&root, &[("page.html", &page)])?;
    let json = query(&root, &["show", "check", "--json"])?;
    assert!(
        !json
            .lines()
            .any(|line| line.contains("run") && line.contains("\"conf\":\"exact\"")),
        "{json}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_named_environment_variable_shows_its_reader_test_and_document() -> TestResult {
    let root = support::exclusive_dir("env-star")?;
    write(
        &root,
        &[
            (
                "cfg.py",
                b"import os\n\ndef load():\n    return os.environ.get(\"APP_TOKEN\")\n",
            ),
            (
                "tests/test_cfg.py",
                b"from cfg import load\n\ndef test_load():\n    load()\n",
            ),
            (
                "docs/env.md",
                b"# Variables\n\n## APP_TOKEN\n\nThe token.\n",
            ),
            (
                "other.py",
                b"import os\n\ndef bare():\n    return os.environ.get(\"LONE_VALUE\")\n",
            ),
        ],
    )?;
    let text = query(&root, &["find", "APP_TOKEN", "token"])?;
    let star = text.lines().next().unwrap_or_default();
    assert!(star.starts_with("~ APP_TOKEN <read cfg.py:4"), "{text}");
    assert!(star.contains("<t tests/test_cfg.py:3"), "{text}");
    assert!(star.contains("<doc docs/env.md:3"), "{text}");
    let text = query(&root, &["find", "LONE_VALUE"])?;
    assert!(
        text.lines().next().unwrap_or_default().ends_with("-t -doc"),
        "{text}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
