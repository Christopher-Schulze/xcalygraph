//! Recovery must not reuse corrupt derived images or write in read-only mode.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

type TestResult = Result<(), Box<dyn Error>>;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> io::Result<Self> {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".cache");
        fs::create_dir_all(&cache)?;
        if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
            return Err(io::Error::other("test cache must not be a symlink"));
        }
        for sequence in 0..1000 {
            let path = cache.join(format!(
                "index-corruption-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("no unused test directory"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    work: TempDir,
    root: PathBuf,
    home: PathBuf,
    temp: PathBuf,
}

impl Fixture {
    fn new() -> io::Result<Self> {
        let work = TempDir::new()?;
        let root = work.0.join("root");
        let home = work.0.join("home");
        let temp = work.0.join("temp");
        fs::create_dir(&root)?;
        fs::create_dir(&home)?;
        fs::create_dir(&temp)?;
        fs::write(
            root.join("lib.rs"),
            "pub fn recoverable_marker() -> usize { 42 }\n",
        )?;
        Ok(Self {
            work,
            root,
            home,
            temp,
        })
    }

    fn run(&self, args: &[&str], index: Option<&Path>) -> io::Result<Output> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xcalygraph"));
        command
            .args(args)
            .arg("-C")
            .arg(&self.root)
            .env("HOME", &self.home)
            .env("TMPDIR", &self.temp)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("USER", "xcalygraph-recovery-test")
            .env_remove("XCALYGRAPH_INDEX_DIR");
        if let Some(index) = index {
            command.arg("--index-dir").arg(index);
        }
        command.output()
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}, stderr {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn derived(base: &Path) -> io::Result<PathBuf> {
    let directories = fs::read_dir(base)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    if directories.len() != 1 || !directories[0].is_dir() {
        return Err(io::Error::other("expected exactly one index directory"));
    }
    Ok(directories[0].join("derived"))
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    inode: u64,
    modified: SystemTime,
    contents: Option<Vec<u8>>,
}

fn snapshot(root: &Path) -> io::Result<BTreeMap<PathBuf, Entry>> {
    fn visit(root: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, Entry>) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::other("unexpected symlink in test fixture"));
        }
        let relative = path.strip_prefix(root).map_err(io::Error::other)?;
        let contents = if metadata.is_file() {
            Some(fs::read(path)?)
        } else {
            None
        };
        entries.insert(
            relative.to_path_buf(),
            Entry {
                inode: metadata.ino(),
                modified: metadata.modified()?,
                contents,
            },
        );
        if metadata.is_dir() {
            for child in fs::read_dir(path)? {
                visit(root, &child?.path(), entries)?;
            }
        }
        Ok(())
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries)?;
    Ok(entries)
}

fn corrupt(path: &Path, kind: &str) -> io::Result<()> {
    let mut image = fs::read(path)?;
    if image.len() <= 128 {
        return Err(io::Error::other("derived fixture is too short"));
    }
    match kind {
        "header" => image[0] ^= 1,
        "length-overflow" => image[24..32].fill(0xff),
        "checksum" => image[32] ^= 1,
        "payload" => {
            let last = image.len() - 1;
            image[last] ^= 1;
        }
        "truncated" => {
            image.pop();
        }
        _ => return Err(io::Error::other("unknown corruption")),
    }
    fs::write(path, image)
}

fn fact_shard(path: &Path, source: &str) -> io::Result<PathBuf> {
    let shard = xxhash_rust::xxh3::xxh3_64(source.as_bytes()) % 64;
    let store = path
        .parent()
        .ok_or_else(|| io::Error::other("missing index store"))?;
    Ok(store.join(format!("facts/{shard:02}")))
}

