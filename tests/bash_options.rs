//! End-to-end Bash CLI option facts and trap handler references.

mod support;

use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Output;

type TestResult = Result<(), Box<dyn Error>>;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

struct Fixture {
    directory: PathBuf,
    root: PathBuf,
    index: PathBuf,
}

impl Fixture {
    fn new() -> io::Result<Self> {
        let directory = support::exclusive_dir("bash-options")?;
        let root = directory.join("root");
        fs::create_dir(&root)?;
        Ok(Self {
            index: directory.join("index"),
            directory,
            root,
        })
    }

    fn write(&self, relative: &str, content: &str) -> io::Result<()> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)
    }

    fn run(&self, arguments: &[&str]) -> io::Result<Output> {
        support::command(BINARY, &self.directory)
            .args(arguments)
            .arg("-C")
            .arg(&self.root)
            .arg("--index-dir")
            .arg(&self.index)
            .output()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn show_lists_static_getopts_options_and_trap_handlers_resolve() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write(
        "src/options.sh",
        include_str!("fixtures/languages/bash/scripts/options.sh"),
    )?;

    let output = fixture.run(&["show", "src/options.sh"])?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let shown = String::from_utf8(output.stdout)?;
    assert!(
        shown.starts_with("src/options.sh bash 46 lines 15 symbols\n"),
        "{shown}"
    );
    assert_eq!(shown.matches("arg=").count(), 9, "{shown}");
    for (name, signature) in [
        ("-a", "arg=none;case=16"),
        ("-b", "arg=required;case=17"),
        ("-c", "arg=none;case=18"),
        ("-d", "arg=none;case=*:21"),
        ("-z", "arg=none;case=none"),
        ("-x", "arg=none;case=*:36"),
        ("-y", "arg=none;case=*:36"),
        ("-m", "arg=none;case=ambiguous"),
        ("-n", "arg=none;case=ambiguous"),
    ] {
        let locator = format!("src/options.sh::{name}");
        let option = fixture.run(&["show", &locator])?;
        assert_eq!(option.status.code(), Some(0), "{locator}");
        let option = String::from_utf8(option.stdout)?;
        assert!(option.contains(" option "), "{locator}: {option}");
        assert!(option.contains(name), "{locator}: {option}");
        assert!(option.contains(signature), "{locator}: {option}");
    }
    assert!(
        !shown.contains("-q"),
        "dynamic optstring was promoted: {shown}"
    );

    let output = fixture.run(&["show", "cleanup"])?;
    assert_eq!(output.status.code(), Some(0));
    let shown = String::from_utf8(output.stdout)?;
    assert!(
        shown.lines().any(|item| item.contains("src/options.sh:8")),
        "trap caller at line 8 is missing: {shown}"
    );
    assert!(
        !shown.lines().any(|item| item.contains("src/options.sh:11")),
        "compound trap command text became a handler reference: {shown}"
    );

    let output = fixture.run(&["impact", "cleanup"])?;
    assert_eq!(output.status.code(), Some(0));
    let impact = String::from_utf8(output.stdout)?;
    assert!(
        impact.contains("src/options.sh"),
        "trap handler impact is missing its source file: {impact}"
    );

    let output = fixture.run(&["audit"])?;
    assert_eq!(output.status.code(), Some(0));
    let audited = String::from_utf8(output.stdout)?;
    assert!(
        !audited.contains("-a"),
        "options were treated as dead code: {audited}"
    );
    Ok(())
}
