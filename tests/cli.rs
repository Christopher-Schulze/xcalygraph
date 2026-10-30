//! End-to-end checks of the binary: stdout, stderr and exit codes.

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

type TestResult = Result<(), Box<dyn Error>>;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

fn run(args: &[&str]) -> io::Result<Output> {
    Command::new(BINARY).args(args).output()
}

#[test]
fn version_prints_binary_version() -> TestResult {
    for args in [&["version"][..], &["--version"], &["-V"]] {
        let output = run(args)?;
        assert_eq!(output.status.code(), Some(0), "args {args:?}");
        assert_eq!(
            String::from_utf8(output.stdout)?,
            format!("xcalygraph {}\n", env!("CARGO_PKG_VERSION"))
        );
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
        "xcalygraph version\n  binary version; also -V, --version\n"
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