#[test]
fn writable_query_and_index_recover_every_corrupt_derived_image() -> TestResult {
    for recovery in ["query", "index"] {
        for kind in [
            "header",
            "length-overflow",
            "checksum",
            "payload",
            "truncated",
        ] {
            let fixture = Fixture::new()?;
            let base = fixture.work.0.join("index");
            assert_success(&fixture.run(&["index"], Some(&base))?);
            let expected = fixture.run(&["find", "recoverable_marker", "--json"], Some(&base))?;
            assert_success(&expected);
            assert!(String::from_utf8_lossy(&expected.stdout).contains("recoverable_marker"));
            let path = derived(&base)?;
            let fresh = fs::read(&path)?;
            corrupt(&path, kind)?;
            let before = snapshot(&fixture.work.0)?;
            let read_only =
                fixture.run(&["find", "recoverable_marker", "--read-only"], Some(&base))?;
            assert!(!read_only.status.success(), "{recovery} {kind}");
            assert!(
                String::from_utf8_lossy(&read_only.stderr).contains("xcalygraph index"),
                "{recovery} {kind}: {}",
                String::from_utf8_lossy(&read_only.stderr)
            );
            assert!(
                snapshot(&fixture.work.0)? == before,
                "read-only query modified the fixture: {recovery} {kind}"
            );
            let repaired = if recovery == "query" {
                fixture.run(&["find", "recoverable_marker", "--json"], Some(&base))?
            } else {
                fixture.run(&["index"], Some(&base))?
            };
            assert_success(&repaired);
            let actual = fixture.run(
                &["find", "recoverable_marker", "--json", "--read-only"],
                Some(&base),
            )?;
            assert_success(&actual);
            assert_eq!(actual.stdout, expected.stdout, "{recovery} {kind}");
            assert!(
                fs::read(&path)? == fresh,
                "recovered image differs from fresh build: {recovery} {kind}"
            );
        }
    }
    Ok(())
}

#[test]
fn unchanged_index_keeps_existing_files_and_metadata() -> TestResult {
    let fixture = Fixture::new()?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let before = snapshot(&base)?;
    let output = fixture.run(&["index"], Some(&base))?;
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("changed 0 deleted 0"));
    assert!(snapshot(&base)? == before, "unchanged index was rewritten");
    Ok(())
}

#[test]
fn corrupt_default_index_read_only_changes_nothing() -> TestResult {
    let fixture = Fixture::new()?;
    assert_success(&fixture.run(&["index"], None)?);
    let base = fixture.temp.join("xcalygraph-xcalygraph-recovery-test");
    let before = snapshot(&fixture.work.0)?;
    let fresh = fixture.run(&["find", "recoverable_marker", "--read-only"], None)?;
    assert_success(&fresh);
    assert!(
        snapshot(&fixture.work.0)? == before,
        "fresh read-only query wrote files"
    );
    corrupt(&derived(&base)?, "payload")?;
    let before = snapshot(&fixture.work.0)?;
    let output = fixture.run(&["find", "recoverable_marker", "--read-only"], None)?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("xcalygraph index"));
    assert!(
        snapshot(&fixture.work.0)? == before,
        "read-only query modified the default-index fixture"
    );
    Ok(())
}

