//! Recovery must not reuse corrupt derived images or write in read-only mode.

#![cfg(unix)]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Output;
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
        self.run_at(&self.root, args, index)
    }

    fn run_at(&self, root: &Path, args: &[&str], index: Option<&Path>) -> io::Result<Output> {
        let mut command = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &self.work.0);
        command
            .args(args)
            .arg("-C")
            .arg(root)
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

    fn git(&self, args: &[&str]) -> io::Result<Output> {
        let output = support::command("git", &self.work.0)
            .arg("-C")
            .arg(&self.root)
            .args([
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "user.name=Fixture",
            ])
            .args(args)
            .output()?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
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
            assert_eq!(read_only.status.code(), Some(5), "{recovery} {kind}");
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
    assert_eq!(output.status.code(), Some(5));
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
    let empty_override = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.work.0)
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
fn every_read_only_command_and_state_leaves_the_filesystem_untouched() -> TestResult {
    let fixture = Fixture::new()?;
    fs::write(
        fixture.root.join("lib.rs"),
        "pub fn recoverable_marker() -> usize { 42 }\npub fn caller() -> usize { recoverable_marker() }\n",
    )?;
    assert_success(&fixture.run(&["index"], None)?);
    let commands: [&[&str]; 7] = [
        &["map"],
        &["find", "marker"],
        &["ctx", "marker"],
        &["show", "caller"],
        &["impact", "recoverable_marker"],
        &["path", "caller", "recoverable_marker"],
        &["audit"],
    ];
    let read_only = |args: &[&str], index: Option<&Path>| -> io::Result<(Option<i32>, bool)> {
        let before = snapshot(&fixture.work.0)?;
        let mut all = args.to_vec();
        all.push("--read-only");
        let output = fixture.run(&all, index)?;
        Ok((output.status.code(), snapshot(&fixture.work.0)? == before))
    };
    // Fresh default index, then the same index without write permission.
    for args in commands {
        assert_eq!(read_only(args, None)?, (Some(0), true), "fresh {args:?}");
    }
    let base = fixture.temp.join("xcalygraph-xcalygraph-recovery-test");
    set_writable(&base, false)?;
    let denied: Vec<_> = commands
        .iter()
        .map(|args| read_only(args, None))
        .collect::<io::Result<_>>()?;
    set_writable(&base, true)?;
    for (args, result) in commands.iter().zip(denied) {
        assert_eq!(result, (Some(0), true), "non-writable {args:?}");
    }
    // Explicit location that does not exist: stale, nothing created.
    let explicit = fixture.work.0.join("explicit");
    assert_eq!(
        read_only(&["find", "marker"], Some(&explicit))?,
        (Some(5), true)
    );
    assert!(!explicit.exists());
    // Changed source: stale for every command, nothing written.
    fs::write(fixture.root.join("added.rs"), "pub fn added() {}\n")?;
    for args in commands {
        assert_eq!(read_only(args, None)?, (Some(5), true), "stale {args:?}");
    }
    Ok(())
}

