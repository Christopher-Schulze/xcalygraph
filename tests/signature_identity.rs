//! Diff identity must use complete, source-backed declarations rather than display signatures.

mod support;

use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Output;
use std::sync::atomic::{AtomicU64, Ordering};

type TestResult = Result<(), Box<dyn Error>>;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    root: PathBuf,
    home: PathBuf,
    temporary: PathBuf,
    index: PathBuf,
}

impl Fixture {
    fn new() -> io::Result<Self> {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".cache");
        fs::create_dir_all(&cache)?;
        if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
            return Err(io::Error::other("test cache must not be a symlink"));
        }
        let directory = loop {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = cache.join(format!(
                "signature-identity-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };
        let fixture = Self {
            root: directory.join("root"),
            home: directory.join("home"),
            temporary: directory.join("temporary"),
            index: directory.join("index"),
            directory,
        };
        fs::create_dir(&fixture.root)?;
        fs::create_dir(&fixture.home)?;
        fs::create_dir(&fixture.temporary)?;
        fixture.git(&["init", "-q"])?;
        Ok(fixture)
    }

    fn write(&self, path: &str, content: &str) -> io::Result<()> {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)
    }

    fn git(&self, args: &[&str]) -> io::Result<()> {
        let output = support::command("git", &self.directory)
            .arg("-C")
            .arg(&self.root)
            .args([
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "user.name=Fixture",
            ])
            .args(args)
            .env("HOME", &self.home)
            .env("TMPDIR", &self.temporary)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
    }

    fn commit(&self) -> io::Result<()> {
        self.git(&["add", "."])?;
        self.git(&["commit", "-qm", "fixture"])
    }

    fn impact(&self, extra: &[&str]) -> io::Result<Output> {
        self.impact_format(extra, true)
    }

    fn impact_format(&self, extra: &[&str], json: bool) -> io::Result<Output> {
        let mut command = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &self.directory);
        command.args(["impact", "--diff"]);
        if json {
            command.arg("--json");
        }
        command
            .args(extra)
            .arg("-C")
            .arg(&self.root)
            .arg("--index-dir")
            .arg(&self.index)
            .env("HOME", &self.home)
            .env("TMPDIR", &self.temporary)
            .output()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn answer(output: Output) -> Result<String, Box<dyn Error>> {
    if !output.status.success() {
        return Err(io::Error::other(String::from_utf8_lossy(&output.stderr).into_owned()).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn line_for<'a>(output: &'a str, path: &str, name: &str) -> Option<&'a str> {
    output.lines().find(|line| {
        line.contains(&format!("\"path\":\"{path}\"")) && line.contains(&format!("{name}\""))
    })
}

#[test]
fn git_fixture_does_not_inherit_an_external_repository_or_index() -> TestResult {
    let fixture = Fixture::new()?;
    let foreign = fixture.directory.join("foreign");
    fs::create_dir(&foreign)?;
    let status = support::command("git", &fixture.directory)
        .args(["init", "-q"])
        .arg(&foreign)
        .env("HOME", &fixture.home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()?;
    assert!(status.success());
    let configuration = foreign.join(".git/config");
    let before = fs::read(&configuration)?;
    let redirected_index = foreign.join("foreign-index");
    let output = support::command(std::env::current_exe()?, &fixture.directory)
        .args([
            "--exact",
            "formatting_changes_do_not_become_signature_changes",
        ])
        .env("HOME", &fixture.home)
        .env("TMPDIR", &fixture.temporary)
        .env("GIT_DIR", foreign.join(".git"))
        .env("GIT_WORK_TREE", &foreign)
        .env("GIT_INDEX_FILE", &redirected_index)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.bare")
        .env("GIT_CONFIG_VALUE_0", "false")
        .output()?;
    assert!(!redirected_index.exists(), "foreign Git index was created");
    assert_eq!(fs::read(configuration)?, before);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn long_type_changes_survive_display_compaction_in_code_and_components() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("go.mod", "module example.com/probe\n\ngo 1.23\n")?;
    let fields = (0..40)
        .map(|i| format!("F{i:02} int"))
        .collect::<Vec<_>>()
        .join("; ");
    let go = format!(
        "package probe\nfunc GoLong(\nvalue struct {{ {fields} }},\n) {{}}\nfunc GoShort(value int) {{}}\n"
    );
    fixture.write("probe.go", &go)?;
    let types = vec!["u32"; 45].join(", ");
    let rust =
        format!("pub fn rust_long(value: ({types})) {{}}\npub fn rust_short(value: u32) {{}}\n");
    fixture.write("probe.rs", &rust)?;
    let fields = (0..28)
        .map(|i| format!("f{i:02}: number"))
        .collect::<Vec<_>>()
        .join("; ");
    let cases = [
        ("ts", "ts"),
        ("vue", "vue"),
        ("svelte", "svelte"),
        ("astro", "astro"),
    ];
    for (label, extension) in cases {
        let declarations = format!(
            "export function {label}Long(\nvalue: {{{fields}}}\n) {{ return 1 }}\nexport function {label}Short(value: number) {{ return 1 }}\n"
        );
        let content = match extension {
            "vue" | "svelte" => format!("<script lang=\"ts\">\n{declarations}</script>\n"),
            "astro" => format!("---\n{declarations}---\n<p>ok</p>\n"),
            _ => declarations,
        };
        fixture.write(&format!("probe.{extension}"), &content)?;
    }
    fixture.commit()?;
    fixture.write(
        "probe.go",
        &go.replace("F00 int", "F00 string")
            .replace("GoShort(value int)", "GoShort(value string)"),
    )?;
    fixture.write(
        "probe.rs",
        &rust
            .replace("value: (u32,", "value: (u64,")
            .replace("rust_short(value: u32)", "rust_short(value: u64)"),
    )?;
    for (label, extension) in cases {
        let path = format!("probe.{extension}");
        let content = fs::read_to_string(fixture.root.join(&path))?;
        fixture.write(
            &path,
            &content.replace("f00: number", "f00: string").replace(
                &format!("{label}Short(value: number)"),
                &format!("{label}Short(value: string)"),
            ),
        )?;
    }
    let output = answer(fixture.impact(&[])?)?;
    for (path, name) in [
        ("probe.go", "GoLong"),
        ("probe.rs", "rust_long"),
        ("probe.ts", "tsLong"),
        ("probe.vue", "vueLong"),
        ("probe.svelte", "svelteLong"),
        ("probe.astro", "astroLong"),
    ] {
        let line = line_for(&output, path, name)
            .ok_or_else(|| io::Error::other(format!("missing {path} {name}: {output}")))?;
        assert!(line.contains("\"kind\":\"sig\""), "{path} {name}: {output}");
    }
    assert_eq!(output, answer(fixture.impact(&["--read-only"])?)?);
    Ok(())
}

#[test]
fn long_overloads_report_the_removed_revision_line() -> TestResult {
    let fixture = Fixture::new()?;
    let rest = (0..25)
        .map(|i| format!("int p{i:02}"))
        .collect::<Vec<_>>()
        .join(", ");
    let first = format!("  public void call(int value, {rest}) {{}}\n");
    let second = format!("  public void call(String value, {rest}) {{}}\n");
    fixture.write(
        "Example.java",
        &format!("class Example {{\n{first}{second}}}\n"),
    )?;
    fixture.commit()?;
    fixture.write("Example.java", &format!("class Example {{\n{second}}}\n"))?;
    for extra in [&[][..], &["--exact"][..]] {
        let output = answer(fixture.impact(extra)?)?;
        assert!(output.contains("\"name\":\"Example.call\",\"path\":\"Example.java\",\"line\":2,\"kind\":\"removed\""), "{output}");
        assert!(
            !output.contains("\"line\":3,\"kind\":\"removed\""),
            "{output}"
        );
    }
    Ok(())
}

#[test]
fn moved_definition_reports_its_signature_change_as_inferred() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("go.mod", "module example.com/probe\n\ngo 1.23\n")?;
    fixture.write(
        "old.go",
        "package probe\nfunc Moved(value int) int { return 1 }\nfunc hidden(value int) int { return 1 }\n",
    )?;
    fixture.commit()?;
    fixture.git(&["mv", "old.go", "new.go"])?;
    fixture.write(
        "new.go",
        "package probe\nfunc Moved(value string) int { return 1 }\nfunc hidden(value string) int { return 1 }\n",
    )?;
    let output = answer(fixture.impact(&[])?)?;
    let line =
        line_for(&output, "old.go", "Moved").ok_or_else(|| io::Error::other(output.clone()))?;
    assert!(
        line.contains("\"kind\":\"moved\"")
            && line.contains("\"sig_change\":\"changed\"")
            && line.contains("\"conf\":\"inferred\""),
        "{output}"
    );
    let hidden = output
        .lines()
        .find(|line| line.contains("\"name\":\"hidden\"") && line.contains("\"kind\":\"moved\""))
        .ok_or_else(|| io::Error::other(output.clone()))?;
    assert!(!hidden.contains("\"sig_change\""), "{output}");
    Ok(())
}

#[test]
fn formatting_changes_do_not_become_signature_changes() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write(
        "probe.ts",
        "export function render(value: string = \"two  spaces\"): string { return value }\n",
    )?;
    fixture.commit()?;
    fixture.write(
        "probe.ts",
        "export function render( value : string = \"two  spaces\" ) : string { return value }\n",
    )?;
    let output = answer(fixture.impact(&[])?)?;
    let line =
        line_for(&output, "probe.ts", "render").ok_or_else(|| io::Error::other(output.clone()))?;
    assert!(!line.contains("\"kind\":\"sig\""), "{output}");
    Ok(())
}

#[test]
fn literal_whitespace_changes_remain_distinct() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write(
        "probe.ts",
        "export function render(value: string = \"two  spaces\"): string { return value }\n",
    )?;
    fixture.commit()?;
    fixture.write(
        "probe.ts",
        "export function render(value: string = \"two spaces\"): string { return value }\n",
    )?;
    let output = answer(fixture.impact(&[])?)?;
    let line =
        line_for(&output, "probe.ts", "render").ok_or_else(|| io::Error::other(output.clone()))?;
    assert!(line.contains("\"kind\":\"sig\""), "{output}");
    Ok(())
}

