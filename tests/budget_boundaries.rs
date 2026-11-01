//! Real CLI output must obey byte budgets and retain actionable test commands.

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
                "budget-boundaries-{}-{sequence}",
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
        for path in [&fixture.root, &fixture.home, &fixture.temporary] {
            fs::create_dir(path)?;
        }
        fs::create_dir(fixture.root.join("pkg"))?;
        fs::write(
            fixture.root.join("go.mod"),
            "module example.com/fixture\n\ngo 1.23\n",
        )?;
        let mut source = String::from("package pkg\nfunc F0() int { return F1() }\n");
        for number in 1..20 {
            source.push_str(&format!("func F{number}() int {{ return {number} }}\n"));
        }
        fs::write(fixture.root.join("pkg/code.go"), source)?;
        let calls = (0..20)
            .map(|number| format!("F{number}()"))
            .collect::<Vec<_>>()
            .join("; ");
        fs::write(
            fixture.root.join("pkg/code_test.go"),
            format!("package pkg\nimport \"testing\"\nfunc TestAll(t *testing.T) {{ {calls} }}\n"),
        )?;
        fs::write(
            fixture.root.join("bad.py"),
            "def bad(value):\n    return value\n",
        )?;
        fixture.git(&["init", "-q"])?;
        fixture.git(&["add", "."])?;
        fixture.git(&["commit", "-qm", "fixture"])?;
        fs::write(fixture.root.join("bad.py"), "def bad(\n")?;
        let indexed = fixture.query(&["index"], false)?;
        if !indexed.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&indexed.stderr).into_owned(),
            ));
        }
        Ok(fixture)
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
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
    }

    fn query(&self, args: &[&str], json: bool) -> io::Result<Output> {
        let mut command = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &self.directory);
        command
            .args(args)
            .arg("-C")
            .arg(&self.root)
            .arg("--index-dir")
            .arg(&self.index);
        if json {
            command.arg("--json");
        }
        command
            .env("HOME", &self.home)
            .env("TMPDIR", &self.temporary)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .output()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn tiny_budgets_are_hard_caps_for_every_query_and_coverage_notices() -> TestResult {
    let fixture = Fixture::new()?;
    let queries: &[&[&str]] = &[
        &["map"],
        &["find", "F0"],
        &["ctx", "F0"],
        &["show", "F0"],
        &["impact", "F0"],
        &["impact", "--diff"],
        &["path", "F0", "F1"],
        &["audit"],
    ];
    for query in queries {
        for budget in 1..=10 {
            for json in [false, true] {
                let value = budget.to_string();
                let mut args = query.to_vec();
                args.extend(["-b", value.as_str(), "--read-only"]);
                let output = fixture.query(&args, json)?;
                assert!(
                    output.status.success(),
                    "{query:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    output.stdout.len() <= budget * 3,
                    "{query:?}, budget {budget}, json {json}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
                let text = String::from_utf8(output.stdout)?;
                if json {
                    assert!(
                        text.lines()
                            .all(|line| line.starts_with('{') && line.ends_with('}')),
                        "{text}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn maximum_budget_never_overflows_query_arithmetic() -> TestResult {
    let fixture = Fixture::new()?;
    let maximum = usize::MAX.to_string();
    for query in [&["map"][..], &["impact", "F0"][..]] {
        for json in [false, true] {
            let mut args = query.to_vec();
            args.extend(["-b", &maximum, "--read-only"]);
            let output = fixture.query(&args, json)?;
            assert!(
                output.status.success(),
                "{query:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.stdout.is_empty());
        }
    }
    Ok(())
}

#[test]
fn compact_run_command_survives_many_targets_with_an_accurate_cut_count() -> TestResult {
    let fixture = Fixture::new()?;
    let names = (0..20)
        .map(|number| format!("F{number}"))
        .collect::<Vec<_>>();
    for (budget, json) in [("40", false), ("80", true)] {
        let mut args = vec!["impact"];
        args.extend(names.iter().map(String::as_str));
        args.extend(["-b", budget, "--read-only"]);
        let output = fixture.query(&args, json)?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.len() <= budget.parse::<usize>()? * 3);
        let text = String::from_utf8(output.stdout)?;
        assert!(text.contains("go test ./pkg"), "{text}");
        let shown = if json {
            text.lines()
                .filter(|line| {
                    line.contains("\"rel\":\"chg\"")
                        || line.contains("\"rel\":\"test\"")
                        || line.contains("\"rel\":\"run\"")
                })
                .count()
        } else {
            text.lines()
                .filter(|line| {
                    line.starts_with("~ ") || line.starts_with("t ") || line.starts_with("r ")
                })
                .count()
        };
        let last = text.lines().last().ok_or("missing truncation notice")?;
        let cut: usize = if json {
            last.split("\"n\":")
                .nth(1)
                .and_then(|s| s.trim_end_matches('}').parse().ok())
        } else {
            last.strip_prefix('+')
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse().ok())
        }
        .ok_or("truncation notice lacks its count")?;
        assert_eq!(shown + cut, 22, "{text}");
    }
    Ok(())
}