fn set_writable(path: &Path, writable: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for child in fs::read_dir(path)? {
            set_writable(&child?.path(), writable)?;
        }
    }
    let mode = metadata.permissions().mode();
    let mode = if writable {
        mode | 0o200
    } else {
        mode & !0o222
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
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
fn impact_diff_recovers_or_reports_every_damaged_required_shard() -> TestResult {
    let mut failures = Vec::new();
    for damage in ["missing", "corrupt", "incomplete", "wrong-generation"] {
        let fixture = Fixture::new()?;
        fs::remove_file(fixture.root.join("lib.rs"))?;
        fs::write(
            fixture.root.join("go.mod"),
            "module example.com/probe\n\ngo 1.23\n",
        )?;
        fs::write(
            fixture.root.join("api.go"),
            "package probe\n\nfunc Retired() {}\n",
        )?;
        let caller = "caller.go";
        let caller_shard = xxhash_rust::xxh3::xxh3_64(caller.as_bytes()) % 64;
        assert_ne!(caller_shard, xxhash_rust::xxh3::xxh3_64(b"api.go") % 64);
        fs::write(
            fixture.root.join(caller),
            "package probe\n\nfunc Caller() { Retired() }\n",
        )?;
        fixture.git(&["init", "-q"])?;
        fixture.git(&["add", "."])?;
        fixture.git(&["commit", "-q", "-m", "initial"])?;
        let base = fixture.work.0.join("index");
        assert_success(&fixture.run(&["index"], Some(&base))?);
        fs::remove_file(fixture.root.join("api.go"))?;
        assert_success(&fixture.run(&["index"], Some(&base))?);

        let current_derived = derived(&base)?;
        let current_store = current_derived
            .parent()
            .ok_or("missing current index directory")?;
        let damaged_shard = fact_shard(&current_derived, caller)?;
        match damage {
            "missing" => fs::remove_file(&damaged_shard)?,
            "corrupt" => fs::write(&damaged_shard, b"corrupt shard")?,
            "incomplete" => {
                let empty = (0..64)
                    .find(|shard| *shard as u64 != caller_shard)
                    .ok_or("missing empty shard")?;
                fs::copy(
                    current_store.join(format!("facts/{empty:02}")),
                    &damaged_shard,
                )?;
            }
            "wrong-generation" => {
                let alternate = fixture.work.0.join("alternate");
                fs::create_dir(&alternate)?;
                fs::write(
                    alternate.join("go.mod"),
                    "module example.com/probe\n\ngo 1.23\n",
                )?;
                fs::write(
                    alternate.join(caller),
                    "package probe\n\nfunc Caller() { OtherMissing() }\n",
                )?;
                let alternate_index = fixture.work.0.join("alternate-index");
                assert_success(&fixture.run_at(&alternate, &["index"], Some(&alternate_index))?);
                let alternate_shard = fact_shard(&derived(&alternate_index)?, caller)?;
                fs::copy(alternate_shard, &damaged_shard)?;
            }
            _ => return Err("unknown shard damage".into()),
        }

        let facts_dir = current_store.join("facts");
        let before_shards = snapshot(&facts_dir)?;
        let before_index = snapshot(&base)?;
        let before_root = snapshot(&fixture.root)?;
        let before_home = snapshot(&fixture.home)?;
        let read_only = fixture.run(&["impact", "--diff", "--json", "--read-only"], Some(&base))?;
        let read_only_unchanged = snapshot(&base)? == before_index
            && snapshot(&fixture.root)? == before_root
            && snapshot(&fixture.home)? == before_home;
        if read_only.status.code() != Some(5) || !read_only_unchanged {
            failures.push(format!(
                "{damage}: read-only status {:?}, unchanged={read_only_unchanged}, stderr={}",
                read_only.status.code(),
                String::from_utf8_lossy(&read_only.stderr)
            ));
        }

        let writable = fixture.run(&["impact", "--diff", "--json"], Some(&base))?;
        let writable_text = String::from_utf8_lossy(&writable.stdout);
        if !writable.status.success()
            || !writable_text.contains("Caller")
            || !writable_text.contains("\"conf\":\"inferred\"")
        {
            failures.push(format!(
                "{damage}: writable status {:?}, stdout={}, stderr={}",
                writable.status.code(),
                writable_text,
                String::from_utf8_lossy(&writable.stderr)
            ));
        }

        let after_shards = snapshot(&facts_dir)?;
        let shard_name = damaged_shard
            .file_name()
            .ok_or("missing damaged shard name")?;
        let before_others: BTreeMap<_, _> = before_shards
            .iter()
            .filter(|(path, _)| {
                !path.as_os_str().is_empty()
                    && path.file_name().is_none_or(|name| name != shard_name)
            })
            .collect();
        let after_others: BTreeMap<_, _> = after_shards
            .iter()
            .filter(|(path, _)| {
                !path.as_os_str().is_empty()
                    && path.file_name().is_none_or(|name| name != shard_name)
            })
            .collect();
        if before_others != after_others {
            failures.push(format!(
                "{damage}: recovery changed an unrelated fact shard"
            ));
        }
        let before_clean_read_only = (
            snapshot(&base)?,
            snapshot(&fixture.root)?,
            snapshot(&fixture.home)?,
        );
        let read_only_after =
            fixture.run(&["impact", "--diff", "--json", "--read-only"], Some(&base))?;
        let clean_read_only_unchanged = snapshot(&base)? == before_clean_read_only.0
            && snapshot(&fixture.root)? == before_clean_read_only.1
            && snapshot(&fixture.home)? == before_clean_read_only.2;
        if !read_only_after.status.success() || !clean_read_only_unchanged {
            failures.push(format!(
                "{damage}: repaired index is not cleanly readable without writes"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn wrong_generation_shard_cannot_contaminate_a_rebuilt_graph() -> TestResult {
    let fixture = Fixture::new()?;
    fs::remove_file(fixture.root.join("lib.rs"))?;
    fs::write(
        fixture.root.join("go.mod"),
        "module example.com/probe\n\ngo 1.23\n",
    )?;
    fs::write(
        fixture.root.join("api.go"),
        "package probe\n\nfunc Retired() {}\n",
    )?;
    let mut used_shards = BTreeSet::from([xxhash_rust::xxh3::xxh3_64(b"api.go") % 64]);
    let choose_path = |prefix: &str, used: &mut BTreeSet<u64>| {
        (0..1000)
            .map(|number| format!("{prefix}{number}.go"))
            .find(|path| used.insert(xxhash_rust::xxh3::xxh3_64(path.as_bytes()) % 64))
    };
    let caller = choose_path("caller", &mut used_shards).ok_or("missing caller shard")?;
    let helper = choose_path("helper", &mut used_shards).ok_or("missing helper shard")?;
    let trigger = choose_path("trigger", &mut used_shards).ok_or("missing trigger shard")?;
    fs::write(
        fixture.root.join(&caller),
        "package probe\n\nfunc Caller() { Retired() }\n",
    )?;
    fs::write(
        fixture.root.join(&helper),
        "package probe\n\nfunc Helper() {}\n",
    )?;
    fs::write(
        fixture.root.join(&trigger),
        "package probe\n\nfunc Trigger() int { return 1 }\n",
    )?;
    fixture.git(&["init", "-q"])?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-q", "-m", "initial"])?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    fs::remove_file(fixture.root.join("api.go"))?;
    assert_success(&fixture.run(&["index"], Some(&base))?);

    let alternate = fixture.work.0.join("alternate");
    fs::create_dir(&alternate)?;
    fs::write(
        alternate.join("go.mod"),
        "module example.com/probe\n\ngo 1.23\n",
    )?;
    fs::write(
        alternate.join(&caller),
        "package probe\n\nfunc Caller() { Helper() }\n",
    )?;
    fs::write(
        alternate.join(&helper),
        "package probe\n\nfunc Helper() {}\n",
    )?;
    fs::write(
        alternate.join(&trigger),
        "package probe\n\nfunc Trigger() int { return 1 }\n",
    )?;
    let alternate_index = fixture.work.0.join("alternate-index");
    assert_success(&fixture.run_at(&alternate, &["index"], Some(&alternate_index))?);
    let foreign_shard = fact_shard(&derived(&alternate_index)?, &caller)?;
    let current_shard = fact_shard(&derived(&base)?, &caller)?;
    fs::copy(foreign_shard, current_shard)?;

    fs::write(
        fixture.root.join(&trigger),
        "package probe\n\nfunc Trigger() int { return 2 }\n",
    )?;
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let fresh = fixture.work.0.join("fresh-index");
    assert_success(&fixture.run(&["index"], Some(&fresh))?);
    let rebuilt = fs::read(derived(&base)?)?;
    let expected = fs::read(derived(&fresh)?)?;
    if rebuilt != expected {
        return Err("rebuilt derived index differs from fresh index".into());
    }
    let impact = fixture.run(&["impact", "--diff", "--json"], Some(&base))?;
    assert_success(&impact);
    let text = String::from_utf8(impact.stdout)?;
    assert!(
        text.contains("Caller") && text.contains("\"conf\":\"inferred\""),
        "{text}"
    );
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

#[test]
fn default_index_reuse_does_not_create_or_remove_probe_entries() -> TestResult {
    let fixture = Fixture::new()?;
    assert_success(&fixture.run(&["index"], None)?);
    let base = fixture.temp.join("xcalygraph-xcalygraph-recovery-test");
    let before = snapshot(&base)?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    let output = fixture.run(&["find", "recoverable_marker"], None)?;
    assert_success(&output);
    assert!(output.stderr.is_empty());
    assert!(snapshot(&base)? == before, "shared base was modified");
    Ok(())
}

#[test]
fn concurrent_default_index_users_share_one_valid_index() -> TestResult {
    let fixture = Fixture::new()?;
    let results = std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|_| scope.spawn(|| fixture.run(&["find", "recoverable_marker"], None)))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| io::Error::other("query thread panicked"))?
            })
            .collect::<io::Result<Vec<_>>>()
    })?;
    for output in results {
        assert_success(&output);
        assert!(output.stderr.is_empty());
        assert!(String::from_utf8_lossy(&output.stdout).contains("recoverable_marker"));
    }
    assert!(!fixture.root.join(".xcalygraph").exists());
    Ok(())
}

#[test]
fn existing_shared_index_is_reused_when_the_base_cannot_create_entries() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new()?;
    assert_success(&fixture.run(&["index"], None)?);
    let base = fixture.temp.join("xcalygraph-xcalygraph-recovery-test");
    let permissions = fs::metadata(&base)?.permissions();
    fs::set_permissions(&base, fs::Permissions::from_mode(0o500))?;
    let result = fixture.run(&["find", "recoverable_marker"], None);
    fs::set_permissions(&base, permissions)?;
    let output = result?;
    assert_success(&output);
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.root.join(".xcalygraph").exists());
    assert!(String::from_utf8_lossy(&output.stdout).contains("recoverable_marker"));
    Ok(())
}