#[test]
fn body_only_changes_do_not_claim_a_signature_change() -> TestResult {
    let fixture = Fixture::new()?;
    let cases = [
        (
            "probe.go",
            "package probe\nfunc Greet(x int) int { return x + 1 }\n",
            "Greet",
        ),
        (
            "probe.rs",
            "pub fn greet(x: i32) -> i32 { x + 1 }\n",
            "greet",
        ),
        (
            "probe.ts",
            "export function greet(x: number): number { return x + 1 }\n",
            "greet",
        ),
        (
            "probe.py",
            "def greet(x: int) -> int:\n    return x + 1\n",
            "greet",
        ),
        (
            "probe.ex",
            "defmodule Probe do\n  def greet(x), do: x + 1\nend\n",
            "greet",
        ),
        (
            "probe.erl",
            "-module(probe).\n-export([greet/1]).\ngreet(X) -> X + 1.\n",
            "greet",
        ),
        ("probe.ml", "let greet x = x + 1\n", "greet"),
    ];
    for (path, content, _) in cases {
        fixture.write(path, content)?;
    }
    fixture.commit()?;
    for (path, content, _) in cases {
        fixture.write(path, &content.replace(" + 1", " + 2"))?;
    }
    let output = answer(fixture.impact(&[])?)?;
    for (path, _, name) in cases {
        let line = line_for(&output, path, name)
            .ok_or_else(|| io::Error::other(format!("missing {path} {name}: {output}")))?;
        assert!(!line.contains("\"kind\":\"sig\""), "{path}: {output}");
        if matches!(path, "probe.ex" | "probe.erl" | "probe.ml") {
            assert!(line.contains("\"kind\":\"uncertain\""), "{path}: {output}");
        }
    }
    Ok(())
}

