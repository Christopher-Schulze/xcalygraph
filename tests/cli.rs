//! End-to-end checks of the binary: stdout, stderr and exit codes.

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

type TestResult = Result<(), Box<dyn Error>>;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

/// Runs the binary with a HOME that never points at the real one.
fn run(args: &[&str]) -> io::Result<Output> {
    Command::new(BINARY)
        .args(args)
        .env(
            "HOME",
            std::env::temp_dir().join("xcalygraph-it-empty-home"),
        )
        .output()
}

#[test]
fn version_prints_binary_index_and_skill_versions() -> TestResult {
    for args in [&["version"][..], &["--version"], &["-V"]] {
        let output = run(args)?;
        assert_eq!(output.status.code(), Some(0), "args {args:?}");
        let text = String::from_utf8(output.stdout)?;
        assert!(
            text.starts_with(&format!(
                "xcalygraph {}\nindex format ",
                env!("CARGO_PKG_VERSION")
            )),
            "{text}"
        );
        assert!(text.ends_with("skill not installed\n"), "{text}");
        assert!(output.stderr.is_empty(), "args {args:?}");
    }
    Ok(())
}

#[test]
fn help_prints_overview_and_command_entries() -> TestResult {
    let overview = run(&["help"])?;
    assert_eq!(overview.status.code(), Some(0));
    assert!(
        String::from_utf8(overview.stdout)?
            .starts_with("xcalygraph <command> [args] [flags]\ncommands:\n")
    );

    let entry = run(&["version", "--help"])?;
    assert_eq!(entry.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(entry.stdout)?,
        "xcalygraph version\n  binary, index format and installed skill versions; also -V, --version\n"
    );
    Ok(())
}