#[test]
fn shared_write_failure_falls_back_without_publishing_partial_state() -> TestResult {
    let fixture = Fixture::new()?;
    let marker_shard = xxhash_rust::xxh3::xxh3_64(b"lib.rs") % 64;
    let changed = (0..100)
        .map(|number| format!("changed_{number}.rs"))
        .find(|name| xxhash_rust::xxh3::xxh3_64(name.as_bytes()) % 64 != marker_shard)
        .ok_or("missing second shard")?;
    fs::write(fixture.root.join(&changed), "pub fn original_marker() {}\n")?;
    assert_success(&fixture.run(&["index"], None)?);
    let base = fixture.temp.join("xcalygraph-xcalygraph-recovery-test");
    let shared_derived = derived(&base)?;
    let blocked_shard = fact_shard(&shared_derived, "lib.rs")?;
    fs::remove_file(&blocked_shard)?;
    fs::create_dir(&blocked_shard)?;
    fs::write(fixture.root.join(&changed), "pub fn updated_marker() {}\n")?;

    let explicit = fixture.run(&["index"], Some(&base))?;
    assert!(
        !explicit.status.success(),
        "explicit locations must not fall back"
    );
    assert!(!fixture.root.join(".xcalygraph").exists());
    let environment_override = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.work.0)
        .args(["index", "-C"])
        .arg(&fixture.root)
        .env("HOME", &fixture.home)
        .env("TMPDIR", &fixture.temp)
        .env("XDG_CONFIG_HOME", fixture.home.join("config"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("XCALYGRAPH_INDEX_DIR", &base)
        .output()?;
    assert!(
        !environment_override.status.success(),
        "environment overrides must not fall back"
    );
    assert!(!fixture.root.join(".xcalygraph").exists());
    let shared_read_only =
        fixture.run(&["find", "recoverable_marker", "--read-only"], Some(&base))?;
    assert_eq!(shared_read_only.status.code(), Some(5));

    let local = fixture.run(&["find", "updated_marker"], None)?;
    assert_success(&local);
    assert!(String::from_utf8_lossy(&local.stderr).contains(".xcalygraph"));
    assert!(String::from_utf8_lossy(&local.stdout).contains("updated_marker"));
    let before = snapshot(&fixture.work.0)?;
    for marker in ["recoverable_marker", "updated_marker"] {
        let output = fixture.run(&["find", marker, "--read-only"], None)?;
        assert_success(&output);
        assert!(String::from_utf8_lossy(&output.stdout).contains(marker));
    }
    assert!(
        snapshot(&fixture.work.0)? == before,
        "read-only fallback wrote files"
    );
    Ok(())
}

#[test]
fn source_read_failure_does_not_fall_back_to_a_local_index() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new()?;
    assert_success(&fixture.run(&["index"], None)?);
    let source = fixture.root.join("lib.rs");
    fs::write(&source, "pub fn changed_source_marker() {}\n")?;
    let permissions = fs::metadata(&source)?.permissions();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o000))?;
    let result = fixture.run(&["find", "changed_source_marker"], None);
    fs::set_permissions(&source, permissions)?;
    let output = result?;
    assert!(
        !output.status.success(),
        "unreadable source must be reported"
    );
    assert!(!fixture.root.join(".xcalygraph").exists());
    Ok(())
}