#[test]
fn read_only_discovers_a_local_index_without_probing_or_creating_a_shared_base() -> TestResult {
    let fixture = Fixture::new()?;
    let local = fixture.root.join(".xcalygraph");
    assert_success(&fixture.run(&["index"], Some(&local))?);
    let expected = fixture.run(&["find", "recoverable_marker", "--read-only"], Some(&local))?;
    assert_success(&expected);
    let before = snapshot(&fixture.work.0)?;
    let actual = fixture.run(&["find", "recoverable_marker", "--read-only"], None)?;
    assert_success(&actual);
    assert_eq!(actual.stdout, expected.stdout);
    let empty_override = Command::new(env!("CARGO_BIN_EXE_xcalygraph"))
        .args(["find", "recoverable_marker", "--read-only", "-C"])
        .arg(&fixture.root)
        .env("HOME", &fixture.home)
        .env("TMPDIR", &fixture.temp)
        .env("XDG_CONFIG_HOME", fixture.home.join("config"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("USER", "xcalygraph-recovery-test")
        .env("XCALYGRAPH_INDEX_DIR", "")
        .output()?;
    assert_success(&empty_override);
    assert_eq!(empty_override.stdout, expected.stdout);
    assert!(
        snapshot(&fixture.work.0)? == before,
        "local read-only query wrote files"
    );
    Ok(())
}

#[test]
fn missing_default_read_only_index_does_not_create_directories() -> TestResult {
    let fixture = Fixture::new()?;
    let before = snapshot(&fixture.work.0)?;
    let output = fixture.run(&["find", "recoverable_marker", "--read-only"], None)?;
    assert_eq!(output.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&output.stderr).contains("index missing"));
    assert!(
        snapshot(&fixture.work.0)? == before,
        "missing read-only query wrote files"
    );
    Ok(())
}

#[test]
fn derived_recovery_never_publishes_graph_without_required_shard_facts() -> TestResult {
    for recovery in ["query", "index"] {
        for kind in ["header", "payload"] {
            let fixture = Fixture::new()?;
            let base = fixture.work.0.join("index");
            assert_success(&fixture.run(&["index"], Some(&base))?);
            let path = derived(&base)?;
            fs::write(fact_shard(&path, "lib.rs")?, b"corrupt shard")?;
            corrupt(&path, kind)?;
            let damaged = fs::read(&path)?;
            let output = if recovery == "query" {
                fixture.run(&["find", "recoverable_marker"], Some(&base))?
            } else {
                fixture.run(&["index"], Some(&base))?
            };
            if output.status.success() {
                if recovery == "query" {
                    assert!(String::from_utf8_lossy(&output.stdout).contains("recoverable_marker"));
                }
                let found =
                    fixture.run(&["find", "recoverable_marker", "--read-only"], Some(&base))?;
                assert_success(&found);
                assert!(String::from_utf8_lossy(&found.stdout).contains("recoverable_marker"));
            } else {
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("fact shard"),
                    "{recovery} {kind}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    fs::read(&path)? == damaged,
                    "failed recovery replaced the corrupt image: {recovery} {kind}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn derived_recovery_never_publishes_graph_missing_per_file_facts() -> TestResult {
    let fixture = Fixture::new()?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let path = derived(&base)?;
    let shard = xxhash_rust::xxh3::xxh3_64(b"lib.rs") % 64;
    let facts = path.parent().ok_or("missing index store")?.join("facts");
    let empty = fs::read(facts.join(format!("{:02}", (shard + 1) % 64)))?;
    fs::write(fact_shard(&path, "lib.rs")?, empty)?;
    corrupt(&path, "payload")?;
    let damaged = fs::read(&path)?;
    let output = fixture.run(&["index"], Some(&base))?;
    if output.status.success() {
        let found = fixture.run(&["find", "recoverable_marker", "--read-only"], Some(&base))?;
        assert_success(&found);
        assert!(String::from_utf8_lossy(&found.stdout).contains("recoverable_marker"));
    } else {
        assert!(String::from_utf8_lossy(&output.stderr).contains("fact missing for lib.rs"));
        assert!(
            fs::read(&path)? == damaged,
            "failed recovery replaced the image"
        );
    }
    Ok(())
}

#[test]
fn damaged_shard_never_publishes_a_stale_or_incomplete_graph() -> TestResult {
    let fixture = Fixture::new()?;
    let marker_shard = xxhash_rust::xxh3::xxh3_64(b"lib.rs") % 64;
    let other = (0..100)
        .map(|number| format!("other_{number}.rs"))
        .find(|name| xxhash_rust::xxh3::xxh3_64(name.as_bytes()) % 64 != marker_shard)
        .ok_or("no second shard")?;
    fs::write(fixture.root.join(&other), "pub fn other() -> u8 { 1 }\n")?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let path = derived(&base)?;
    let files = path.parent().ok_or("missing index store")?.join("files");
    let original_derived = fs::read(&path)?;
    let original_files = fs::read(&files)?;
    fs::write(fact_shard(&path, "lib.rs")?, b"corrupt shard")?;
    fs::write(
        fixture.root.join(other),
        "pub fn other_changed() -> u8 { 2 }\n",
    )?;
    let output = fixture.run(&["index"], Some(&base))?;
    if output.status.success() {
        for name in ["recoverable_marker", "other_changed"] {
            let found = fixture.run(&["find", name, "--read-only"], Some(&base))?;
            assert_success(&found);
            assert!(String::from_utf8_lossy(&found.stdout).contains(name));
        }
    } else {
        assert!(String::from_utf8_lossy(&output.stderr).contains("fact shard"));
        assert!(
            fs::read(&path)? == original_derived,
            "failed rebuild replaced the graph"
        );
        assert!(
            fs::read(&files)? == original_files,
            "failed rebuild replaced the file table"
        );
        let read_only = fixture.run(&["find", "recoverable_marker", "--read-only"], Some(&base))?;
        assert_eq!(read_only.status.code(), Some(5));
    }
    Ok(())
}

#[test]
fn damaged_unrelated_shards_recover_to_a_fresh_graph() -> TestResult {
    for damage in ["missing", "compression", "missing-fact", "corrupt"] {
        for trigger in ["changed-source", "derived-recovery"] {
            let fixture = Fixture::new()?;
            fs::write(
                fixture.root.join("lib.rs"),
                "pub fn recoverable_marker() -> usize { 42 }\npub fn calls_marker() -> usize { recoverable_marker() }\n",
            )?;
            let marker_shard = xxhash_rust::xxh3::xxh3_64(b"lib.rs") % 64;
            let mut other = (0..100)
                .map(|number| format!("other_{number}.rs"))
                .filter(|name| xxhash_rust::xxh3::xxh3_64(name.as_bytes()) % 64 != marker_shard);
            let changed = other.next().ok_or("missing changed source")?;
            let changed_shard = xxhash_rust::xxh3::xxh3_64(changed.as_bytes()) % 64;
            let untouched = other
                .find(|name| xxhash_rust::xxh3::xxh3_64(name.as_bytes()) % 64 != changed_shard)
                .ok_or("missing untouched source")?;
            fs::write(fixture.root.join(&changed), "pub fn changed_old() {}\n")?;
            fs::write(fixture.root.join(&untouched), "pub fn stable_marker() {}\n")?;
            let base = fixture.work.0.join("index");
            assert_success(&fixture.run(&["index"], Some(&base))?);
            let path = derived(&base)?;
            let damaged = fact_shard(&path, "lib.rs")?;
            let stable = fact_shard(&path, &untouched)?;
            let stable_metadata = fs::metadata(&stable)?;
            let stable_before = (
                stable_metadata.ino(),
                stable_metadata.modified()?,
                fs::read(&stable)?,
            );
            match damage {
                "missing" => fs::remove_file(&damaged)?,
                "compression" => {
                    let mut image = fs::read(&damaged)?;
                    let start = usize::try_from(u64::from_le_bytes(image[64..72].try_into()?))?;
                    image[start] = 0xff;
                    let checksum = xxhash_rust::xxh3::xxh3_64(&image[64..]);
                    image[32..40].copy_from_slice(&checksum.to_le_bytes());
                    fs::write(&damaged, image)?;
                }
                "missing-fact" => {
                    let untouched_shard = xxhash_rust::xxh3::xxh3_64(untouched.as_bytes()) % 64;
                    let empty_shard = (0..64)
                        .find(|shard| {
                            ![marker_shard, changed_shard, untouched_shard].contains(shard)
                        })
                        .ok_or("missing empty shard")?;
                    let facts = damaged.parent().ok_or("missing facts directory")?;
                    fs::write(&damaged, fs::read(facts.join(format!("{empty_shard:02}")))?)?;
                }
                "corrupt" => fs::write(&damaged, b"corrupt shard")?,
                _ => return Err("unknown damage".into()),
            }
            if trigger == "changed-source" {
                fs::write(fixture.root.join(&changed), "pub fn changed_new() {}\n")?;
            } else {
                corrupt(&path, "payload")?;
            }
            let result = fixture.run(&["index"], Some(&base))?;
            assert!(
                result.status.success(),
                "{damage} {trigger}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let stable_metadata = fs::metadata(&stable)?;
            assert_eq!(
                (
                    stable_metadata.ino(),
                    stable_metadata.modified()?,
                    fs::read(&stable)?
                ),
                stable_before,
                "valid unaffected shard changed: {damage} {trigger}"
            );
            let fresh_base = fixture.work.0.join("fresh-index");
            assert_success(&fixture.run(&["index"], Some(&fresh_base))?);
            assert!(
                fs::read(&path)? == fs::read(derived(&fresh_base)?)?,
                "recovered graph differs from fresh graph: {damage} {trigger}"
            );
            let recovered = fixture.run(
                &["show", "recoverable_marker", "--json", "--read-only"],
                Some(&base),
            )?;
            let fresh = fixture.run(
                &["show", "recoverable_marker", "--json", "--read-only"],
                Some(&fresh_base),
            )?;
            assert_success(&recovered);
            assert_success(&fresh);
            assert_eq!(recovered.stdout, fresh.stdout);
            let output = String::from_utf8_lossy(&recovered.stdout);
            assert!(output.contains("recoverable_marker") && output.contains("calls_marker"));
        }
    }
    Ok(())
}

#[test]
fn failed_update_cannot_leave_new_facts_behind_an_old_file_table() -> TestResult {
    let fixture = Fixture::new()?;
    let marker_shard = xxhash_rust::xxh3::xxh3_64(b"lib.rs") % 64;
    let mut other = (0..100)
        .map(|number| format!("other_{number}.rs"))
        .filter(|name| xxhash_rust::xxh3::xxh3_64(name.as_bytes()) % 64 != marker_shard);
    let changed = other.next().ok_or("missing changed source")?;
    let changed_shard = xxhash_rust::xxh3::xxh3_64(changed.as_bytes()) % 64;
    let later = other
        .find(|name| xxhash_rust::xxh3::xxh3_64(name.as_bytes()) % 64 != changed_shard)
        .ok_or("missing later source")?;
    fs::write(fixture.root.join(&changed), "pub fn changed_old() {}\n")?;
    fs::write(fixture.root.join(&later), "pub fn later_old() {}\n")?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let path = derived(&base)?;
    let files = path.parent().ok_or("missing index store")?.join("files");
    let original_derived = fs::read(&path)?;
    let original_files = fs::read(&files)?;
    let damaged = fact_shard(&path, "lib.rs")?;
    let original_shard = fs::read(&damaged)?;
    fs::remove_file(&damaged)?;
    fs::create_dir(&damaged)?;
    fs::write(fixture.root.join(&changed), "pub fn changed_new() {}\n")?;
    let result = fixture.run(&["index"], Some(&base))?;
    assert!(
        !result.status.success(),
        "blocked shard replacement must fail"
    );
    assert!(fs::read(&path)? == original_derived);
    assert!(fs::read(&files)? == original_files);
    let read_only = fixture.run(&["find", "recoverable_marker", "--read-only"], Some(&base))?;
    assert_eq!(read_only.status.code(), Some(5));
    fs::remove_dir(&damaged)?;
    fs::write(&damaged, original_shard)?;
    fs::write(fixture.root.join(&changed), "pub fn changed_old() {}\n")?;
    assert_success(&fixture.run(&["index"], Some(&base))?);
    fs::write(fixture.root.join(&later), "pub fn later_new() {}\n")?;
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let fresh_base = fixture.work.0.join("fresh-index");
    assert_success(&fixture.run(&["index"], Some(&fresh_base))?);
    assert!(
        fs::read(path)? == fs::read(derived(&fresh_base)?)?,
        "a failed update left stale facts in the graph"
    );
    Ok(())
}