#[test]
fn usage_errors_exit_2_with_one_stderr_line() -> TestResult {
    let cases: [(&[&str], &str); 4] = [
        (&[], "xcalygraph: missing command; run 'xcalygraph help'\n"),
        (
            &["nope"],
            "xcalygraph: unknown command 'nope'; run 'xcalygraph help'\n",
        ),
        (
            &["find"],
            "xcalygraph: 'find' needs an argument; run 'xcalygraph help'\n",
        ),
        (
            &["version", "x"],
            "xcalygraph: unexpected argument 'x'; run 'xcalygraph help'\n",
        ),
    ];
    for (args, message) in cases {
        let output = run(args)?;
        assert_eq!(output.status.code(), Some(2), "args {args:?}");
        assert!(output.stdout.is_empty(), "args {args:?}");
        assert_eq!(String::from_utf8(output.stderr)?, message);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn closed_stdout_is_not_an_error() -> TestResult {
    let mut child = Command::new(BINARY)
        .arg("help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    drop(child.stdout.take());
    let output = child.wait_with_output()?;
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    Ok(())
}

/// Isolated temporary directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> io::Result<Self> {
        let path =
            std::env::temp_dir().join(format!("xcalygraph-it-{}-{label}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
        fs::create_dir_all(&path)?;
        Ok(Self(path.canonicalize()?))
    }

    fn write_bytes(&self, relative: &str, content: &[u8]) -> io::Result<()> {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)
    }

    fn write(&self, relative: &str, content: &str) -> io::Result<()> {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_in(root: &Path, index: &Path, args: &[&str]) -> io::Result<Output> {
    Command::new(BINARY)
        .args(args)
        .arg("-C")
        .arg(root)
        .arg("--index-dir")
        .arg(index)
        .env("HOME", index)
        .output()
}

#[test]
fn index_builds_refreshes_and_drops() -> TestResult {
    let dir = TempDir::new("index")?;
    dir.write("root/main.go", "package main\n")?;
    dir.write("root/.env", "SECRET=1\n")?;
    let (root, index) = (dir.0.join("root"), dir.0.join("index"));
    let first = run_in(&root, &index, &["index"])?;
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        String::from_utf8(first.stdout)?,
        "files 2 changed 2 deleted 0\n"
    );
    fs::remove_file(root.join(".env"))?;
    let second = run_in(&root, &index, &["index"])?;
    assert_eq!(
        String::from_utf8(second.stdout)?,
        "files 1 changed 0 deleted 1\n"
    );
    let dropped = run_in(&root, &index, &["index", "--drop"])?;
    assert_eq!(String::from_utf8(dropped.stdout)?, "index dropped\n");
    let again = run_in(&root, &index, &["index", "--drop"])?;
    assert_eq!(String::from_utf8(again.stdout)?, "no index\n");
    Ok(())
}

#[test]
fn missing_root_is_a_runtime_error() -> TestResult {
    let dir = TempDir::new("missing-root")?;
    let output = run_in(&dir.0.join("absent"), &dir.0.join("index"), &["index"])?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.starts_with("xcalygraph: root "));
    Ok(())
}

#[test]
fn install_writes_the_skill_and_version_reports_mismatches() -> TestResult {
    let dir = TempDir::new("install")?;
    let home = dir.0.join("home");
    let with_home = |args: &[&str]| Command::new(BINARY).args(args).env("HOME", &home).output();
    // The Claude Code link appears without a flag once `~/.claude` exists.
    fs::create_dir_all(home.join(".claude"))?;
    let installed = with_home(&["install"])?;
    assert_eq!(
        installed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    let text = String::from_utf8(installed.stdout)?;
    assert!(
        text.contains("binary ") && text.contains("written") && text.contains("created"),
        "{text}"
    );
    // The binary lands in ~/.local/bin and runs from there.
    let copied = home.join(".local/bin/xcalygraph");
    let copied_version = Command::new(&copied).arg("version").output()?;
    assert!(String::from_utf8(copied_version.stdout)?.starts_with("xcalygraph "));
    let skill = fs::read_to_string(home.join(".agents/skills/xcalygraph/SKILL.md"))?;
    assert!(skill.starts_with("---\nname: xcalygraph\n"));
    assert!(
        fs::symlink_metadata(home.join(".claude/skills/xcalygraph"))?
            .file_type()
            .is_symlink()
    );
    let version = String::from_utf8(with_home(&["version"])?.stdout)?;
    assert!(
        version.ends_with(&format!("skill {}\n", env!("CARGO_PKG_VERSION"))),
        "{version}"
    );
    fs::write(home.join(".agents/skills/xcalygraph/.version"), "0.0.1\n")?;
    let stale = String::from_utf8(with_home(&["version"])?.stdout)?;
    assert!(stale.contains("! installed skill 0.0.1"), "{stale}");
    let bad = with_home(&["install", "--link", "vim"])?;
    assert_eq!(bad.status.code(), Some(2));
    let skill_only =
        String::from_utf8(with_home(&["install", "--skill-only", "--no-link"])?.stdout)?;
    assert!(
        skill_only.starts_with("skill ")
            && !skill_only.contains("binary")
            && !skill_only.contains("link"),
        "{skill_only}"
    );
    Ok(())
}

#[test]
fn update_check_compares_with_the_latest_release() -> TestResult {
    let dir = TempDir::new("update")?;
    dir.write("latest.json", "{\"tag_name\": \"v0.0.1\", \"assets\": []}")?;
    let output = Command::new(BINARY)
        .args(["update", "--check"])
        .env("HOME", dir.0.join("home"))
        .env(
            "XCALYGRAPH_UPDATE_URL",
            format!("file://{}", dir.0.join("latest.json").display()),
        )
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("is current"), "{text}");
    // Without --check an up-to-date binary still refreshes a stale skill.
    let skill = dir.0.join("home/.agents/skills/xcalygraph");
    fs::create_dir_all(&skill)?;
    fs::write(skill.join(".version"), "0.0.1\n")?;
    let refreshed = Command::new(BINARY)
        .arg("update")
        .env("HOME", dir.0.join("home"))
        .env(
            "XCALYGRAPH_UPDATE_URL",
            format!("file://{}", dir.0.join("latest.json").display()),
        )
        .output()?;
    let refreshed_text = String::from_utf8(refreshed.stdout)?;
    assert!(refreshed_text.contains("written"), "{refreshed_text}");
    assert_eq!(
        fs::read_to_string(skill.join(".version"))?.trim(),
        env!("CARGO_PKG_VERSION")
    );
    dir.write("latest.json", "{\"tag_name\": \"v99.0.0\", \"assets\": []}")?;
    let newer = Command::new(BINARY)
        .args(["update", "--check"])
        .env("HOME", dir.0.join("home"))
        .env(
            "XCALYGRAPH_UPDATE_URL",
            format!("file://{}", dir.0.join("latest.json").display()),
        )
        .output()?;
    assert!(String::from_utf8(newer.stdout)?.contains("update available"));
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> io::Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .output()?;
    if status.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&status.stderr).into_owned(),
        ))
    }
}