#[test]
fn last_use_symlink_never_truncates_or_modifies_its_target() -> TestResult {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new()?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let path = derived(&base)?;
    let stamp = path.parent().ok_or("missing index store")?.join("used");
    let target = fixture.work.0.join("foreign");
    fs::write(&target, b"preserve foreign content")?;
    fs::File::open(&target)?
        .set_modified(SystemTime::now() - std::time::Duration::from_secs(7200))?;
    fs::remove_file(&stamp)?;
    symlink(&target, &stamp)?;
    let target_before = fs::metadata(&target)?;
    let link_before = fs::symlink_metadata(&stamp)?;
    assert_success(&fixture.run(&["find", "recoverable_marker"], Some(&base))?);
    let target_after = fs::metadata(&target)?;
    let link_after = fs::symlink_metadata(&stamp)?;
    assert_eq!(fs::read(&target)?, b"preserve foreign content");
    assert_eq!(
        (target_after.ino(), target_after.modified()?),
        (target_before.ino(), target_before.modified()?)
    );
    assert_eq!(
        (link_after.ino(), link_after.modified()?),
        (link_before.ino(), link_before.modified()?)
    );
    Ok(())
}

#[test]
fn last_use_of_an_ordinary_index_still_advances() -> TestResult {
    let fixture = Fixture::new()?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let path = derived(&base)?;
    let stamp = path.parent().ok_or("missing index store")?.join("used");
    let old = SystemTime::now() - std::time::Duration::from_secs(7200);
    fs::File::open(&stamp)?.set_modified(old)?;
    let before = fs::metadata(&stamp)?;
    assert_success(&fixture.run(&["find", "recoverable_marker"], Some(&base))?);
    let after = fs::metadata(&stamp)?;
    assert_eq!(before.ino(), after.ino());
    assert!(after.modified()? > old);
    assert!(fs::read(&stamp)?.is_empty());
    Ok(())
}

