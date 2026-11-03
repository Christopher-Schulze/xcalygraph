//! Rust test discovery must distinguish runnable tests from their helpers.

mod support;

use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> io::Result<Self> {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".cache");
        fs::create_dir_all(&cache)?;
        if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
            return Err(io::Error::other("test cache must not be a symlink"));
        }
        loop {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = cache.join(format!(
                "rust-test-entrypoints-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn rust_helpers_are_not_reported_as_runnable_tests() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("root");
    let home = fixture.0.join("home");
    let temporary = fixture.0.join("temporary");
    fs::create_dir(&root)?;
    fs::create_dir(&home)?;
    fs::create_dir(&temporary)?;
    fs::create_dir(root.join("tests"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )?;
    fs::write(
        root.join("tests/suite.rs"),
        "fn target() {}\nfn helper() { target(); }\n#[test]\nfn actual_test() { helper(); }\n",
    )?;

    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["impact", "target", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .env("HOME", &home)
        .env("TMPDIR", &temporary)
        .env("XDG_CONFIG_HOME", &home)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.lines()
            .any(|line| line.starts_with("t ") && line.contains("actual_test")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("r ") && line.contains("actual_test")),
        "{text}"
    );
    assert!(
        text.lines()
            .filter(|line| line.starts_with("t ") || line.starts_with("r "))
            .all(|line| !line.contains("helper")),
        "helper is not a runnable test: {text}"
    );
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "target", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .env("HOME", &home)
        .env("TMPDIR", &temporary)
        .env("XDG_CONFIG_HOME", &home)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let show = String::from_utf8(output.stdout)?;
    assert!(
        show.lines()
            .any(|line| line.contains("t2 ") && line.contains("actual_test")),
        "{show}"
    );
    assert!(
        !show
            .lines()
            .any(|line| line.contains(" test ") && line.contains("helper")),
        "{show}"
    );
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["map", "--json", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .env("HOME", &home)
        .env("TMPDIR", &temporary)
        .env("XDG_CONFIG_HOME", &home)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map = String::from_utf8(output.stdout)?;
    assert!(
        !map.lines()
            .any(|line| line.contains("\"rel\":\"entry\"") && line.contains("actual_test")),
        "{map}"
    );
    Ok(())
}