#[test]
fn identical_duplicate_signatures_do_not_invent_a_removed_line() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write(
        "probe.js",
        "function greet(x) { return 1 }\nfunction greet(x) { return 2 }\n",
    )?;
    fixture.commit()?;
    fixture.write("probe.js", "function greet(x) { return 2 }\n")?;
    let output = answer(fixture.impact(&[])?)?;
    assert!(!output.contains("\"kind\":\"removed\""), "{output}");
    assert!(output.contains("\"kind\":\"uncertain\""), "{output}");
    let exact = answer(fixture.impact(&["--exact"])?)?;
    assert!(!exact.contains("\"kind\":\"removed\""), "{exact}");

    let fixture = Fixture::new()?;
    fixture.write(
        "guide.md",
        "# Introduction\nBefore.\n# Repeated\nFirst.\n# Repeated\nSecond.\n",
    )?;
    fixture.commit()?;
    fixture.write("guide.md", "# Introduction\nAfter.\n# Repeated\nSecond.\n")?;
    let output = answer(fixture.impact(&[])?)?;
    let introduction = line_for(&output, "guide.md", "Introduction")
        .ok_or_else(|| io::Error::other(output.clone()))?;
    assert!(!introduction.contains("\"kind\":\"uncertain\""), "{output}");
    assert!(!output.contains("\"kind\":\"removed\""), "{output}");
    assert!(output.contains("\"kind\":\"uncertain\""), "{output}");
    Ok(())
}

