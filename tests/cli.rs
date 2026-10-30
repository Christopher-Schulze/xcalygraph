//! End-to-end checks of the binary: stdout, stderr and exit codes.

use std::error::Error;
use std::io;
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
        String::from_utf8(overview.stdout)?.starts_with("xcalygraph <command> [args]\ncommands:\n")
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
    let cases: [(&[&str], &str); 3] = [
        (&[], "xcalygraph: missing command; run 'xcalygraph help'\n"),
        (
            &["find"],
            "xcalygraph: unknown command 'find'; run 'xcalygraph help'\n",
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
