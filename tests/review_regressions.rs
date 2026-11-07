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
    assert!(star.contains("<doc? docs/env.md:3"), "{text}");
    let text = query(&root, &["find", "LONE_VALUE"])?;
    assert!(
        text.lines().next().unwrap_or_default().ends_with("-t -doc"),
        "{text}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn environment_documents_require_a_whole_identifier_and_obey_filters() -> TestResult {
    let root = support::exclusive_dir("env-star-document-filter")?;
    write(
        &root,
        &[
            (
                "cfg.py",
                b"import os\ndef load():\n    return os.environ.get('APP_TOKEN')\n",
            ),
            ("docs/a.md", b"# NOT_APP_TOKEN\n"),
            ("docs/b.md", b"# APP_TOKEN settings\n"),
        ],
    )?;
    let text = query(&root, &["find", "APP_TOKEN"])?;
    let star = text.lines().next().unwrap_or_default();
    assert!(star.contains("<doc? docs/b.md:1"), "{text}");
    assert!(!star.contains("docs/a.md"), "{text}");
    for filter in [["-p", "cfg.py"], ["-l", "python"], ["-k", "fn"]] {
        let text = query(&root, &["find", "APP_TOKEN", filter[0], filter[1]])?;
        let star = text.lines().next().unwrap_or_default();
        assert!(star.contains("-doc") && !star.contains("<doc"), "{text}");
    }
    let exact = query(&root, &["find", "APP_TOKEN", "--exact"])?;
    let star = exact.lines().next().unwrap_or_default();
    assert!(star.contains("-doc") && !star.contains("<doc?"), "{exact}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn environment_stars_fit_the_entire_answer_budget() -> TestResult {
    let root = support::exclusive_dir("env-star-budget")?;
    write(
        &root,
        &[(
            "config\\escaped.py",
            b"import os\ndef load():\n    return os.environ.get('APP_TOKEN')\n",
        )],
    )?;
    let full = query(&root, &["find", "APP_TOKEN", "-b", "600"])?;
    assert!(full.contains("<read config\\\\escaped.py:3"), "{full}");
    for budget in [1usize, 2, 3, 10, 20, 40, 100, 600] {
        let text = query(&root, &["find", "APP_TOKEN", "-b", &budget.to_string()])?;
        assert!(text.len() <= budget * 3, "budget {budget}: {text:?}");
        if text.contains("<read") {
            assert!(text.contains("config\\\\escaped.py"), "{text}");
        }
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn environment_star_marks_an_inferred_test_relation() -> TestResult {
    let root = support::exclusive_dir("env-star-inferred-test")?;
    write(
        &root,
        &[
            (
                "service.js",
                b"function Service() {}\nService.prototype.render = function render() { return process.env.INFERRED_KEY; };\nmodule.exports = Service;\n",
            ),
            (
                "tests/service.test.js",
                b"var Service = require('../service');\nfunction test_read(unknown) { unknown.render(); }\n",
            ),
        ],
    )?;
    let text = query(&root, &["find", "INFERRED_KEY"])?;
    let star = text.lines().next().unwrap_or_default();
    assert!(star.contains("<t? tests/service.test.js:2"), "{text}");
    let exact = query(&root, &["find", "INFERRED_KEY", "--exact"])?;
    assert!(
        !exact.lines().next().unwrap_or_default().contains("<t?"),
        "{exact}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn destructured_exports_resolve_to_each_apps_own_binding() -> TestResult {
    let root = support::exclusive_dir("destructured-exports")?;
    write(
        &root,
        &[
            ("app-a/auth.ts", b"export const { useUser, useLogin } = makeAuth();\n"),
            ("app-b/auth.ts", b"export const { useUser, useLogin } = makeAuth();\n"),
            (
                "app-a/main.ts",
                b"import { useUser, useLogin as login } from './auth';\nexport function entryA() { return useUser() + login(); }\nexport function shadow(useUser: () => void) { useUser(); }\n",
            ),
            (
                "app-b/main.ts",
                b"import { useUser } from './auth';\nexport function entryB() { return useUser(); }\n",
            ),
            ("other.ts", b"export function useUser() {}\n"),
        ],
    )?;
    let first = query(&root, &["show", "entryA", "--exact"])?;
    assert!(first.contains("> useUser app-a/auth.ts:1"), "{first}");
    assert!(first.contains("> useLogin :1"), "{first}");
    assert!(
        !first.contains("app-b/auth.ts") && !first.contains("other.ts"),
        "{first}"
    );
    let second = query(&root, &["show", "entryB", "--exact"])?;
    assert!(second.contains("> useUser app-b/auth.ts:1"), "{second}");
    assert!(
        !second.contains("app-a/auth.ts") && !second.contains("other.ts"),
        "{second}"
    );
    let shadow = query(&root, &["show", "shadow", "--exact"])?;
    assert!(!shadow.contains("> useUser "), "{shadow}");
    fs::remove_dir_all(root)?;
    Ok(())
}