#[test]
fn formatting_inside_template_substitutions_does_not_change_a_signature() -> TestResult {
    let fixture = Fixture::new()?;
    let cases = [
        (
            "probe.ts",
            "export function render(value = `a  b ${ 1 + 2 }`) { return value }\n",
            " 1 + 2 ",
            "1+2",
        ),
        (
            "probe.py",
            "def render(value = f\"a  b { 1 + 2 }\"):\n    return value\n",
            " 1 + 2 ",
            "1+2",
        ),
        (
            "probe.rb",
            "def render(value = \"a  b #{ 1 + 2 }\")\n  value\nend\n",
            " 1 + 2 ",
            "1+2",
        ),
    ];
    for (path, before, _, _) in cases {
        fixture.write(path, before)?;
    }
    fixture.commit()?;
    for (path, before, from, to) in cases {
        fixture.write(path, &before.replace(from, to))?;
    }
    let output = answer(fixture.impact(&[])?)?;
    for (path, _, _, _) in cases {
        let line =
            line_for(&output, path, "render").ok_or_else(|| io::Error::other(output.clone()))?;
        assert!(!line.contains("\"kind\":\"sig\""), "{path}: {output}");
    }
    Ok(())
}

#[test]
fn incomplete_current_or_revision_source_cannot_produce_exact_removals() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("probe.py", "def greet(x):\n    return x\n")?;
    fixture.commit()?;
    fixture.write("probe.py", "def greet(\n")?;
    let output = answer(fixture.impact(&[])?)?;
    assert!(
        output.contains("incomplete current") && !output.contains("\"kind\":\"removed\""),
        "{output}"
    );

    let fixture = Fixture::new()?;
    fixture.write("probe.py", "def greet(\n")?;
    fixture.commit()?;
    fixture.write("probe.py", "def greet(x):\n    return x\n")?;
    let output = answer(fixture.impact(&[])?)?;
    assert!(
        output.contains("incomplete revision") && !output.contains("\"kind\":\"removed\""),
        "{output}"
    );
    Ok(())
}