#[test]
fn impact_of_a_diff_lists_changes_dependents_tests_and_run_lines() -> TestResult {
    let dir = TempDir::new("impact")?;
    let root = dir.0.join("repo");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write(
        "repo/store/store.go",
        "package store\n\nfunc Open(path string) int {\n\treturn len(path)\n}\n",
    )?;
    dir.write(
        "repo/store/use.go",
        "package store\n\nfunc Warm() int {\n\treturn Open(\"x\")\n}\n",
    )?;
    dir.write(
        "repo/store/store_test.go",
        "package store\n\nimport \"testing\"\n\nfunc TestWarm(t *testing.T) {\n\tWarm()\n}\n",
    )?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "init"])?;
    dir.write(
        "repo/store/store.go",
        "package store\n\nfunc Open(path string, mode int) int {\n\treturn len(path) + mode\n}\n",
    )?;
    let output = run_in(&root, &dir.0.join("index"), &["impact", "--diff"])?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.contains("~ store.Open store/store.go:3 sig\n"),
        "{text}"
    );
    assert!(text.contains("< 1 store.Warm store/use.go:3\n"), "{text}");
    assert!(
        text.contains("t store.TestWarm store/store_test.go:5\n"),
        "{text}"
    );
    assert!(
        text.contains("r go test ./store -run '^(TestWarm)$'\n"),
        "{text}"
    );
    let named = run_in(&root, &dir.0.join("index"), &["impact", "store.Warm"])?;
    let text = String::from_utf8(named.stdout)?;
    assert!(text.starts_with("~ store.Warm store/use.go:3\n"), "{text}");
    assert!(text.contains("t store.TestWarm"), "{text}");
    let missing = run_in(&root, &dir.0.join("index"), &["impact"])?;
    assert_eq!(missing.status.code(), Some(1));
    Ok(())
}

#[test]
fn audit_reports_dead_code_stale_references_coupling_and_hotspots() -> TestResult {
    let dir = TempDir::new("audit")?;
    let root = dir.0.join("repo");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write("repo/app/app.go", "package app\n\nfunc Run() int {\n\treturn helper()\n}\n\nfunc helper() int { return 1 }\n\nfunc unusedHelper() int { return 2 }\n")?;
    dir.write(
        "repo/app/config.go",
        "package app\n\nconst Name = \"app\"\n",
    )?;
    dir.write(
        "repo/README.md",
        "# App\n\nSee [the guide](docs/guide.md) and [app](app/app.go).\n",
    )?;
    dir.write("repo/docs/keep.md", "# Keep\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "init"])?;
    for round in 0..3 {
        dir.write("repo/app/app.go", &format!("package app\n\nfunc Run() int {{\n\treturn helper() + {round}\n}}\n\nfunc helper() int {{ return 1 }}\n\nfunc unusedHelper() int {{ return 2 }}\n"))?;
        dir.write(
            "repo/app/config.go",
            &format!("package app\n\nconst Name = \"app{round}\"\n"),
        )?;
        git(&root, &["commit", "-q", "-am", "change"])?;
    }
    let output = run_in(&root, &dir.0.join("index"), &["audit"])?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.contains("dead\napp.unusedHelper app/app.go:9\n"),
        "{text}"
    );
    assert!(!text.contains("app.helper "), "{text}");
    assert!(
        text.contains("stale\ndocs/guide.md README.md:3\n"),
        "{text}"
    );
    assert!(
        text.contains("coupling\napp/app.go app/config.go 4\n"),
        "{text}"
    );
    assert!(text.contains("hotspots\napp/app.go 4 commits\n"), "{text}");
    Ok(())
}

#[test]
fn impact_on_jvm_tests_prints_gradle_and_maven_run_lines() -> TestResult {
    let dir = TempDir::new("jvm-impact")?;
    let source = "package com.example;\n\npublic class Store {\n    public int open() {\n        return 1;\n    }\n}\n";
    let test = "package com.example;\n\nimport org.junit.jupiter.api.Test;\n\nclass StoreTest {\n    @Test\n    void opens() {\n        new Store().open();\n        Store store = new Store();\n        store.open();\n    }\n}\n";
    for (project, manifest) in [("maven", "pom.xml"), ("gradle", "build.gradle.kts")] {
        dir.write(&format!("{project}/{manifest}"), "\n")?;
        dir.write(
            &format!("{project}/src/main/java/com/example/Store.java"),
            source,
        )?;
        dir.write(
            &format!("{project}/src/test/java/com/example/StoreTest.java"),
            test,
        )?;
    }
    let maven = run_in(
        &dir.0.join("maven"),
        &dir.0.join("index"),
        &["impact", "Store.open"],
    )?;
    let text = String::from_utf8(maven.stdout)?;
    assert!(text.contains("t com.example.StoreTest.opens "), "{text}");
    assert!(
        text.contains("r mvn test -Dtest=com.example.StoreTest\n"),
        "{text}"
    );
    let gradle = run_in(
        &dir.0.join("gradle"),
        &dir.0.join("index"),
        &["impact", "Store.open"],
    )?;
    let text = String::from_utf8(gradle.stdout)?;
    assert!(
        text.contains("r ./gradlew test --tests com.example.StoreTest\n"),
        "{text}"
    );
    Ok(())
}

