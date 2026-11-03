//! Incomplete analysis stays visible: answers that rest on absence name the files not analyzed,
//! healthy repositories stay silent and one bad file never costs the other files' answers.

mod support;

use std::error::Error;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

type TestResult = Result<(), Box<dyn Error>>;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

struct Fixture {
    directory: PathBuf,
    root: PathBuf,
    index: PathBuf,
}

impl Fixture {
    fn new() -> io::Result<Self> {
        let directory = support::exclusive_dir("analysis-coverage")?;
        let root = directory.join("root");
        fs::create_dir(&root)?;
        Ok(Self {
            index: directory.join("index"),
            directory,
            root,
        })
    }

    fn write(&self, relative: &str, content: &str) -> io::Result<()> {
        fs::write(self.root.join(relative), content)
    }

    fn query(&self, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
        let output = support::command(BINARY, &self.directory)
            .args(arguments)
            .arg("-C")
            .arg(&self.root)
            .arg("--index-dir")
            .arg(&self.index)
            .output()?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

const GOOD: &str = "package main\n\nfunc Lonely() {}\n\nfunc Used() {}\n\nfunc main() { Used() }\n";

#[test]
fn healthy_repository_has_no_coverage_lines() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("go.mod", "module example.com/app\n\ngo 1.23\n")?;
    fixture.write("main.go", GOOD)?;
    for query in [
        vec!["map"],
        vec!["audit"],
        vec!["show", "Lonely"],
        vec!["impact", "Lonely"],
    ] {
        let answer = fixture.query(&query)?;
        assert!(!answer.contains("coverage"), "{query:?}:\n{answer}");
    }
    Ok(())
}

#[test]
fn parse_errors_qualify_absence_but_not_orientation() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("go.mod", "module example.com/app\n\ngo 1.23\n")?;
    fixture.write("main.go", GOOD)?;
    fixture.write("broken.go", "package main\n\nfunc Broken( {\n\tLonely(\n")?;

    let note = "! coverage: 1 of 3 files incomplete (1 parse errors): broken.go; absence unproven";
    let audit = fixture.query(&["audit"])?;
    assert!(audit.starts_with(note), "{audit}");
    // A symbol with a caller claims no absence.
    let show = fixture.query(&["show", "Used"])?;
    assert!(!show.contains("coverage"), "has a caller:\n{show}");
    // No caller found for Lonely: that absence is not proven, the broken file may call it.
    let show = fixture.query(&["show", "Lonely"])?;
    assert!(show.contains("(1 parse errors): broken.go;"), "{show}");
    let map = fixture.query(&["map"])?;
    assert!(!map.contains("coverage"), "{map}");

    let json = fixture.query(&["audit", "--json"])?;
    let first = json.lines().next().unwrap_or_default();
    assert!(first.contains("\"t\":\"note\""), "{first}");
    assert!(first.contains("1 parse errors"), "{first}");
    Ok(())
}

#[test]
fn absence_notes_stay_within_the_language_family_and_filter() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("go.mod", "module example.com/app\n\ngo 1.23\n")?;
    fixture.write("main.go", GOOD)?;
    fixture.write("broken.sh", "#!/bin/bash\nif then fi ((\n")?;
    fixture.write("other.go", "package main\n\nfunc Other( {\n")?;

    // A broken shell script cannot hide a Go caller; the broken Go file can.
    let show = fixture.query(&["show", "Lonely"])?;
    assert!(
        show.contains("! coverage: 1 of 2 files incomplete (1 parse errors): other.go;"),
        "{show}"
    );
    assert!(!show.contains("broken.sh"), "{show}");
    let impact = fixture.query(&["impact", "Lonely"])?;
    assert!(impact.contains("other.go; absence unproven"), "{impact}");
    // A path filter that excludes the broken file also excludes its limitation.
    let filtered = fixture.query(&["audit", "-p", "main.go"])?;
    assert!(!filtered.contains("coverage"), "{filtered}");
    Ok(())
}

#[test]
fn unreadable_source_costs_only_that_file_and_recovers() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("go.mod", "module example.com/app\n\ngo 1.23\n")?;
    fixture.write("main.go", GOOD)?;
    fixture.write("locked.go", "package main\n\nfunc Hidden() { Lonely() }\n")?;
    let locked = fixture.root.join("locked.go");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;

    let found = fixture.query(&["find", "Used"])?;
    assert!(found.contains("main.go"), "{found}");
    // Severe enough for orientation, and the absent caller of Lonely is not proven absent.
    let map = fixture.query(&["map"])?;
    assert!(
        map.contains("! coverage: 1 of 3 files incomplete (1 unreadable): locked.go;"),
        "{map}"
    );
    let show = fixture.query(&["show", "Lonely"])?;
    assert!(show.contains("(1 unreadable): locked.go;"), "{show}");

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644))?;
    let map = fixture.query(&["map"])?;
    assert!(!map.contains("coverage"), "{map}");
    let show = fixture.query(&["show", "Lonely"])?;
    assert!(show.contains("Hidden"), "caller after recovery:\n{show}");
    assert!(!show.contains("coverage"), "{show}");
    Ok(())
}

#[test]
fn coverage_note_lists_at_most_three_paths_most_severe_first_within_budget() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("go.mod", "module example.com/app\n\ngo 1.23\n")?;
    fixture.write("main.go", GOOD)?;
    for name in ["a", "b", "c", "d", "e"] {
        fixture.write(&format!("{name}.go"), "package main\n\nfunc Broken( {\n")?;
    }
    let locked = fixture.root.join("z.go");
    fs::write(&locked, "package main\n")?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;

    let audit = fixture.query(&["audit"])?;
    let expected = "! coverage: 6 of 8 files incomplete (1 unreadable, 5 parse errors): z.go a.go b.go +3; absence unproven\n";
    assert!(audit.starts_with(expected), "{audit}");
    assert_eq!(audit, fixture.query(&["audit"])?, "deterministic");
    let tiny = fixture.query(&["audit", "-b", "10"])?;
    assert!(tiny.len() <= 30, "budget of 10 tokens:\n{tiny}");

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644))?;
    Ok(())
}