#[test]
fn filtered_and_complete_changes_survive_an_unrelated_parse_error() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("good.py", "def visible(value: int):\n    return value\n")?;
    fixture.write(
        "bad.py",
        "from good import visible\ndef other(value):\n    return visible(value)\n",
    )?;
    fixture.commit()?;
    fixture.write("good.py", "def visible(value: str):\n    return value\n")?;
    fixture.write(
        "bad.py",
        "from good import visible\ndef other(value):\n    return visible(value)\ndef broken(\n",
    )?;
    for extra in [
        &[][..],
        &["-p", "good.py"][..],
        &["-p", "good.py", "--exact"][..],
    ] {
        let output = answer(fixture.impact(extra)?)?;
        let line = line_for(&output, "good.py", "visible")
            .ok_or_else(|| io::Error::other(output.clone()))?;
        assert!(line.contains("\"kind\":\"sig\""), "{output}");
        assert!(
            output.contains("\"t\":\"note\"")
                && output.contains("\"path\":\"bad.py\"")
                && output.contains("incomplete current"),
            "{output}"
        );
        assert!(!output.contains("bad.other"), "{output}");
    }
    let text = answer(fixture.impact_format(&["-p", "good.py"], false)?)?;
    assert!(
        text.contains("! incomplete current bad.py")
            && text.contains("visible")
            && text.contains(" sig"),
        "{text}"
    );
    let current = answer(fixture.impact(&["-p", "good.py"])?)?;
    assert_eq!(
        current,
        answer(fixture.impact(&["-p", "good.py", "--read-only"])?)?
    );
    for budget in ["10", "20", "30", "50"] {
        let output = fixture.impact(&["-p", "good.py", "-b", budget])?;
        assert!(output.status.success());
        assert!(
            output.stdout.len() <= budget.parse::<usize>()? * 3,
            "{budget}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    Ok(())
}

#[test]
fn an_incomplete_current_file_cannot_fabricate_same_language_removals() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("old.py", "def moved(value):\n    return value\n")?;
    fixture.write("bad.py", "def other(value):\n    return value\n")?;
    fixture.write("old.go", "package fixture\nfunc Gone() {}\n")?;
    fixture.commit()?;
    fs::remove_file(fixture.root.join("old.py"))?;
    fs::remove_file(fixture.root.join("old.go"))?;
    fixture.write("bad.py", "def moved(\n")?;
    fixture.write("fresh.py", "def fresh(\n")?;
    let output = answer(fixture.impact(&[])?)?;
    let python =
        line_for(&output, "old.py", "moved").ok_or_else(|| io::Error::other(output.clone()))?;
    assert!(
        python.contains("\"kind\":\"uncertain\"") && !python.contains("\"kind\":\"removed\""),
        "{output}"
    );
    let go = line_for(&output, "old.go", "Gone").ok_or_else(|| io::Error::other(output.clone()))?;
    assert!(go.contains("\"kind\":\"removed\""), "{output}");
    assert!(
        output.contains("\"path\":\"fresh.py\"")
            && !output.contains(
                "\"name\":\"fresh.py\",\"path\":\"fresh.py\",\"line\":1,\"kind\":\"new 0\""
            ),
        "{output}"
    );
    let exact = answer(fixture.impact(&["--exact"])?)?;
    assert!(
        !exact.contains("\"path\":\"old.py\",\"line\":1,\"kind\":\"removed\""),
        "{exact}"
    );
    assert!(
        exact.contains("\"path\":\"old.go\"") && exact.contains("\"kind\":\"removed\""),
        "{exact}"
    );
    Ok(())
}

#[test]
fn an_incomplete_revision_does_not_fabricate_new_definitions() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("bad.py", "def broken(\n")?;
    fixture.write(
        "good.go",
        "package fixture\nfunc Good(value int) int { return value }\n",
    )?;
    fixture.commit()?;
    fixture.write("bad.py", "def broken(value):\n    return value\n")?;
    fixture.write(
        "good.go",
        "package fixture\nfunc Good(value int64) int64 { return value }\n",
    )?;
    let output = answer(fixture.impact(&[])?)?;
    assert!(
        output.contains("\"t\":\"note\"")
            && output.contains("\"path\":\"bad.py\"")
            && output.contains("incomplete revision"),
        "{output}"
    );
    let python =
        line_for(&output, "bad.py", "broken").ok_or_else(|| io::Error::other(output.clone()))?;
    assert!(
        python.contains("\"kind\":\"uncertain\"") && !python.contains("\"kind\":\"new\""),
        "{output}"
    );
    let go =
        line_for(&output, "good.go", "Good").ok_or_else(|| io::Error::other(output.clone()))?;
    assert!(go.contains("\"kind\":\"sig\""), "{output}");
    Ok(())
}

#[test]
fn tiny_json_distinguishes_truncated_coverage_from_an_empty_diff() -> TestResult {
    let fixture = Fixture::new()?;
    let complete = "def greet(value):\n    return value\n";
    fixture.write("probe.py", complete)?;
    fixture.commit()?;
    fixture.write("probe.py", "def greet(\n")?;
    assert_eq!(answer(fixture.impact(&["-b", "1"])?)?, "{}\n");
    fixture.write("probe.py", complete)?;
    assert_eq!(answer(fixture.impact(&["-b", "1"])?)?, "");
    Ok(())
}