#[test]
fn impact_on_dotnet_tests_prints_dotnet_test_filters() -> TestResult {
    let dir = TempDir::new("dotnet-impact")?;
    dir.write("repo/src/App/App.csproj", "<Project />\n")?;
    dir.write(
        "repo/src/App/Store.cs",
        "namespace App;\n\npublic class Store\n{\n    public int Open() => 1;\n}\n",
    )?;
    dir.write("repo/tests/App.Tests/App.Tests.csproj", "<Project />\n")?;
    dir.write(
        "repo/tests/App.Tests/StoreTests.cs",
        "using App;\nusing Xunit;\n\nnamespace App.Tests;\n\npublic class StoreTests\n{\n    [Fact]\n    public void Opens()\n    {\n        var store = new Store();\n        store.Open();\n    }\n}\n",
    )?;
    let output = run_in(
        &dir.0.join("repo"),
        &dir.0.join("index"),
        &["impact", "Store.Open"],
    )?;
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("t App.Tests.StoreTests.Opens "), "{text}");
    assert!(
        text.contains("r dotnet test tests/App.Tests --filter \"FullyQualifiedName~App.Tests.StoreTests.Opens\"\n"),
        "{text}"
    );
    Ok(())
}

#[test]
fn impact_on_script_language_tests_prints_their_runners() -> TestResult {
    let dir = TempDir::new("script-impact")?;
    dir.write(
        "repo/lib/store/repo.ex",
        "defmodule Store.Repo do\n  def get(rows, id), do: Map.get(rows, id)\nend\n",
    )?;
    dir.write(
        "repo/test/repo_test.exs",
        "defmodule Store.RepoTest do\n  use ExUnit.Case\n  alias Store.Repo\n\n  test \"get\" do\n    assert Repo.get(%{}, 1) == nil\n  end\nend\n",
    )?;
    dir.write(
        "repo/src/repo.zig",
        "pub fn count(n: u32) u32 {\n    return n;\n}\n\ntest \"count\" {\n    _ = count(1);\n}\n",
    )?;
    dir.write(
        "repo/src/Store.php",
        "<?php\nnamespace App;\n\nclass Store\n{\n    public function open(): int { return 1; }\n}\n",
    )?;
    dir.write(
        "repo/tests/StoreTest.php",
        "<?php\nnamespace App;\n\nclass StoreTest\n{\n    public function testOpen(): void { (new Store())->open(); }\n}\n",
    )?;
    let root = dir.0.join("repo");
    let index = dir.0.join("index");
    let elixir = String::from_utf8(run_in(&root, &index, &["impact", "Store.Repo.get"])?.stdout)?;
    assert!(
        elixir.contains("r mix test test/repo_test.exs\n"),
        "{elixir}"
    );
    let zig = String::from_utf8(run_in(&root, &index, &["impact", "src/repo.count"])?.stdout)?;
    assert!(zig.contains("r zig build test\n"), "{zig}");
    let php = String::from_utf8(run_in(&root, &index, &["impact", "Store.open"])?.stdout)?;
    assert!(
        php.contains("r vendor/bin/phpunit --filter 'testOpen'\n"),
        "{php}"
    );
    Ok(())
}

/// A minimal DOCX: one `Heading1` paragraph and one body paragraph.
fn docx(heading: &str, body: &str) -> Vec<u8> {
    use std::io::Write;
    let document = format!(
        "<w:document xmlns:w=\"w\"><w:body><w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>{heading}</w:t></w:r></w:p><w:p><w:r><w:t>{body}</w:t></w:r></w:p></w:body></w:document>"
    );
    let mut buffer = io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let written = writer
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .is_ok()
            && writer.write_all(document.as_bytes()).is_ok()
            && writer.finish().is_ok();
        assert!(written);
    }
    buffer.into_inner()
}