#[test]
fn last_use_does_not_modify_an_external_hard_link() -> TestResult {
    let fixture = Fixture::new()?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let path = derived(&base)?;
    let stamp = path.parent().ok_or("missing index store")?.join("used");
    let target = fixture.work.0.join("foreign");
    fs::File::create_new(&target)?;
    fs::File::open(&target)?
        .set_modified(SystemTime::now() - std::time::Duration::from_secs(7200))?;
    fs::remove_file(&stamp)?;
    fs::hard_link(&target, &stamp)?;
    let before = fs::metadata(&target)?;
    assert_success(&fixture.run(&["find", "recoverable_marker"], Some(&base))?);
    let after = fs::metadata(&target)?;
    assert_eq!(
        (after.ino(), after.modified()?),
        (before.ino(), before.modified()?)
    );
    Ok(())
}

#[test]
fn writer_lock_never_follows_a_dangling_symlink() -> TestResult {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new()?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let path = derived(&base)?;
    let lock = path.parent().ok_or("missing index store")?.join("lock");
    let target = fixture.work.0.join("foreign");
    fs::remove_file(&lock)?;
    symlink(&target, &lock)?;
    let before = fs::symlink_metadata(&lock)?;
    let output = fixture.run(&["find", "recoverable_marker"], Some(&base))?;
    let after = fs::symlink_metadata(&lock)?;
    assert!(
        !output.status.success(),
        "a symlink cannot be an index writer lock"
    );
    assert!(
        !target.exists(),
        "the lock created a file outside the index"
    );
    assert_eq!(
        (after.ino(), after.modified()?),
        (before.ino(), before.modified()?)
    );
    Ok(())
}

#[test]
fn writer_lock_does_not_lock_a_foreign_hard_link() -> TestResult {
    let fixture = Fixture::new()?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let path = derived(&base)?;
    let lock = path.parent().ok_or("missing index store")?.join("lock");
    let target = fixture.work.0.join("foreign");
    fs::write(&target, b"preserve")?;
    fs::remove_file(&lock)?;
    fs::hard_link(&target, &lock)?;
    let before = fs::metadata(&target)?;
    let output = fixture.run(&["find", "recoverable_marker"], Some(&base))?;
    let after = fs::metadata(&target)?;
    assert!(
        !output.status.success(),
        "a hard link cannot be an index writer lock"
    );
    assert_eq!(fs::read(&target)?, b"preserve");
    assert_eq!(
        (after.ino(), after.modified()?),
        (before.ino(), before.modified()?)
    );
    Ok(())
}

#[test]
fn invalid_default_lock_falls_back_without_touching_its_target() -> TestResult {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new()?;
    assert_success(&fixture.run(&["index"], None)?);
    let base = fixture.temp.join("xcalygraph-xcalygraph-recovery-test");
    let path = derived(&base)?;
    let lock = path.parent().ok_or("missing index store")?.join("lock");
    let target = fixture.work.0.join("foreign");
    fs::remove_file(&lock)?;
    symlink(&target, &lock)?;
    let output = fixture.run(&["find", "recoverable_marker"], None)?;
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains(".xcalygraph"));
    assert!(!target.exists());
    assert!(lock.is_symlink());
    Ok(())
}

#[test]
fn last_use_does_not_overwrite_a_nonempty_stamp() -> TestResult {
    let fixture = Fixture::new()?;
    let base = fixture.work.0.join("index");
    assert_success(&fixture.run(&["index"], Some(&base))?);
    let path = derived(&base)?;
    let stamp = path.parent().ok_or("missing index store")?.join("used");
    fs::write(&stamp, b"preserve")?;
    fs::File::open(&stamp)?
        .set_modified(SystemTime::now() - std::time::Duration::from_secs(7200))?;
    let before = fs::metadata(&stamp)?;
    assert_success(&fixture.run(&["find", "recoverable_marker"], Some(&base))?);
    let after = fs::metadata(&stamp)?;
    assert_eq!(fs::read(&stamp)?, b"preserve");
    assert_eq!(
        (after.ino(), after.modified()?),
        (before.ino(), before.modified()?)
    );
    Ok(())
}