#[test]
fn documents_and_assets_feed_search_references_and_audit() -> TestResult {
    let dir = TempDir::new("documents")?;
    let root = dir.0.join("repo");
    dir.write(
        "repo/docs/guide.rst",
        "Guide\n=====\n\nThe ingestion pipeline throttles ``Store.open``.\n\n.. image:: ../assets/diagram.png\n.. include:: missing.rst\n",
    )?;
    dir.write(
        "repo/site/index.html",
        "<html><body><h1>Welcome</h1><p>Quarterly rollout checklist.</p><img src=\"/img/hero.svg\"></body></html>\n",
    )?;
    dir.write(
        "repo/public/img/hero.svg",
        "<svg viewBox=\"0 0 48 24\"><title>Hero</title></svg>\n",
    )?;
    dir.write("repo/assets/diagram.png", "\u{89}PNG")?;
    dir.write(
        "repo/assets/logo.svg",
        "<svg viewBox=\"0 0 16 16\"></svg>\n",
    )?;
    dir.write(
        "repo/assets/unused.svg",
        "<svg viewBox=\"0 0 8 8\"></svg>\n",
    )?;
    dir.write(
        "repo/src/app.ts",
        "export function header(): string {\n  return `<img src=\"assets/logo.svg\">`;\n}\n",
    )?;
    dir.write_bytes(
        "repo/docs/plan.docx",
        &docx("Rollout", "Quarterly rollout checklist."),
    )?;
    let index = dir.0.join("index");
    let find =
        String::from_utf8(run_in(&root, &index, &["find", "quarterly rollout checklist"])?.stdout)?;
    assert!(find.contains("docs/plan.docx"), "{find}");
    let heading = String::from_utf8(run_in(&root, &index, &["find", "Welcome"])?.stdout)?;
    assert!(heading.contains("site/index.html"), "{heading}");
    let audit = String::from_utf8(run_in(&root, &index, &["audit"])?.stdout)?;
    assert!(audit.contains("assets\nassets/unused.svg\n"), "{audit}");
    assert!(!audit.contains("assets/logo.svg\n"), "{audit}");
    assert!(!audit.contains("hero.svg"), "{audit}");
    assert!(!audit.contains("assets/diagram.png\n"), "{audit}");
    assert!(audit.contains("missing.rst docs/guide.rst:7"), "{audit}");
    let show = String::from_utf8(run_in(&root, &index, &["show", "public/img/hero.svg"])?.stdout)?;
    assert!(show.contains("svg 48x24 Hero"), "{show}");
    Ok(())
}

#[test]
fn env_definitions_feed_map_env_and_audit_drift_without_values() -> TestResult {
    let dir = TempDir::new("env-surface")?;
    let root = dir.0.join("repo");
    dir.write(
        "repo/.env.example",
        "DATABASE_URL=postgres://secret-value\nUNUSED_FLAG=1\n",
    )?;
    dir.write(
        "repo/docker-compose.yml",
        "services:\n  api:\n    environment:\n      - DATABASE_URL=postgres://db\n    healthcheck:\n      retries: 3\n",
    )?;
    dir.write(
        "repo/main.go",
        "package main\n\nimport \"os\"\n\nfunc main() {\n\t_ = os.Getenv(\"DATABASE_URL\")\n\t_ = os.Getenv(\"MISSING_TOKEN\")\n}\n",
    )?;
    let index = dir.0.join("index");
    let map = String::from_utf8(run_in(&root, &index, &["map"])?.stdout)?;
    assert!(
        map.contains("DATABASE_URL 1r .env.example,compose main.go:6"),
        "{map}"
    );
    let audit = String::from_utf8(run_in(&root, &index, &["audit"])?.stdout)?;
    assert!(
        audit.contains("read MISSING_TOKEN main.go:7 undefined"),
        "{audit}"
    );
    assert!(
        audit.contains("defined UNUSED_FLAG .env.example:2 unread"),
        "{audit}"
    );
    let found = String::from_utf8(run_in(&root, &index, &["find", "healthcheck retries"])?.stdout)?;
    assert!(found.starts_with("docker-compose.yml"), "{found}");
    for output in [&map, &audit, &found] {
        assert!(!output.contains("secret-value"), "{output}");
    }
    Ok(())
}
