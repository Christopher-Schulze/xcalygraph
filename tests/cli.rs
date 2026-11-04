//! End-to-end checks of the binary: stdout, stderr and exit codes.

mod support;

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

type TestResult = Result<(), Box<dyn Error>>;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

/// Runs the binary with a HOME that never points at the real one.
fn run(args: &[&str]) -> io::Result<Output> {
    let fixture = TempDir::new("empty-home")?;
    support::command(BINARY, &fixture.0).args(args).output()
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
    let fixture = TempDir::new("closed-stdout")?;
    let mut child = support::command(BINARY, &fixture.0)
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
        let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache");
        fs::create_dir_all(&cache)?;
        if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
            return Err(io::Error::other("test cache must not be a symlink"));
        }
        Self::new_in(&cache, label)
    }

    fn new_in(base: &Path, label: &str) -> io::Result<Self> {
        let base = base.canonicalize()?;
        let prefix = format!("xcalygraph-it-{}-{label}", std::process::id());
        for sequence in 0..1000 {
            let name = if sequence == 0 {
                prefix.clone()
            } else {
                format!("{prefix}-{sequence}")
            };
            let path = base.join(name);
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("no unused test directory"))
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

#[test]
fn temporary_directory_is_created_in_repository_cache() -> TestResult {
    let fixture = TempDir::new("cache-location")?;
    let cache = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".cache")
        .canonicalize()?;
    assert_eq!(fixture.0.parent(), Some(cache.as_path()));
    Ok(())
}

#[test]
fn temporary_directory_preserves_colliding_entries() -> TestResult {
    let root = std::env::temp_dir();
    if std::env::var_os("XCALYGRAPH_CLI_COLLISION_CHILD")
        .is_some_and(|path| Path::new(&path) == root.as_path())
    {
        let prefix = format!("xcalygraph-it-{}-collision", std::process::id());
        let directory = root.join(&prefix);
        let file = root.join(format!("{prefix}-1"));
        fs::create_dir(&directory)?;
        fs::write(directory.join("keep"), b"directory sentinel")?;
        fs::write(&file, b"file sentinel")?;
        #[cfg(unix)]
        {
            let link = root.join(format!("{prefix}-2"));
            std::os::unix::fs::symlink(&directory, link)?;
            std::os::unix::fs::symlink(root.join("missing"), root.join(format!("{prefix}-3")))?;
        }
        let owned = TempDir::new_in(&root, "collision")?;
        assert_ne!(owned.0, directory);
        drop(owned);
        assert_eq!(fs::read(directory.join("keep"))?, b"directory sentinel");
        assert_eq!(fs::read(file)?, b"file sentinel");
        #[cfg(unix)]
        for sequence in [2, 3] {
            assert!(
                fs::symlink_metadata(root.join(format!("{prefix}-{sequence}")))?
                    .file_type()
                    .is_symlink()
            );
        }
        return Ok(());
    }

    let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache");
    fs::create_dir_all(&cache)?;
    if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
        return Err(io::Error::other("test cache must not be a symlink").into());
    }
    let path = cache.join(format!("cli-collision-fixture-{}", std::process::id()));
    fs::create_dir(&path)?;
    let fixture = TempDir(path);
    let output = support::command(std::env::current_exe()?, &fixture.0)
        .args(["--exact", "temporary_directory_preserves_colliding_entries"])
        .env("TMPDIR", &fixture.0)
        .env("XCALYGRAPH_CLI_COLLISION_CHILD", &fixture.0)
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn run_in(root: &Path, index: &Path, args: &[&str]) -> io::Result<Output> {
    support::command(BINARY, index.parent().unwrap_or(root))
        .args(args)
        .arg("-C")
        .arg(root)
        .arg("--index-dir")
        .arg(index)
        .env("HOME", index)
        .output()
}

fn assert_impact_large_depth_matches(
    root: &Path,
    index: &Path,
    args: &[&str],
    expected: &str,
) -> TestResult {
    let mut child = support::command(BINARY, index.parent().unwrap_or(root))
        .args(args)
        .arg("-C")
        .arg(root)
        .arg("--index-dir")
        .arg(index)
        .env("HOME", index)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let started = std::time::Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            assert_eq!(
                output.status.code(),
                Some(0),
                "args {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8(output.stdout)?, expected, "args {args:?}");
            return Ok(());
        }
        if started.elapsed() >= std::time::Duration::from_secs(5) {
            child.kill()?;
            child.wait()?;
            return Err(format!("impact command exceeded 5-second deadline: {args:?}").into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn assert_json_lines_parse(text: &str, fixture: &Path) -> TestResult {
    let mut parser = support::command("python3", fixture)
        .args([
            "-c",
            "import json,sys; [json.loads(line) for line in sys.stdin]",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let Some(mut input) = parser.stdin.take() else {
        return Err(io::Error::other("JSON parser stdin was not piped").into());
    };
    input.write_all(text.as_bytes())?;
    drop(input);
    let output = parser.wait_with_output()?;
    assert!(
        output.status.success(),
        "invalid JSON Lines: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn index_builds_refreshes_lists_and_cleans() -> TestResult {
    let dir = TempDir::new("index")?;
    dir.write("root/.gitignore", "!.env*\n")?;
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
        "files 3 changed 3 deleted 0\n"
    );
    fs::remove_file(root.join(".env"))?;
    let second = run_in(&root, &index, &["index"])?;
    assert_eq!(
        String::from_utf8(second.stdout)?,
        "files 2 changed 0 deleted 1\n"
    );
    let tool = |args: &[&str]| {
        support::command(BINARY, &dir.0)
            .args(args)
            .arg("--index-dir")
            .arg(&index)
            .current_dir(&dir.0)
            .output()
    };
    let listed = String::from_utf8(tool(&["ls"])?.stdout)?;
    assert!(
        listed.contains(&format!("{} ", root.canonicalize()?.display()))
            && listed.contains("1 indexes"),
        "{listed}"
    );
    let cleaned = String::from_utf8(tool(&["clean", "root"])?.stdout)?;
    assert!(cleaned.starts_with("deleted "), "{cleaned}");
    let again = String::from_utf8(tool(&["clean", "root"])?.stdout)?;
    assert_eq!(again, "nothing to delete\n");
    // Without arguments every index goes.
    run_in(&root, &index, &["index"])?;
    let all = String::from_utf8(tool(&["clean"])?.stdout)?;
    assert!(all.starts_with("deleted "), "{all}");
    assert!(String::from_utf8(tool(&["ls"])?.stdout)?.starts_with("no indexes"));
    Ok(())
}

fn index_for(base: &Path, root_name: &str) -> io::Result<PathBuf> {
    let prefix = format!("{root_name}-");
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            return Ok(entry.path());
        }
    }
    Err(io::Error::other("index not found"))
}

fn clean_in(dir: &TempDir, base: &Path, args: &[&str]) -> io::Result<Output> {
    support::command(BINARY, &dir.0)
        .arg("clean")
        .args(args)
        .arg("--index-dir")
        .arg(base)
        .current_dir(&dir.0)
        .env("HOME", dir.0.join("home"))
        .output()
}

#[test]
fn clean_keeps_foreign_local_entries_and_removes_both_indexes() -> TestResult {
    let dir = TempDir::new("clean-local-ownership")?;
    dir.write("repo/main.go", "package main\n")?;
    dir.write("repo/.xcalygraph/notes.txt", "keep local notes")?;
    let root = dir.0.join("repo");
    let shared = dir.0.join("shared");
    let local = root.join(".xcalygraph");
    assert!(run_in(&root, &shared, &["index"])?.status.success());
    assert!(run_in(&root, &local, &["index"])?.status.success());
    let shared_store = index_for(&shared, "repo")?;
    let local_store = index_for(&local, "repo")?;
    let output = clean_in(&dir, &shared, &["repo"])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert_eq!(text.matches("deleted ").count(), 2, "{text}");
    assert!(!shared_store.exists() && !local_store.exists());
    assert_eq!(fs::read(local.join("notes.txt"))?, b"keep local notes");
    Ok(())
}

#[test]
fn clean_preserves_unrecognized_indexes_and_unexpected_contents() -> TestResult {
    let dir = TempDir::new("clean-unowned")?;
    dir.write("safe/main.go", "package safe\n")?;
    dir.write("mixed/main.go", "package mixed\n")?;
    dir.write("corrupt/main.go", "package corrupt\n")?;
    dir.write("nested/main.go", "package nested\n")?;
    let base = dir.0.join("index");
    for name in ["safe", "mixed", "corrupt", "nested"] {
        assert!(
            run_in(&dir.0.join(name), &base, &["index"])?
                .status
                .success()
        );
    }
    let safe = index_for(&base, "safe")?;
    let mixed = index_for(&base, "mixed")?;
    let corrupt = index_for(&base, "corrupt")?;
    let nested = index_for(&base, "nested")?;
    fs::write(mixed.join("precious.txt"), b"preserve unexpected contents")?;
    fs::write(corrupt.join("derived"), b"damaged container")?;
    fs::write(nested.join("facts/extra"), b"preserve nested contents")?;
    dir.write("index/invalid/meta", "unrelated metadata")?;
    dir.write("index/invalid/valuable", "preserve foreign directory")?;
    fs::create_dir(base.join("wrong-identity"))?;
    fs::copy(safe.join("meta"), base.join("wrong-identity/meta"))?;
    dir.write("index/wrong-identity/valuable", "preserve wrong identity")?;
    let output = clean_in(&dir, &base, &[])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(!safe.exists(), "{text}");
    assert_eq!(
        fs::read(mixed.join("precious.txt"))?,
        b"preserve unexpected contents"
    );
    assert_eq!(
        fs::read(base.join("invalid/valuable"))?,
        b"preserve foreign directory"
    );
    assert_eq!(
        fs::read(base.join("wrong-identity/valuable"))?,
        b"preserve wrong identity"
    );
    assert_eq!(fs::read(corrupt.join("derived"))?, b"damaged container");
    assert_eq!(
        fs::read(nested.join("facts/extra"))?,
        b"preserve nested contents"
    );
    assert_eq!(text.matches("skipped ").count(), 5, "{text}");
    Ok(())
}

#[test]
fn clean_missing_only_removes_indexes_of_absent_roots() -> TestResult {
    let dir = TempDir::new("clean-missing")?;
    dir.write("present/main.go", "package present\n")?;
    dir.write("gone/main.go", "package gone\n")?;
    let base = dir.0.join("index");
    for name in ["present", "gone"] {
        assert!(
            run_in(&dir.0.join(name), &base, &["index"])?
                .status
                .success()
        );
    }
    let present = index_for(&base, "present")?;
    let gone = index_for(&base, "gone")?;
    let targeted = clean_in(&dir, &base, &["--missing", "present"])?;
    assert!(targeted.status.success());
    assert!(
        present.is_dir(),
        "targeted --missing removed an existing root"
    );
    fs::remove_dir_all(dir.0.join("gone"))?;
    let all = clean_in(&dir, &base, &["--missing"])?;
    assert!(
        all.status.success(),
        "{}",
        String::from_utf8_lossy(&all.stderr)
    );
    assert!(present.is_dir());
    assert!(!gone.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn clean_preserves_linked_entries_and_reports_them() -> TestResult {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("clean-links")?;
    dir.write("repo/main.go", "package main\n")?;
    dir.write("outside/keep", "preserve target")?;
    let root = dir.0.join("repo");
    let base = dir.0.join("index");
    assert!(run_in(&root, &base, &["index"])?.status.success());
    let store = index_for(&base, "repo")?;
    fs::hard_link(store.join("meta"), dir.0.join("outside/meta"))?;
    symlink(dir.0.join("outside"), root.join(".xcalygraph"))?;
    let output = clean_in(&dir, &base, &["repo"])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(store.is_dir(), "{text}");
    assert!(
        fs::symlink_metadata(root.join(".xcalygraph"))?
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(dir.0.join("outside/keep"))?, b"preserve target");
    assert!(dir.0.join("outside/meta").is_file());
    assert_eq!(text.matches("skipped ").count(), 2, "{text}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn clean_preserves_symlinked_index_directory() -> TestResult {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("clean-index-link")?;
    dir.write("repo/main.go", "package main\n")?;
    let root = dir.0.join("repo");
    let base = dir.0.join("index");
    assert!(run_in(&root, &base, &["index"])?.status.success());
    let store = index_for(&base, "repo")?;
    let other = dir.0.join("other");
    fs::create_dir(&other)?;
    let link = other.join(
        store
            .file_name()
            .ok_or_else(|| io::Error::other("index name"))?,
    );
    symlink(&store, &link)?;
    let output = clean_in(&dir, &other, &["repo"])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fs::symlink_metadata(&link)?.file_type().is_symlink());
    assert!(store.join("meta").is_file());
    assert!(String::from_utf8(output.stdout)?.contains("skipped "));
    Ok(())
}

#[cfg(unix)]
#[test]
fn clean_preserves_symlinked_shards_and_their_target() -> TestResult {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("clean-shard-link")?;
    dir.write("repo/main.go", "package main\n")?;
    let root = dir.0.join("repo");
    let base = dir.0.join("index");
    assert!(run_in(&root, &base, &["index"])?.status.success());
    let store = index_for(&base, "repo")?;
    let shards = dir.0.join("saved-shards");
    fs::rename(store.join("facts"), &shards)?;
    symlink(&shards, store.join("facts"))?;
    let output = clean_in(&dir, &base, &["repo"])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::symlink_metadata(store.join("facts"))?
            .file_type()
            .is_symlink()
    );
    assert!(shards.is_dir());
    assert!(store.join("meta").is_file());
    assert!(String::from_utf8(output.stdout)?.contains("skipped "));
    Ok(())
}

fn verified_previous_format_cleanup() -> TestResult {
    for legacy_format in [8_u32] {
        let dir = TempDir::new(&format!("clean-previous-format-{legacy_format}"))?;
        dir.write("repo/main.go", "package main\n")?;
        let root = dir.0.join("repo");
        let base = dir.0.join("index");
        assert!(run_in(&root, &base, &["index"])?.status.success());
        let store = index_for(&base, "repo")?;
        let mut converted = 0;
        for (path, mut content, _) in index_snapshot(&store)? {
            if content.starts_with(b"XCALYGR\0") {
                content[8..12].copy_from_slice(&legacy_format.to_le_bytes());
                fs::write(path, content)?;
                converted += 1;
            }
        }
        assert!(converted > 2);
        let listed = support::command(BINARY, &dir.0)
            .arg("ls")
            .arg("--index-dir")
            .arg(&base)
            .env("HOME", dir.0.join("home"))
            .output()?;
        assert!(
            String::from_utf8(listed.stdout)?.contains(&root.canonicalize()?.display().to_string()),
            "format {legacy_format}"
        );
        let foreign = store.join("foreign");
        fs::write(&foreign, b"preserve")?;
        let skipped = clean_in(&dir, &base, &["repo"])?;
        assert!(String::from_utf8(skipped.stdout)?.contains("skipped "));
        assert_eq!(fs::read(&foreign)?, b"preserve");
        fs::remove_file(foreign)?;
        let removed = clean_in(&dir, &base, &["repo"])?;
        assert!(String::from_utf8(removed.stdout)?.starts_with("deleted "));
        assert!(!store.exists());
    }
    Ok(())
}

#[test]
fn unsupported_previous_format_is_not_listed_or_cleaned() -> TestResult {
    let dir = TempDir::new("clean-unsupported-format")?;
    dir.write("repo/main.go", "package main\n")?;
    let root = dir.0.join("repo");
    let base = dir.0.join("index");
    assert!(run_in(&root, &base, &["index"])?.status.success());
    let store = index_for(&base, "repo")?;
    let mut converted = 0;
    for (path, mut content, _) in index_snapshot(&store)? {
        if content.starts_with(b"XCALYGR\0") {
            content[8..12].copy_from_slice(&7_u32.to_le_bytes());
            fs::write(path, content)?;
            converted += 1;
        }
    }
    assert!(converted > 2);
    let before = index_snapshot(&store)?;
    let listed = support::command(BINARY, &dir.0)
        .arg("ls")
        .arg("--index-dir")
        .arg(&base)
        .env("HOME", dir.0.join("home"))
        .output()?;
    assert!(listed.status.success());
    assert!(
        !String::from_utf8(listed.stdout)?.contains(&root.canonicalize()?.display().to_string())
    );
    let cleaned = clean_in(&dir, &base, &["repo"])?;
    assert!(String::from_utf8(cleaned.stdout)?.contains("skipped "));
    assert_eq!(index_snapshot(&store)?, before);
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

#[cfg(unix)]
#[test]
fn confined_agents_get_the_index_inside_the_root() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("confined")?;
    dir.write("root/main.go", "package main\n")?;
    let locked = dir.0.join("tmp");
    fs::create_dir_all(&locked)?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500))?;
    let output = support::command(BINARY, &dir.0)
        .args(["index", "-C"])
        .arg(dir.0.join("root"))
        .env("TMPDIR", &locked)
        .env_remove("XCALYGRAPH_INDEX_DIR")
        .output()?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "files 1 changed 1 deleted 0\n"
    );
    assert!(String::from_utf8(output.stderr)?.contains(".xcalygraph/ to .gitignore"));
    assert!(dir.0.join("root/.xcalygraph").is_dir());
    Ok(())
}

#[test]
fn install_writes_the_skill_and_version_reports_mismatches() -> TestResult {
    let dir = TempDir::new("install")?;
    let home = dir.0.join("home");
    let with_home = |args: &[&str]| {
        support::command(BINARY, &dir.0)
            .args(args)
            .env("HOME", &home)
            .output()
    };
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
    let copied_version = support::command(&copied, &dir.0).arg("version").output()?;
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
fn version_detects_and_install_repairs_skill_content_mismatches() -> TestResult {
    let dir = TempDir::new("skill-content")?;
    let home = dir.0.join("home");
    let invoke = |args: &[&str]| {
        support::command(BINARY, &dir.0)
            .args(args)
            .env("HOME", &home)
            .output()
    };
    let installed = invoke(&["install", "--skill-only", "--no-link"])?;
    assert_eq!(installed.status.code(), Some(0));

    let skill = home.join(".agents/skills/xcalygraph/SKILL.md");
    let stamp = home.join(".agents/skills/xcalygraph/.version");
    assert_eq!(
        fs::read(&skill)?,
        include_bytes!("../skills/xcalygraph/SKILL.md")
    );
    assert_eq!(
        fs::read_to_string(&stamp)?.trim(),
        env!("CARGO_PKG_VERSION")
    );
    let matching = invoke(&["version"])?;
    assert_eq!(matching.status.code(), Some(0));
    assert!(!String::from_utf8(matching.stdout)?.contains("! installed skill"));

    fs::write(&skill, b"modified installed skill\n")?;
    let changed = invoke(&["version"])?;
    let changed = String::from_utf8(changed.stdout)?;
    assert!(
        changed.contains(&format!(
            "! installed skill content differs from binary {}; run 'xcalygraph install'",
            env!("CARGO_PKG_VERSION")
        )),
        "{changed}"
    );

    let root = dir.0.join("repo");
    fs::create_dir_all(&root)?;
    fs::write(root.join("main.go"), "package main\n\nfunc main() {}\n")?;
    let query = support::command(BINARY, &dir.0)
        .args(["map", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(dir.0.join("index"))
        .env("HOME", &home)
        .output()?;
    assert_eq!(
        query.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&query.stderr)
    );
    assert!(
        !String::from_utf8(query.stdout)?.contains("! installed skill"),
        "query commands stay silent about install state"
    );

    fs::remove_file(&skill)?;
    let missing = invoke(&["version"])?;
    let missing = String::from_utf8(missing.stdout)?;
    assert!(
        missing.contains("! installed skill content differs from binary"),
        "{missing}"
    );
    let repaired = invoke(&["install", "--skill-only", "--no-link"])?;
    assert_eq!(repaired.status.code(), Some(0));
    assert!(
        String::from_utf8(repaired.stdout)?.contains(" written\n"),
        "skill content must be restored"
    );
    assert_eq!(
        fs::read(&skill)?,
        include_bytes!("../skills/xcalygraph/SKILL.md")
    );
    assert_eq!(
        fs::read_to_string(&stamp)?.trim(),
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}

#[test]
fn install_test_does_not_inherit_an_external_install_destination() -> TestResult {
    let fixture = TempDir::new("inherited-install")?;
    let foreign = fixture.0.join("foreign");
    fs::create_dir(&foreign)?;
    fs::write(foreign.join("xcalygraph"), b"preserve foreign binary")?;
    let output = support::command(std::env::current_exe()?, &fixture.0)
        .args([
            "--exact",
            "install_writes_the_skill_and_version_reports_mismatches",
        ])
        .env("HOME", fixture.0.join("home"))
        .env("TMPDIR", &fixture.0)
        .env("XCALYGRAPH_INSTALL_DIR", &foreign)
        .output()?;
    let foreign_binary = foreign.join("xcalygraph");
    assert!(
        fs::metadata(&foreign_binary)?.len() == b"preserve foreign binary".len() as u64
            && fs::read(&foreign_binary)? == b"preserve foreign binary",
        "foreign install destination was changed"
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn installer_test_ignores_inherited_shell_startup_and_tar_options() -> TestResult {
    let root = std::env::temp_dir();
    if std::env::var_os("XCALYGRAPH_ENV_INSPECTION_CHILD")
        .is_some_and(|path| Path::new(&path) == root.as_path())
    {
        let environment = support::command("/bin/sh", &root)
            .args([
                "-c",
                "test -z \"${BASH_ENV+x}\" && test -z \"${TAR_OPTIONS+x}\"",
            ])
            .status()?;
        assert!(environment.success(), "shell options were inherited");
        return Ok(());
    }
    let fixture = TempDir::new("inherited-shell")?;
    fixture.write("startup.sh", "printf started > \"$TEST_SHELL_MARKER\"\n")?;
    let marker = fixture.0.join("startup-ran");
    let output = support::command(std::env::current_exe()?, &fixture.0)
        .args([
            "--exact",
            "installer_without_skill_preserves_foreign_temporary_entries",
        ])
        .env("BASH_ENV", fixture.0.join("startup.sh"))
        .env("TAR_OPTIONS", "--xcalygraph-invalid-option")
        .env("TEST_SHELL_MARKER", &marker)
        .output()?;
    assert!(!marker.exists(), "inherited shell startup file ran");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let inspection = support::command(std::env::current_exe()?, &fixture.0)
        .args([
            "--exact",
            "installer_test_ignores_inherited_shell_startup_and_tar_options",
        ])
        .env("XCALYGRAPH_ENV_INSPECTION_CHILD", &fixture.0)
        .env("BASH_ENV", fixture.0.join("startup.sh"))
        .env("TAR_OPTIONS", "--xcalygraph-invalid-option")
        .output()?;
    assert!(
        inspection.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&inspection.stdout),
        String::from_utf8_lossy(&inspection.stderr)
    );
    Ok(())
}

fn gate_fixture(label: &str) -> io::Result<(TempDir, PathBuf)> {
    let fixture = TempDir::new(label)?;
    let root = fixture.0.join("project");
    for directory in ["scripts/tests", "scripts/utils"] {
        fs::create_dir_all(root.join(directory))?;
    }
    for relative in ["scripts/tests/test.sh", "scripts/utils/prune-cache.sh"] {
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(relative),
            root.join(relative),
        )?;
    }
    fs::write(root.join("Cargo.toml"), "not a valid manifest\n")?;
    Ok((fixture, root))
}

#[test]
fn gate_refuses_blessing_before_running_checks() -> TestResult {
    let (fixture, root) = gate_fixture("gate-blessing")?;
    for value in ["1", ""] {
        let output = support::command("bash", &fixture.0)
            .arg(root.join("scripts/tests/test.sh"))
            .env("XCALYGRAPH_BLESS", value)
            .output()?;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("XCALYGRAPH_BLESS"));
        assert!(
            output.stdout.is_empty(),
            "checks ran before rejecting blessing"
        );
        assert!(!root.join(".cache").exists());
    }
    Ok(())
}

#[test]
fn gate_propagates_real_shellcheck_findings() -> TestResult {
    let (fixture, root) = gate_fixture("gate-shellcheck")?;
    fs::write(
        root.join("scripts/utils/invalid.sh"),
        "#!/usr/bin/env bash\necho $UNQUOTED\n",
    )?;
    let output = support::command("bash", &fixture.0)
        .arg(root.join("scripts/tests/test.sh"))
        .arg("--all")
        .output()?;
    assert!(!output.status.success());
    let diagnostics = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(diagnostics.contains("SC2086"), "{diagnostics}");
    // The product gate leaves script checks to --all.
    let product = support::command("bash", &fixture.0)
        .arg(root.join("scripts/tests/test.sh"))
        .output()?;
    let diagnostics = String::from_utf8_lossy(&product.stdout).to_string()
        + &String::from_utf8_lossy(&product.stderr);
    assert!(!diagnostics.contains("SC2086"), "{diagnostics}");
    Ok(())
}

#[test]
fn update_check_compares_with_the_latest_release() -> TestResult {
    let dir = TempDir::new("update")?;
    dir.write("latest.json", "{\"tag_name\": \"v0.0.1\", \"assets\": []}")?;
    let output = support::command(BINARY, &dir.0)
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
    let refreshed = support::command(BINARY, &dir.0)
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
        fs::read(skill.join("SKILL.md"))?,
        include_bytes!("../skills/xcalygraph/SKILL.md")
    );
    assert_eq!(
        fs::read_to_string(skill.join(".version"))?.trim(),
        env!("CARGO_PKG_VERSION")
    );
    fs::write(skill.join("SKILL.md"), "modified installed skill\n")?;
    let content_refreshed = support::command(BINARY, &dir.0)
        .arg("update")
        .env("HOME", dir.0.join("home"))
        .env(
            "XCALYGRAPH_UPDATE_URL",
            format!("file://{}", dir.0.join("latest.json").display()),
        )
        .output()?;
    assert_eq!(content_refreshed.status.code(), Some(0));
    assert!(String::from_utf8(content_refreshed.stdout)?.contains("written"));
    assert_eq!(
        fs::read(skill.join("SKILL.md"))?,
        include_bytes!("../skills/xcalygraph/SKILL.md")
    );
    dir.write("latest.json", "{\"tag_name\": \"v99.0.0\", \"assets\": []}")?;
    let newer = support::command(BINARY, &dir.0)
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

#[test]
fn update_check_rejects_malformed_metadata_and_decodes_escaped_fields() -> TestResult {
    let dir = TempDir::new("update-json")?;
    let metadata = dir.0.join("latest.json");
    let home = dir.0.join("home");
    let check_metadata = |contents: &[u8]| -> io::Result<Output> {
        fs::write(&metadata, contents)?;
        support::command(BINARY, &dir.0)
            .args(["update", "--check"])
            .env("HOME", &home)
            .env(
                "XCALYGRAPH_UPDATE_URL",
                format!("file://{}", metadata.display()),
            )
            .output()
    };

    let truncated = check_metadata(br#"{"tag_name":"v99.0.0","assets":[]"#)?;
    assert_eq!(truncated.status.code(), Some(1));
    assert!(truncated.stdout.is_empty());
    assert!(String::from_utf8_lossy(&truncated.stderr).contains("release metadata"));
    assert!(!home.exists());

    let invalid_utf8 = check_metadata(b"{\"tag_name\":\"v99.0.0\",\"assets\":[]}\xff")?;
    assert_eq!(invalid_utf8.status.code(), Some(1));
    assert!(invalid_utf8.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid_utf8.stderr).contains("UTF-8"));
    assert!(!home.exists());

    for json in [
        br#"{"tag_name":"v\u0039\u0039.0.0","assets":[]}"#.as_slice(),
        br#"{"tag\u005fname":"v99.0.0","assets":[]}"#.as_slice(),
    ] {
        let decoded = check_metadata(json)?;
        assert_eq!(decoded.status.code(), Some(0));
        let text = String::from_utf8(decoded.stdout)?;
        assert!(
            text.contains(&format!(
                "update available: {} -> 99.0.0",
                env!("CARGO_PKG_VERSION")
            )),
            "{text}"
        );
    }

    for json in [
        br#"{"release":{"tag_name":"v99.0.0"},"assets":[]}"#.as_slice(),
        br#"{"notes":"\"tag_name\":\"v99.0.0\"","assets":[]}"#.as_slice(),
    ] {
        let unrelated = check_metadata(json)?;
        assert_eq!(unrelated.status.code(), Some(1));
        assert!(unrelated.stdout.is_empty());
        assert!(String::from_utf8_lossy(&unrelated.stderr).contains("release metadata"));
        assert!(!home.exists());
    }
    Ok(())
}

/// A release directory whose binary reports `reported_version` and runs `install` through
/// `delegate` (a shell command line receiving the arguments).
fn fake_release(
    dir: &TempDir,
    delegate: &str,
    reported_version: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    fake_release_at(&dir.0.join("release"), delegate, reported_version)
}

fn json_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{08}' => encoded.push_str("\\b"),
            '\u{0c}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            control if control <= '\u{1f}' => {
                encoded.push_str(&format!("\\u{:04x}", u32::from(control)));
            }
            other => encoded.push(other),
        }
    }
    encoded.push('"');
    encoded
}

fn fake_release_at(
    release: &Path,
    delegate: &str,
    reported_version: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    use sha2::Digest;
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = version ]; then echo 'xcalygraph {reported_version}'; exit 0; fi\n{delegate}\n"
    );
    let os = if std::env::consts::OS == "macos" {
        "darwin"
    } else {
        std::env::consts::OS
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    };
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder.append_data(
        &mut header,
        format!("xcalygraph-99.0.0-{os}-{arch}/xcalygraph"),
        script.as_bytes(),
    )?;
    let archive = builder.into_inner()?.finish()?;
    let name = format!("xcalygraph-99.0.0-{os}-{arch}.tar.gz");
    fs::create_dir_all(release)?;
    fs::write(release.join(&name), &archive)?;
    let sum: String = sha2::Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    fs::write(release.join("SHA256SUMS"), format!("{sum}  {name}\n"))?;
    let archive_url = json_string(&format!("file://{}", release.join(&name).display()));
    let sums_url = json_string(&format!("file://{}", release.join("SHA256SUMS").display()));
    fs::write(
        release.join("latest.json"),
        format!(
            "{{\"tag_name\":\"v99.0.0\",\"assets\":[{{\"browser_download_url\":{archive_url}}},{{\"browser_download_url\":{sums_url}}}]}}"
        ),
    )?;
    Ok(release.join("latest.json"))
}

fn installer_release(dir: &TempDir, reported_version: &str) -> TestResult {
    let latest = fake_release(dir, "exit 0", reported_version)?;
    let release = latest
        .parent()
        .ok_or_else(|| io::Error::other("release directory"))?;
    let tagged = release.join("v99.0.0");
    fs::create_dir(&tagged)?;
    for entry in fs::read_dir(release)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.file_name() != "latest.json" {
            fs::copy(entry.path(), tagged.join(entry.file_name()))?;
        }
    }
    fs::create_dir(dir.0.join("tmp"))?;
    Ok(())
}

fn install_without_skill(dir: &TempDir) -> io::Result<Output> {
    let release = dir.0.join("release");
    let mut release_url = String::from("file://");
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &byte in release.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            release_url.push(char::from(byte));
        } else {
            release_url.push('%');
            release_url.push(char::from(HEX[usize::from(byte >> 4)]));
            release_url.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    support::command("bash", &dir.0)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/utils/install.sh"))
        .args(["--version", "99.0.0", "--no-skill"])
        .env("XCALYGRAPH_RELEASE_BASE", release_url)
        .env("XCALYGRAPH_INSTALL_DIR", dir.0.join("bin"))
        .env("HOME", dir.0.join("home"))
        .env("TMPDIR", dir.0.join("tmp"))
        .output()
}

#[cfg(unix)]
#[test]
fn installer_without_skill_preserves_foreign_temporary_entries() -> TestResult {
    use std::os::unix::fs::symlink;

    for (linked, label) in [
        (true, "installer-temp-link"),
        (false, "installer-temp-file"),
        (true, "installer-temp-link \n"),
        (false, "installer-temp-file#%\n\n"),
    ] {
        let dir = TempDir::new(label)?;
        installer_release(&dir, "99.0.0")?;
        fs::create_dir(dir.0.join("bin"))?;
        let temporary = dir.0.join("bin/.xcalygraph.new");
        let foreign = dir.0.join("foreign");
        fs::write(&foreign, b"preserve foreign target")?;
        if linked {
            symlink(&foreign, &temporary)?;
        } else {
            fs::write(&temporary, b"preserve foreign temporary")?;
        }
        fs::write(dir.0.join("bin/xcalygraph"), b"old binary")?;
        let output = install_without_skill(&dir)?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(fs::read_to_string(dir.0.join("bin/xcalygraph"))?.contains("xcalygraph 99.0.0"));
        assert_eq!(fs::read(&foreign)?, b"preserve foreign target");
        if linked {
            assert!(fs::symlink_metadata(&temporary)?.file_type().is_symlink());
        } else {
            assert_eq!(fs::read(&temporary)?, b"preserve foreign temporary");
        }
        for entry in fs::read_dir(dir.0.join("bin"))? {
            assert!(
                !entry?
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".xcalygraph.new.")
            );
        }
    }
    Ok(())
}

#[test]
fn installer_without_skill_preserves_existing_output_on_failure() -> TestResult {
    for case in ["checksum", "smoke", "directory"] {
        let dir = TempDir::new(&format!("installer-failure-{case}"))?;
        installer_release(&dir, if case == "smoke" { "0.0.1" } else { "99.0.0" })?;
        fs::create_dir(dir.0.join("bin"))?;
        let binary = dir.0.join("bin/xcalygraph");
        let temporary = dir.0.join("bin/.xcalygraph.new");
        fs::write(&temporary, b"keep temporary")?;
        if case == "directory" {
            fs::create_dir(&binary)?;
            fs::write(binary.join("keep"), b"keep directory")?;
        } else {
            fs::write(&binary, b"old binary")?;
        }
        if case == "checksum" {
            let sums = dir.0.join("release/v99.0.0/SHA256SUMS");
            let original = fs::read_to_string(&sums)?;
            let (_, name) = original
                .split_once(' ')
                .ok_or_else(|| io::Error::other("checksum"))?;
            fs::write(sums, format!("{} {name}", "0".repeat(64)))?;
        }
        let output = install_without_skill(&dir)?;
        assert!(
            !output.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(fs::read(&temporary)?, b"keep temporary");
        if case == "directory" {
            assert_eq!(fs::read(binary.join("keep"))?, b"keep directory");
            assert_eq!(fs::read_dir(&binary)?.count(), 1);
        } else {
            assert_eq!(fs::read(&binary)?, b"old binary");
        }
    }
    Ok(())
}

fn release_builder_fixture(dir: &TempDir) -> io::Result<PathBuf> {
    dir.write(
        "project/Cargo.toml",
        "[package]\nname = \"xcalygraph\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[dependencies]\nxcalygraph_absent_fixture_dependency = \"=0.0.0\"\n",
    )?;
    dir.write(
        "project/src/main.rs",
        "fn main() { println!(\"fixture\"); }\n",
    )?;
    dir.write("project/LICENSE", "fixture license\n")?;
    let script = dir.0.join("project/scripts/build/build-release.sh");
    fs::create_dir_all(
        script
            .parent()
            .ok_or_else(|| io::Error::other("script parent"))?,
    )?;
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/build/build-release.sh"),
        &script,
    )?;
    let utils = dir.0.join("project/scripts/utils");
    fs::create_dir_all(&utils)?;
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/utils/validate-release-tag.sh"),
        utils.join("validate-release-tag.sh"),
    )?;
    Ok(script)
}

#[test]
fn release_builder_rejects_a_mismatched_tag_before_touching_dist() -> TestResult {
    let dir = TempDir::new("release-tag-mismatch")?;
    let script = release_builder_fixture(&dir)?;
    let output = support::command("/bin/bash", &dir.0)
        .arg(&script)
        .env("CARGO_NET_OFFLINE", "true")
        .env("GITHUB_REF_NAME", "v9.9.9")
        .env("HOME", dir.0.join("home"))
        .current_dir(dir.0.join("project"))
        .output()?;
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)?
            .contains("tag v9.9.9 does not match Cargo package version 0.0.0 (expected v0.0.0)")
    );
    assert!(!dir.0.join("project/dist").exists());
    Ok(())
}

#[test]
fn release_builder_preserves_foreign_dist_on_preparation_failure() -> TestResult {
    let dir = TempDir::new("release-dist-preservation")?;
    let script = release_builder_fixture(&dir)?;
    dir.write("project/dist/foreign.txt", "preserve release data")?;
    let output = support::command("/bin/bash", &dir.0)
        .arg(&script)
        .env("CARGO_NET_OFFLINE", "true")
        .env("GITHUB_REF_NAME", "v0.0.0")
        .env("HOME", dir.0.join("home"))
        .current_dir(dir.0.join("project"))
        .output()?;
    assert!(!output.status.success());
    assert_eq!(
        fs::read(dir.0.join("project/dist/foreign.txt"))?,
        b"preserve release data"
    );
    assert_eq!(fs::read_dir(dir.0.join("project/dist"))?.count(), 1);
    let os = if std::env::consts::OS == "macos" {
        "darwin"
    } else {
        std::env::consts::OS
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    };
    let conflict = dir
        .0
        .join(format!("project/dist/xcalygraph-0.0.0-{os}-{arch}.tar.gz"));
    fs::write(&conflict, b"keep conflicting archive")?;
    let output = support::command("/bin/bash", &dir.0)
        .arg(&script)
        .env("CARGO_NET_OFFLINE", "true")
        .env("GITHUB_REF_NAME", "v0.0.0")
        .env("HOME", dir.0.join("home"))
        .current_dir(dir.0.join("project"))
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("output already exists"));
    assert_eq!(fs::read(conflict)?, b"keep conflicting archive");
    assert_eq!(
        fs::read(dir.0.join("project/dist/foreign.txt"))?,
        b"preserve release data"
    );
    assert_eq!(fs::read_dir(dir.0.join("project/dist"))?.count(), 2);
    let unsupported = support::command("/bin/bash", &dir.0)
        .arg(&script)
        .arg("unsupported-target")
        .env("CARGO_NET_OFFLINE", "true")
        .env("GITHUB_REF_NAME", "v0.0.0")
        .env("HOME", dir.0.join("home"))
        .current_dir(dir.0.join("project"))
        .output()?;
    assert!(!unsupported.status.success());
    assert!(String::from_utf8(unsupported.stderr)?.contains("unsupported target"));
    assert_eq!(fs::read_dir(dir.0.join("project/dist"))?.count(), 2);
    let duplicate = TempDir::new("release-duplicate-target")?;
    let duplicate_script = release_builder_fixture(&duplicate)?;
    let output = support::command("/bin/bash", &duplicate.0)
        .arg(duplicate_script)
        .args(["aarch64-apple-darwin", "aarch64-apple-darwin"])
        .env("CARGO_NET_OFFLINE", "true")
        .env("GITHUB_REF_NAME", "v0.0.0")
        .env("HOME", duplicate.0.join("home"))
        .current_dir(duplicate.0.join("project"))
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("duplicate output"));
    Ok(())
}

#[test]
fn release_packaging_preserves_foreign_files_and_rejects_conflicts() -> TestResult {
    use sha2::Digest;

    let dir = TempDir::new("release-packaging")?;
    let script = release_builder_fixture(&dir)?;
    let stage = dir.0.join("stage with spaces");
    let dist = dir.0.join("dist with spaces");
    fs::create_dir(&stage)?;
    fs::create_dir(&dist)?;
    fs::write(stage.join("THIRD-PARTY-LICENSES.txt"), b"fixture notices")?;
    fs::write(dist.join("foreign.tar.gz"), b"preserve unrelated archive")?;
    let name = "xcalygraph-0.0.0-darwin-arm64";
    let shell = "source \"$1\"; package_release \"$2\" \"$3\" \"$4\"; publish_releases \"$2\" \"$5\" \"$3.tar.gz\"";
    let package = |destination: &Path, staging: &Path| {
        support::command("/bin/bash", &dir.0)
            .args(["-c", shell, "release-test"])
            .arg(&script)
            .arg(staging)
            .arg(name)
            .arg(BINARY)
            .arg(destination)
            .env("CARGO_NET_OFFLINE", "true")
            .env("HOME", dir.0.join("home"))
            .output()
    };
    let output = package(&dist, &stage)?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let archive = dist.join(format!("{name}.tar.gz"));
    let digest: String = sha2::Sha256::digest(fs::read(&archive)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        fs::read_to_string(dist.join("SHA256SUMS"))?,
        format!("{digest}  {name}.tar.gz\n")
    );
    assert_eq!(
        fs::read(dist.join("foreign.tar.gz"))?,
        b"preserve unrelated archive"
    );
    let listing = support::command("tar", &dir.0)
        .arg("-tzf")
        .arg(&archive)
        .output()?;
    assert!(listing.status.success());
    let listing = String::from_utf8(listing.stdout)?;
    for file in ["xcalygraph", "LICENSE", "THIRD-PARTY-LICENSES.txt"] {
        assert!(listing.contains(&format!("{name}/{file}")), "{listing}");
    }
    let extracted = dir.0.join("extracted");
    fs::create_dir(&extracted)?;
    let unpacked = support::command("tar", &dir.0)
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&extracted)
        .status()?;
    assert!(unpacked.success());
    let packaged = extracted.join(name);
    assert!(
        support::command("cmp", &dir.0)
            .arg("-s")
            .arg(BINARY)
            .arg(packaged.join("xcalygraph"))
            .status()?
            .success()
    );
    assert_eq!(fs::read(packaged.join("LICENSE"))?, b"fixture license\n");
    assert_eq!(
        fs::read(packaged.join("THIRD-PARTY-LICENSES.txt"))?,
        b"fixture notices"
    );
    let conflict_stage = dir.0.join("conflict-stage");
    let conflict_dist = dir.0.join("conflict-dist");
    fs::create_dir(&conflict_stage)?;
    fs::create_dir(&conflict_dist)?;
    fs::write(
        conflict_stage.join("THIRD-PARTY-LICENSES.txt"),
        b"fixture notices",
    )?;
    fs::write(
        conflict_dist.join(format!("{name}.tar.gz")),
        b"preserve conflicting archive",
    )?;
    let conflict = package(&conflict_dist, &conflict_stage)?;
    assert!(!conflict.status.success());
    assert_eq!(
        fs::read(conflict_dist.join(format!("{name}.tar.gz")))?,
        b"preserve conflicting archive"
    );
    assert!(!conflict_dist.join("SHA256SUMS").exists());
    let checksum_dist = dir.0.join("checksum-dist");
    fs::create_dir(&checksum_dist)?;
    fs::write(
        checksum_dist.join("SHA256SUMS"),
        b"preserve foreign checksums",
    )?;
    let check = support::command("/bin/bash", &dir.0)
        .args([
            "-c",
            "source \"$1\"; check_outputs \"$2\" \"$3\"",
            "release-check",
        ])
        .arg(&script)
        .arg(&checksum_dist)
        .arg(format!("{name}.tar.gz"))
        .output()?;
    assert!(!check.status.success());
    assert_eq!(
        fs::read(checksum_dist.join("SHA256SUMS"))?,
        b"preserve foreign checksums"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let linked_dist = dir.0.join("linked-dist");
        fs::create_dir(&linked_dist)?;
        let link = linked_dist.join(format!("{name}.tar.gz"));
        symlink(dir.0.join("missing-target"), &link)?;
        let check = support::command("/bin/bash", &dir.0)
            .args([
                "-c",
                "source \"$1\"; check_outputs \"$2\" \"$3\"",
                "release-check",
            ])
            .arg(&script)
            .arg(&linked_dist)
            .arg(format!("{name}.tar.gz"))
            .output()?;
        assert!(!check.status.success());
        assert!(fs::symlink_metadata(link)?.file_type().is_symlink());
    }
    Ok(())
}

#[test]
fn update_reports_a_failed_skill_phase_after_replacing_the_binary() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    // (name, delegate for `install`, make the skill directory read-only, expected exit)
    let cases = [
        ("success", format!("exec '{BINARY}' \"$@\""), false, Some(0)),
        (
            "read-only skill",
            format!("exec '{BINARY}' \"$@\""),
            true,
            Some(1),
        ),
        (
            "child failure",
            "echo 'xcalygraph: disk on fire' >&2; exit 7".to_owned(),
            false,
            Some(1),
        ),
    ];
    for (name, delegate, read_only, code) in cases {
        let dir = TempDir::new(&format!("update-skill-{}", name.replace(' ', "-")))?;
        let release = if name == "success" {
            dir.0.join("release\nwith-newline")
        } else {
            dir.0.join("release")
        };
        let latest = fake_release_at(&release, &delegate, "99.0.0")?;
        let exe = dir.0.join("bin/xcalygraph");
        fs::create_dir_all(dir.0.join("bin"))?;
        fs::copy(BINARY, &exe)?;
        let home = dir.0.join("home");
        let skills = home.join(".agents/skills");
        fs::create_dir_all(&skills)?;
        if read_only {
            fs::set_permissions(&skills, fs::Permissions::from_mode(0o555))?;
        }
        let output = support::command(&exe, &dir.0)
            .arg("update")
            .env("HOME", &home)
            .env(
                "XCALYGRAPH_UPDATE_URL",
                format!("file://{}", latest.display()),
            )
            .output()?;
        fs::set_permissions(&skills, fs::Permissions::from_mode(0o755))?;
        let stdout = String::from_utf8(output.stdout)?;
        let stderr = String::from_utf8(output.stderr)?;
        assert_eq!(output.status.code(), code, "{name}\n{stdout}\n{stderr}");
        // The binary is replaced in every case: the new script is in place.
        assert!(
            fs::read_to_string(&exe)?.contains("xcalygraph 99.0.0"),
            "{name}"
        );
        let skill = skills.join("xcalygraph/SKILL.md");
        if code == Some(0) {
            assert!(stdout.starts_with("updated "), "{name}\n{stdout}");
            assert!(skill.exists(), "{name}: skill missing");
        } else {
            assert!(stdout.is_empty(), "{name}\n{stdout}");
            assert!(
                stderr.contains("binary updated to 99.0.0 at ")
                    && stderr.contains("but the skill was not installed")
                    && stderr.contains("run 'xcalygraph install'"),
                "{name}\n{stderr}"
            );
            assert!(!skill.exists(), "{name}: skill unexpectedly written");
        }
        if name == "child failure" {
            assert!(stderr.contains("(disk on fire)"), "{stderr}");
        }
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> io::Result<()> {
    let status = support::command("git", root)
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

#[cfg(unix)]
struct GitProbe {
    path: std::ffi::OsString,
    real_git: PathBuf,
    trace: PathBuf,
    captured: PathBuf,
}

#[cfg(unix)]
fn install_git_probe(dir: &TempDir) -> Result<GitProbe, Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let path = std::env::var_os("PATH").ok_or("PATH unavailable")?;
    let real_git = std::env::split_paths(&path)
        .map(|directory| directory.join("git"))
        .find(|candidate| candidate.is_file())
        .ok_or("git unavailable")?;
    dir.write(
        "bin/git",
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TRACE_PATH"
case "$3" in
  diff|show) ;;
  cat-file) [ "$4" = "blob" ] || exec "$REAL_GIT" "$@" ;;
  *) exec "$REAL_GIT" "$@" ;;
esac
capture="${CAPTURE_PATH}.$$"
"$REAL_GIT" "$@" > "$capture"
status=$?
cat "$capture" >> "$CAPTURE_PATH"
cat "$capture"
rm -f "$capture"
exit "$status"
"#,
    )?;
    let shim = dir.0.join("bin/git");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755))?;
    let search_path = std::env::join_paths(
        std::iter::once(dir.0.join("bin")).chain(std::env::split_paths(&path)),
    )?;
    Ok(GitProbe {
        path: search_path,
        real_git,
        trace: dir.0.join("git-invocations"),
        captured: dir.0.join("git-content-output"),
    })
}

const DISPATCH_GO: &str = "package app

type Store interface {
	Write(data string) error
}

type FileStore struct{}

func (f *FileStore) Write(data string) error {
	return nil
}

func Send(s Store) error {
	return s.Write(\"x\")
}

func Direct() error {
	f := &FileStore{}
	return f.Write(\"y\")
}

func Both() error {
	_ = Send(&FileStore{})
	return Direct()
}

func Run() error {
	return Send(&FileStore{})
}

func Top() error {
	return Run()
}

func Ping() error {
	return Pong()
}

func Pong() error {
	return Ping()
}
";

#[test]
fn path_and_impact_keep_inferred_dispatch_inferred_on_every_hop() -> TestResult {
    let dir = TempDir::new("confidence")?;
    let root = dir.0.join("repo");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write("repo/app.go", DISPATCH_GO)?;
    let index = dir.0.join("index");
    let query = |args: &[&str]| -> Result<(Option<i32>, String), Box<dyn Error>> {
        let output = run_in(&root, &index, args)?;
        Ok((output.status.code(), String::from_utf8(output.stdout)?))
    };
    let (_, show) = query(&["show", "Send"])?;
    assert!(show.contains("> FileStore.Write :9?\n"), "{show}");
    // Directed path through interface dispatch: the dispatch hop is inferred in text and JSON.
    let (_, text) = query(&["path", "Send", "FileStore.Write"])?;
    assert_eq!(text, ". Send app.go:13\n> FileStore.Write :9?\n");
    let (_, json) = query(&["path", "Send", "FileStore.Write", "--json"])?;
    assert!(
        json.contains(
            "\"name\":\"FileStore.Write\",\"path\":\"app.go\",\"line\":9,\"conf\":\"inferred\""
        ),
        "{json}"
    );
    // Only the last hop is inferred; earlier exact calls stay exact.
    let (_, text) = query(&["path", "Top", "FileStore.Write"])?;
    assert_eq!(
        text,
        ". Top app.go:31\n> Run :27\n> Send :13\n> FileStore.Write :9?\n"
    );
    // An equally short all-exact route wins over an inferred one listed first in source.
    let (_, text) = query(&["path", "Both", "FileStore.Write"])?;
    assert_eq!(
        text,
        ". Both app.go:22\n> Direct :17\n> FileStore.Write :9\n"
    );
    // --exact drops the inferred dispatch and never falls back to an undirected path.
    let (code, text) = query(&["path", "Send", "FileStore.Write", "--exact"])?;
    assert_eq!((code, text.as_str()), (Some(4), ""));
    // Undirected fallback: the reversed hop is inferred.
    let (_, text) = query(&["path", "FileStore.Write", "Direct"])?;
    assert_eq!(text, ". FileStore.Write app.go:9\n< Direct :17?\n");
    // Cycles terminate and stay exact.
    let (_, text) = query(&["path", "Ping", "Pong"])?;
    assert_eq!(text, ". Ping app.go:35\n> Pong :39\n");
    // Impact: uncertainty propagates downstream; an exact chain at the same depth wins.
    let (_, text) = query(&["impact", "FileStore.Write"])?;
    for line in [
        "< 1 Send app.go:13?\n",
        "< 1 Direct :17\n",
        "< 2 Both :22\n",
        "< 2 Run :27?\n",
        "< 3 Top :31?\n",
    ] {
        assert!(text.contains(line), "{line}{text}");
    }
    let (_, json) = query(&["impact", "FileStore.Write", "--json"])?;
    assert!(
        json.contains(
            "\"name\":\"Top\",\"path\":\"app.go\",\"line\":31,\"n\":3,\"conf\":\"inferred\""
        ),
        "{json}"
    );
    let (_, exact) = query(&["impact", "FileStore.Write", "--exact"])?;
    assert!(
        !exact.contains('?') && !exact.contains("Run") && exact.contains("< 2 Both"),
        "{exact}"
    );
    // Several targets: dependents of exact targets are exact.
    let (_, text) = query(&["impact", "Send", "Direct"])?;
    for line in ["< 1 Both :22\n", "< 1 Run app.go:27\n", "< 2 Top :31\n"] {
        assert!(text.contains(line), "{line}{text}");
    }
    // Deterministic across runs.
    assert_eq!(
        query(&["impact", "FileStore.Write"])?.1,
        query(&["impact", "FileStore.Write"])?.1
    );
    Ok(())
}

#[test]
fn impact_stops_after_reachable_frontiers_are_exhausted() -> TestResult {
    let dir = TempDir::new("impact-large-depth")?;
    let root = dir.0.join("repo");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write("repo/app.go", DISPATCH_GO)?;
    let index = dir.0.join("index");

    let bounded = run_in(
        &root,
        &index,
        &["impact", "FileStore.Write", "--depth", "3"],
    )?;
    assert_eq!(bounded.status.code(), Some(0));
    let bounded = String::from_utf8(bounded.stdout)?;
    assert!(bounded.contains("< 1 Send app.go:13?\n"), "{bounded}");
    assert!(bounded.contains("< 3 Top :31?\n"), "{bounded}");
    assert_impact_large_depth_matches(
        &root,
        &index,
        &["impact", "FileStore.Write", "--depth", "1000000000"],
        &bounded,
    )?;

    let cycle = run_in(&root, &index, &["impact", "Pong", "--depth", "3"])?;
    assert_eq!(cycle.status.code(), Some(0));
    let cycle = String::from_utf8(cycle.stdout)?;
    assert!(cycle.contains("< 1 Ping app.go:35\n"), "{cycle}");
    assert_impact_large_depth_matches(
        &root,
        &index,
        &["impact", "Pong", "--depth", "1000000000"],
        &cycle,
    )?;
    Ok(())
}

#[test]
fn show_indirect_test_confidence_includes_the_caller_edge() -> TestResult {
    let dir = TempDir::new("show-test-confidence")?;
    let root = dir.0.join("repo");
    let index = dir.0.join("index");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write("repo/app.go", DISPATCH_GO)?;
    dir.write(
        "repo/app_test.go",
        "package app\n\nimport \"testing\"\n\nfunc TestSend(t *testing.T) {\n\tSend(&FileStore{})\n}\n\nfunc TestDirect(t *testing.T) {\n\tDirect()\n}\n\nfunc TestBoth(t *testing.T) {\n\tSend(&FileStore{})\n\tDirect()\n}\n",
    )?;
    git(&root, &["init", "-q"])?;

    for json in [false, true] {
        let mut ordinary = vec!["show", "FileStore.Write"];
        let mut exact = vec!["show", "FileStore.Write", "--exact"];
        if json {
            ordinary.push("--json");
            exact.push("--json");
        }
        let ordinary = run_in(&root, &index, &ordinary)?;
        assert_eq!(ordinary.status.code(), Some(0));
        let ordinary = String::from_utf8(ordinary.stdout)?;
        if json {
            assert!(
                ordinary.lines().any(|line| {
                    line.contains("TestSend")
                        && line.contains("app_test.go")
                        && line.contains("\"conf\":\"inferred\"")
                }),
                "{ordinary}"
            );
            assert!(
                ordinary.lines().any(|line| {
                    line.contains("TestDirect")
                        && line.contains("app_test.go")
                        && line.contains("\"conf\":\"exact\"")
                }),
                "{ordinary}"
            );
            assert!(
                ordinary.lines().any(|line| {
                    line.contains("TestBoth") && line.contains("\"conf\":\"exact\"")
                }),
                "{ordinary}"
            );
        } else {
            assert!(
                ordinary
                    .lines()
                    .any(|line| { line.contains("TestSend") && line.contains("app_test.go:6?") }),
                "{ordinary}"
            );
            assert!(
                ordinary.lines().any(|line| {
                    line.contains("TestDirect") && line.contains(":10") && !line.ends_with('?')
                }),
                "{ordinary}"
            );
            assert!(
                ordinary
                    .lines()
                    .any(|line| { line.contains("TestBoth") && !line.ends_with('?') }),
                "{ordinary}"
            );
        }

        let exact = run_in(&root, &index, &exact)?;
        assert_eq!(exact.status.code(), Some(0));
        let exact = String::from_utf8(exact.stdout)?;
        assert!(exact.contains("TestDirect"), "{exact}");
        assert!(exact.contains("TestBoth"), "{exact}");
        assert!(!exact.contains("TestSend"), "{exact}");
    }
    Ok(())
}

#[test]
fn impact_diff_exact_drops_inferred_removed_test_chains() -> TestResult {
    let dir = TempDir::new("impact-removed-test-confidence")?;
    let root = dir.0.join("repo");
    let index = dir.0.join("index");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write(
        "repo/a/removed.go",
        "package a\n\nfunc Removed() int { return 1 }\n",
    )?;
    dir.write(
        "repo/a/change.go",
        "package a\n\ntype Changeable interface { Changed() int }\n\ntype FileStore struct{}\n\nfunc (*FileStore) Changed() int { return 1 }\n\nfunc Send(store Changeable) int { return store.Changed() }\n\nfunc Direct() int {\n\tstore := &FileStore{}\n\treturn store.Changed()\n}\n",
    )?;
    dir.write(
        "repo/b/use.go",
        "package b\n\nimport \"example.com/app/a\"\n\nfunc UsesRemoved() int { return a.Removed() }\n",
    )?;
    dir.write(
        "repo/b/use_test.go",
        "package b\n\nimport \"testing\"\n\nfunc TestUsesRemoved(t *testing.T) { _ = UsesRemoved() }\n",
    )?;
    dir.write(
        "repo/c/use.go",
        "package c\n\nimport \"example.com/app/a\"\n\nfunc UsesChanged() int {\n\tstore := &a.FileStore{}\n\t_ = a.Send(store)\n\treturn a.Direct()\n}\n",
    )?;
    dir.write(
        "repo/c/use_test.go",
        "package c\n\nimport \"testing\"\n\nfunc TestUsesChanged(t *testing.T) { _ = UsesChanged() }\n",
    )?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    dir.write(
        "repo/a/change.go",
        "package a\n\ntype Changeable interface { Changed() int }\n\ntype FileStore struct{}\n\nfunc (*FileStore) Changed() int { return 2 }\n\nfunc Send(store Changeable) int { return store.Changed() }\n\nfunc Direct() int {\n\tstore := &FileStore{}\n\treturn store.Changed()\n}\n",
    )?;
    fs::remove_file(root.join("a/removed.go"))?;

    for json in [false, true] {
        let mut ordinary = vec!["impact", "--diff"];
        let mut exact = vec!["impact", "--diff", "--exact"];
        if json {
            ordinary.push("--json");
            exact.push("--json");
        }
        let exact = run_in(&root, &index, &exact)?;
        assert_eq!(exact.status.code(), Some(0));
        let exact = String::from_utf8(exact.stdout)?;
        assert!(exact.contains("TestUsesChanged"), "{exact}");
        assert!(exact.contains("go test ./c"), "{exact}");
        assert!(!exact.contains("b.UsesRemoved"), "{exact}");
        assert!(!exact.contains("TestUsesRemoved"), "{exact}");
        assert!(!exact.contains("go test ./b"), "{exact}");

        let ordinary = run_in(&root, &index, &ordinary)?;
        assert_eq!(ordinary.status.code(), Some(0));
        let ordinary = String::from_utf8(ordinary.stdout)?;
        assert!(ordinary.contains("Removed"), "{ordinary}");
        assert!(ordinary.contains("UsesRemoved"), "{ordinary}");
        assert!(ordinary.contains("go test ./b"), "{ordinary}");
        assert!(ordinary.contains("TestUsesChanged"), "{ordinary}");
        assert!(ordinary.contains("go test ./c"), "{ordinary}");
        if json {
            assert!(
                ordinary.lines().any(|line| {
                    line.contains("TestUsesRemoved") && line.contains("\"conf\":\"inferred\"")
                }),
                "{ordinary}"
            );
            assert!(
                ordinary.lines().any(|line| {
                    line.contains("TestUsesChanged") && line.contains("\"conf\":\"exact\"")
                }),
                "{ordinary}"
            );
        } else {
            assert!(
                ordinary
                    .lines()
                    .any(|line| { line.contains("TestUsesRemoved") && line.ends_with('?') }),
                "{ordinary}"
            );
            assert!(
                ordinary
                    .lines()
                    .any(|line| { line.contains("TestUsesChanged") && !line.ends_with('?') }),
                "{ordinary}"
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn literal_indexed_paths_precede_name_syntax_in_shared_queries() -> TestResult {
    let dir = TempDir::new("literal-name-precedence")?;
    let root = dir.0.join("repo");
    for (path, source) in [
        ("  spaced.rs", "pub fn leading_literal() {}\n"),
        ("spaced.rs", "pub fn leading_decoy() {}\n"),
        ("trailing.rs ", "literal file\n"),
        ("trailing.rs", "pub fn trailing_decoy() {}\n"),
        ("prefix.rs", "pub fn copy() {}\n"),
        ("prefix.rs::copy", "literal file\n"),
        ("line.rs", "pub fn line_symbol() {}\n"),
        ("line.rs:1", "literal file\n"),
        ("control\t.rs", "literal file\n"),
        ("control\u{0001}.rs", "literal file\n"),
        ("newline\n.rs", "literal file\n"),
        ("雪.rs", "literal file\n"),
        ("syntax.rs", "pub fn first() {}\npub fn second() {}\n"),
    ] {
        dir.write(&format!("repo/{path}"), source)?;
    }
    let index = dir.0.join("index");
    let indexed = run_in(&root, &index, &["index"])?;
    assert_eq!(
        indexed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    let query = |args: &[&str]| -> io::Result<Output> { run_in(&root, &index, args) };

    for name in [
        "  spaced.rs",
        "trailing.rs ",
        "prefix.rs::copy",
        "line.rs:1",
        "control\t.rs",
        "control\u{0001}.rs",
        "newline\n.rs",
        "雪.rs",
    ] {
        let output = query(&["show", name, "--read-only"])?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "show {name:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        let rendered_name = match name {
            "control\t.rs" => "control\\t.rs",
            "control\u{0001}.rs" => "control\\u0001.rs",
            "newline\n.rs" => "newline\\n.rs",
            _ => name,
        };
        assert!(
            text.starts_with(&format!("{rendered_name} ")),
            "show {name:?}: {text:?}"
        );
    }
    for name in ["spaced.rs", "trailing.rs"] {
        let output = query(&["show", name, "--read-only"])?;
        let text = String::from_utf8(output.stdout)?;
        assert!(
            text.starts_with(&format!("{name} ")),
            "show {name:?}: {text:?}"
        );
    }

    for name in [
        "  spaced.rs",
        "trailing.rs ",
        "prefix.rs::copy",
        "line.rs:1",
    ] {
        let output = query(&["impact", name, "--json", "--read-only"])?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "impact {name:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json = String::from_utf8(output.stdout)?;
        assert!(
            json.contains(&format!("\"name\":\"{name}\"")),
            "impact {name:?}: {json}"
        );

        let output = query(&["path", name, name, "--json", "--read-only"])?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "path {name:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json = String::from_utf8(output.stdout)?;
        assert!(
            json.contains(&format!("\"name\":\"{name}\"")),
            "path {name:?}: {json}"
        );
    }

    for (name, symbol) in [
        ("crate::copy", "crate::copy"),
        ("crate.copy", "crate::copy"),
        ("syntax.rs::second", "crate::second"),
        ("syntax.rs:2", "crate::second"),
    ] {
        let output = query(&["show", name, "--read-only"])?;
        assert_eq!(output.status.code(), Some(0), "show {name:?}");
        let text = String::from_utf8(output.stdout)?;
        let short = symbol.trim_start_matches("crate::");
        assert!(
            text.contains(&format!("fn {short}")),
            "show {name:?}: {text}"
        );

        let output = query(&["impact", name, "--json", "--read-only"])?;
        assert_eq!(output.status.code(), Some(0), "impact {name:?}");
        let json = String::from_utf8(output.stdout)?;
        assert!(
            json.contains(&format!("\"name\":\"{symbol}\"")),
            "impact {name:?}: {json}"
        );
    }
    for (from, to, symbol) in [
        ("crate::copy", "crate.copy", "crate::copy"),
        ("syntax.rs::second", "syntax.rs:2", "crate::second"),
    ] {
        let output = query(&["path", from, to, "--json", "--read-only"])?;
        assert_eq!(output.status.code(), Some(0), "path {from:?} to {to:?}");
        let json = String::from_utf8(output.stdout)?;
        assert!(
            json.contains(&format!("\"name\":\"{symbol}\"")),
            "path {from:?} to {to:?}: {json}"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn query_text_escapes_control_characters_in_paths_across_commands() -> TestResult {
    let dir = TempDir::new("escaped-query-paths")?;
    let root = dir.0.join("repo");
    let name = "control\\name\trow\rreturn\nend.go";
    dir.write(
        &format!("repo/{name}"),
        "package main\nfunc escaped_path_symbol() {}\nfunc main() {}\n",
    )?;
    let index = dir.0.join("index");
    let indexed = run_in(&root, &index, &["index"])?;
    assert_eq!(
        indexed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );

    let escaped = "control\\\\name\\trow\\rreturn\\nend.go";
    let cases: [(&str, &[&str], u32); 7] = [
        ("find", &["find", "escaped_path_symbol"], 2),
        ("ctx", &["ctx", "escaped_path_symbol"], 2),
        ("show", &["show", "escaped_path_symbol"], 2),
        (
            "path",
            &["path", "escaped_path_symbol", "escaped_path_symbol"],
            2,
        ),
        ("impact", &["impact", "escaped_path_symbol"], 2),
        ("audit", &["audit"], 2),
        ("map", &["map"], 3),
    ];
    for (command, args, line) in cases {
        let expected_text = match command {
            "find" | "ctx" => format!("{escaped}\n  {line} func escaped_path_symbol()"),
            "show" => format!("{escaped}:{line}-{line} fn escaped_path_symbol"),
            "path" => format!(". escaped_path_symbol {escaped}:{line}"),
            "impact" => format!("~ escaped_path_symbol {escaped}:{line}"),
            "audit" => format!("escaped_path_symbol {escaped}:{line}"),
            "map" => format!("main {escaped}:{line}"),
            _ => return Err(format!("unexpected query command: {command}").into()),
        };
        let mut text_args = args.to_vec();
        text_args.push("--read-only");
        let output = run_in(&root, &index, &text_args)?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        assert!(
            text.contains(&expected_text),
            "{command}: expected {expected_text:?}, got {text:?}"
        );
        assert!(text.ends_with('\n'), "{command}: {text:?}");
        assert!(!text.contains(name), "{command}: raw path in {text:?}");
        assert!(
            text.chars()
                .filter(|&character| character != '\n')
                .all(|character| !character.is_control()),
            "{command}: control character in {text:?}"
        );
        let repeated = run_in(&root, &index, &text_args)?;
        assert_eq!(text, String::from_utf8(repeated.stdout)?, "{command}");

        let mut json_args = args.to_vec();
        json_args.extend(["--json", "--read-only"]);
        let output = run_in(&root, &index, &json_args)?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{command} --json: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json = String::from_utf8(output.stdout)?;
        assert_json_lines_parse(&json, &dir.0)?;
        assert!(
            json.contains(&format!("\"path\":{}", json_string(name))),
            "{command} --json: {json:?}"
        );
        assert!(
            json.contains(&format!("\"line\":{line}")),
            "{command} --json: {json:?}"
        );
        assert!(json.ends_with('\n'), "{command} --json: {json:?}");
        assert!(
            !json.contains(name),
            "{command} --json: raw path in {json:?}"
        );
    }
    Ok(())
}

/// Arguments, exit code, required substrings, forbidden substrings.
type FilterCase<'a> = (&'a [&'a str], i32, &'a [&'a str], &'a [&'a str]);

#[test]
fn every_query_stage_obeys_the_shared_filters() -> TestResult {
    let dir = TempDir::new("filters")?;
    let root = dir.0.join("repo");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write(
        "repo/a/a.go",
        "package a\n\nimport \"example.com/app/b\"\n\nfunc A() {\n\tb.B()\n}\n",
    )?;
    dir.write(
        "repo/b/b.go",
        "package b\n\nimport \"example.com/app/c\"\n\nfunc B() {\n\tc.C()\n}\n",
    )?;
    dir.write(
        "repo/c/c.go",
        "package c\n\nimport \"os\"\n\nfunc C() string {\n\treturn os.Getenv(\"MODE\")\n}\n",
    )?;
    dir.write(
        "repo/c/c_test.go",
        "package c\n\nimport \"testing\"\n\nfunc TestC(t *testing.T) {\n\tC()\n}\n",
    )?;
    git(&root, &["init", "-q"])?;
    let index = dir.0.join("index");
    let cases: &[FilterCase] = &[
        (&["path", "a.A", "c.C"], 0, &["> B b/b.go:5\n"], &[]),
        (
            &["path", "a.A", "c.C", "-p", "a/", "-p", "c/"],
            4,
            &[],
            &["b.B"],
        ),
        (&["show", "c/c.go:6"], 0, &["fn C"], &[]),
        (&["show", "c/c.go:6", "-p", "a/"], 4, &[], &["c.C"]),
        (&["show", "c/c.go", "-p", "a/"], 4, &[], &["c.C"]),
        (&["show", "c/c.go::C", "-p", "a/"], 4, &[], &["c.C"]),
        (
            &["show", "c/c.go", "-k", "fn"],
            0,
            &["5 func C() string\n"],
            &[],
        ),
        (&["show", "c.C", "-k", "struct"], 4, &[], &[]),
        (&["show", "c.C", "-l", "python"], 4, &[], &[]),
        (&["show", "c.C"], 0, &["t TestC", "< B"], &[]),
        (&["show", "c.C", "--no-tests"], 0, &["< B"], &["TestC"]),
        (
            &["impact", "c.C"],
            0,
            &["< 1 B", "< 2 A", "t TestC", "r go test ./c"],
            &[],
        ),
        (
            &["impact", "c.C", "--no-tests"],
            0,
            &["< 1 B", "< 2 A"],
            &["TestC", "go test"],
        ),
        (
            &["impact", "c.C", "--no-tests", "--json"],
            0,
            &["\"name\":\"b.B\""],
            &["TestC", "go test"],
        ),
        (&["impact", "c.C", "-p", "a/"], 4, &[], &[]),
        (
            &["impact", "c.C", "-p", "c/", "-p", "a/"],
            0,
            &["< 2 A", "t TestC"],
            &["b.B"],
        ),
        (&["find", "C", "-p", "a/"], 4, &[], &["c/c.go"]),
        (
            &["map", "-p", "a/"],
            0,
            &["a.A"],
            &["b.B", "c.C", " b\n", " c\n", "MODE"],
        ),
        (&["map"], 0, &["MODE"], &[]),
    ];
    for (args, code, required, forbidden) in cases {
        let output = run_in(&root, &index, args)?;
        let text = String::from_utf8(output.stdout)?;
        assert_eq!(output.status.code(), Some(*code), "{args:?}\n{text}");
        for needle in *required {
            assert!(text.contains(needle), "{args:?} lacks {needle:?}\n{text}");
        }
        for needle in *forbidden {
            assert!(
                !text.contains(needle),
                "{args:?} exposes {needle:?}\n{text}"
            );
        }
    }
    Ok(())
}

#[test]
fn filtered_cycles_are_recomputed_for_map_and_audit() -> TestResult {
    let dir = TempDir::new("filtered-cycles")?;
    let root = dir.0.join("repo");
    let index = dir.0.join("index");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    for (path, source) in [
        (
            "repo/a/a.go",
            "package a\n\nimport \"example.com/app/b\"\n\nfunc A() { b.B() }\n",
        ),
        (
            "repo/b/b.go",
            "package b\n\nimport \"example.com/app/c\"\n\nfunc B() { c.C() }\n",
        ),
        (
            "repo/c/c.go",
            "package c\n\nimport \"example.com/app/a\"\n\nfunc C() { a.A() }\n",
        ),
        (
            "repo/d/d.go",
            "package d\n\nimport \"example.com/app/e\"\n\nfunc D() { e.E() }\n",
        ),
        (
            "repo/e/e.go",
            "package e\n\nimport \"example.com/app/d\"\n\nfunc E() { d.D() }\n",
        ),
        (
            "repo/f/f.go",
            "package f\n\nimport \"example.com/app/a\"\n\nfunc F() { a.A() }\n",
        ),
        (
            "repo/g/g.go",
            "package g\n\nimport \"example.com/app/f\"\n\nfunc G() { f.F() }\n",
        ),
    ] {
        dir.write(path, source)?;
    }
    git(&root, &["init", "-q"])?;

    for command in ["map", "audit"] {
        for json in [false, true] {
            let mut complete = vec![command];
            let mut filtered = vec![command, "-p", "a/", "-p", "c/", "-p", "d/", "-p", "e/"];
            if json {
                complete.push("--json");
                filtered.push("--json");
            }
            let complete = run_in(&root, &index, &complete)?;
            assert_eq!(complete.status.code(), Some(0), "{command}: complete graph");
            let complete = String::from_utf8(complete.stdout)?;
            assert!(complete.contains("a <-> b <-> c"), "{command}: {complete}");
            assert!(complete.contains("d <-> e"), "{command}: {complete}");
            if json && command == "map" {
                assert!(
                    complete.contains("\"t\":\"cycle\",\"name\":\"a <-> b <-> c\",\"n\":3,\"members\":[\"a\",\"b\",\"c\"]"),
                    "{complete}"
                );
                assert!(
                    complete.lines().any(|line| {
                        line.contains("\"t\":\"layer\"")
                            && line.contains("\"members\":[")
                            && line.contains("\"g\"")
                    }),
                    "{complete}"
                );
            }
            if json && command == "audit" {
                assert!(
                    complete.contains("\"rel\":\"cycles\"")
                        && complete.contains("\"members\":[\"a\",\"b\",\"c\"]"),
                    "{complete}"
                );
            }

            let filtered = run_in(&root, &index, &filtered)?;
            assert_eq!(filtered.status.code(), Some(0), "{command}: filtered graph");
            let filtered = String::from_utf8(filtered.stdout)?;
            assert!(!filtered.contains("a <-> c"), "{command}: {filtered}");
            assert!(filtered.contains("d <-> e"), "{command}: {filtered}");
        }
    }
    Ok(())
}

#[test]
fn show_and_audit_results_obey_shared_filters() -> TestResult {
    let dir = TempDir::new("remaining-filters-show-audit")?;
    let root = dir.0.join("repo");
    let index = dir.0.join("index");
    dir.write("repo/.gitignore", "!.env*\n")?;
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write("repo/a/.env.example", "UNUSED_FLAG=1\n")?;
    dir.write(
        "repo/a/a.go",
        "package a\n\nimport (\n\t\"os\"\n\t\"example.com/app/b\"\n)\n\ntype Box struct { Value string }\n\nfunc Read() { _ = os.Getenv(\"A_MISSING\") }\n\nfunc Call() { b.Caller() }\n",
    )?;
    dir.write(
        "repo/b/b.go",
        "package b\n\nimport (\n\t\"os\"\n\t\"example.com/app/a\"\n)\n\nfunc Caller() {\n\ta.Read()\n\t_ = os.Getenv(\"B_MISSING\")\n\t_ = \"UNUSED_FLAG\"\n}\n",
    )?;
    dir.write(
        "repo/a/a_test.go",
        "package a\n\nimport (\n\t\"os\"\n\t\"testing\"\n\t\"example.com/app/b\"\n)\n\nfunc TestIndirect(t *testing.T) {\n\tb.Caller()\n\t_ = os.Getenv(\"TEST_MISSING\")\n}\n",
    )?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;

    for (args, required, forbidden) in [
        (
            &["show", "a/a.go", "-k", "fn"][..],
            &["Read", "Call"][..],
            &["Box"][..],
        ),
        (
            &["show", "a/a.go", "-k", "fn", "--json"][..],
            &["Read", "Call"][..],
            &["Box"][..],
        ),
        (
            &["show", "a/a.go", "-k", "struct"][..],
            &["Box"][..],
            &["Read", "Call"][..],
        ),
        (
            &["show", "a/a.go", "-k", "struct", "--json"][..],
            &["\"name\":\"a.Box\""][..],
            &["\"name\":\"a.Read\"", "\"name\":\"a.Call\""][..],
        ),
    ] {
        let output = run_in(&root, &index, args)?;
        let text = String::from_utf8(output.stdout)?;
        if output.status.code() != Some(0)
            || required.iter().any(|needle| !text.contains(needle))
            || forbidden.iter().any(|needle| text.contains(needle))
        {
            return Err(format!("{args:?}: status {:?}\n{text}", output.status.code()).into());
        }
    }

    for json in [false, true] {
        let mut unfiltered = vec!["show", "a.Read"];
        let mut path_filtered = vec!["show", "a.Read", "-p", "a/"];
        let mut audit_path = vec!["audit", "-p", "a/"];
        let mut audit_language = vec!["audit", "-l", "python"];
        let mut audit_no_tests = vec!["audit", "--no-tests"];
        let mut audit_kind = vec!["audit", "-k", "struct"];
        if json {
            unfiltered.push("--json");
            path_filtered.push("--json");
            audit_path.push("--json");
            audit_language.push("--json");
            audit_no_tests.push("--json");
            audit_kind.push("--json");
        }
        let show_unfiltered = run_in(&root, &index, &unfiltered)?;
        let show_unfiltered = String::from_utf8(show_unfiltered.stdout)?;
        assert!(
            show_unfiltered.contains("TestIndirect"),
            "{show_unfiltered}"
        );
        let show_path_filtered = run_in(&root, &index, &path_filtered)?;
        let show_path_filtered = String::from_utf8(show_path_filtered.stdout)?;
        assert!(
            !show_path_filtered.contains("TestIndirect"),
            "{show_path_filtered}"
        );

        let audit_path = run_in(&root, &index, &audit_path)?;
        let audit_path = String::from_utf8(audit_path.stdout)?;
        assert!(audit_path.contains("A_MISSING"), "{audit_path}");
        assert!(!audit_path.contains("B_MISSING"), "{audit_path}");
        assert!(!audit_path.contains("cycles"), "{audit_path}");
        if json {
            assert!(
                audit_path.contains("\"name\":\"UNUSED_FLAG\"")
                    && audit_path.contains("\"kind\":\"defined\",\"status\":\"unread\""),
                "{audit_path}"
            );
        } else {
            assert!(audit_path.contains("defined UNUSED_FLAG"), "{audit_path}");
        }

        let audit_language = run_in(&root, &index, &audit_language)?;
        let audit_language = String::from_utf8(audit_language.stdout)?;
        assert!(!audit_language.contains("cycles"), "{audit_language}");
        assert!(!audit_language.contains("A_MISSING"), "{audit_language}");
        assert!(!audit_language.contains("B_MISSING"), "{audit_language}");

        let audit_no_tests = run_in(&root, &index, &audit_no_tests)?;
        let audit_no_tests = String::from_utf8(audit_no_tests.stdout)?;
        assert!(audit_no_tests.contains("A_MISSING"), "{audit_no_tests}");
        assert!(!audit_no_tests.contains("TEST_MISSING"), "{audit_no_tests}");

        let audit_kind = run_in(&root, &index, &audit_kind)?;
        let audit_kind = String::from_utf8(audit_kind.stdout)?;
        assert!(audit_kind.contains("cycles"), "{audit_kind}");
        assert!(!audit_kind.contains("A_MISSING"), "{audit_kind}");
        assert!(!audit_kind.contains("B_MISSING"), "{audit_kind}");
    }
    Ok(())
}

#[test]
fn impact_diff_respects_symbol_file_and_test_filters() -> TestResult {
    let dir = TempDir::new("remaining-filters-impact-diff")?;
    let root = dir.0.join("repo");
    let index = dir.0.join("index");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write(
        "repo/a/a.go",
        "package a\n\ntype Box struct { A int }\n\ntype RetiredType struct {}\n\nfunc Retired() {}\n\nfunc Changed() {}\n",
    )?;
    dir.write(
        "repo/a/obsolete.go",
        "package a\n\ntype ObsoleteType struct {}\n\nfunc Obsolete() {}\n",
    )?;
    dir.write(
        "repo/a/a_test.go",
        "package a\n\nimport \"testing\"\n\nfunc TestChanged(t *testing.T) { Changed() }\n",
    )?;
    dir.write(
        "repo/b/b.go",
        "package b\n\nimport \"example.com/app/a\"\n\nfunc Caller() { a.Retired() }\n",
    )?;
    dir.write(
        "repo/b/b_test.go",
        "package b\n\nimport \"testing\"\n\nfunc TestCaller(t *testing.T) { Caller() }\n",
    )?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;

    dir.write(
        "repo/a/a.go",
        "package a\n\ntype Box struct { A int; B int }\n\nfunc Changed() { _ = 1 }\n",
    )?;
    dir.write(
        "repo/a/new.go",
        "package a\n\ntype AddedType struct{}\n\nfunc AddedFunction() {}\n",
    )?;
    dir.write(
        "repo/a/new_test.go",
        "package a\n\nimport \"testing\"\n\nfunc TestAdded(t *testing.T) { AddedFunction(); _ = AddedType{} }\n",
    )?;
    fs::remove_file(root.join("a/obsolete.go"))?;
    fs::remove_file(root.join("b/b_test.go"))?;

    for json in [false, true] {
        let mut args = vec!["impact", "--diff"];
        if json {
            args.push("--json");
        }
        let cases: &[(&[&str], &[&str], &[&str])] = &[
            (
                &[],
                &[
                    "Changed",
                    "Retired",
                    "RetiredType",
                    "Obsolete",
                    "ObsoleteType",
                    "TestAdded",
                    "TestCaller",
                ],
                &[],
            ),
            (
                &["-k", "fn"],
                &["Changed", "Retired", "Obsolete", "TestAdded", "new 1"],
                &["Box", "RetiredType", "ObsoleteType"],
            ),
            (
                &["-k", "struct"],
                &["Box", "RetiredType", "ObsoleteType", "new 1"],
                &[],
            ),
            (
                &["-l", "python"],
                &[],
                &[
                    "Changed",
                    "Retired",
                    "RetiredType",
                    "Obsolete",
                    "ObsoleteType",
                    "Box",
                ],
            ),
            (
                &["-p", "a/"],
                &["Retired", "Obsolete", "TestAdded"],
                &["Caller", "TestCaller"],
            ),
            (
                &["-p", "b/"],
                &["TestCaller"],
                &["Retired", "Obsolete", "Changed", "TestAdded"],
            ),
            (
                &["--no-tests"],
                &["Changed", "Retired", "Obsolete"],
                &["TestChanged", "TestAdded", "TestCaller", "go test"],
            ),
        ];
        for (filters, required, forbidden) in cases {
            let mut query = args.clone();
            query.extend_from_slice(filters);
            let output = run_in(&root, &index, &query)?;
            let text = String::from_utf8(output.stdout)?;
            if output.status.code() != Some(0)
                || required.iter().any(|needle| !text.contains(needle))
                || forbidden.iter().any(|needle| text.contains(needle))
                || (filters == &["-k", "struct"]
                    && if json {
                        [
                            "\"name\":\"Changed\"",
                            "\"name\":\"Retired\"",
                            "\"name\":\"Obsolete\"",
                            "\"name\":\"TestAdded\"",
                        ]
                        .iter()
                        .any(|needle| text.contains(needle))
                    } else {
                        ["~ Changed ", "~ Retired ", "~ Obsolete ", "~ TestAdded "]
                            .iter()
                            .any(|needle| text.contains(needle))
                    })
            {
                return Err(format!(
                    "json={json} filters={filters:?}: status {:?}\n{text}",
                    output.status.code()
                )
                .into());
            }
        }
    }
    Ok(())
}

/// The single derived image below an index base.
fn derived_image(base: &Path) -> io::Result<Vec<u8>> {
    let mut stores = fs::read_dir(base)?.collect::<io::Result<Vec<_>>>()?;
    match stores.pop() {
        Some(store) if stores.is_empty() => fs::read(store.path().join("derived")),
        _ => Err(io::Error::other("expected exactly one index store")),
    }
}

/// Writes a file whose mtime lies in the past, so a later scan trusts its stat record.
fn write_aged(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    let past = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800);
    fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(past)
}

fn index_snapshot(base: &Path) -> io::Result<Vec<(PathBuf, Vec<u8>, std::time::SystemTime)>> {
    let mut directories = vec![base.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                directories.push(entry.path());
            } else {
                files.push((entry.path(), fs::read(entry.path())?, metadata.modified()?));
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

#[test]
fn preserved_mtime_edits_and_replacements_invalidate_the_index() -> TestResult {
    const BEFORE: &str = "package main\nfunc Alpha() {}\n";
    const AFTER: &str = "package main\nfunc Bravo() {}\n";
    assert_eq!(BEFORE.len(), AFTER.len());
    for replacement in [true, false] {
        let dir = TempDir::new(if replacement {
            "same-mtime-replace"
        } else {
            "same-mtime-edit"
        })?;
        let root = dir.0.join("repo");
        let source = root.join("main.go");
        let index = dir.0.join("index");
        write_aged(&source, BEFORE)?;
        let initial = run_in(&root, &index, &["show", "Alpha"])?;
        assert_eq!(initial.status.code(), Some(0));
        let original_mtime = fs::metadata(&source)?.modified()?;
        if replacement {
            let candidate = root.join("replacement.tmp");
            write_aged(&candidate, AFTER)?;
            fs::rename(candidate, &source)?;
        } else {
            fs::write(&source, AFTER)?;
            fs::File::options()
                .write(true)
                .open(&source)?
                .set_modified(original_mtime)?;
        }
        assert_eq!(fs::metadata(&source)?.modified()?, original_mtime);
        assert_eq!(fs::metadata(&source)?.len(), BEFORE.len() as u64);
        let before = index_snapshot(&index)?;
        let read_only = run_in(&root, &index, &["show", "Bravo", "--read-only"])?;
        assert_eq!(read_only.status.code(), Some(5), "{replacement}");
        assert_eq!(
            index_snapshot(&index)?,
            before,
            "read-only wrote to the index"
        );
        let current = run_in(&root, &index, &["show", "Bravo"])?;
        assert_eq!(
            current.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&current.stderr)
        );
        assert!(String::from_utf8(current.stdout)?.contains("Bravo"));
        assert_eq!(
            run_in(&root, &index, &["show", "Alpha"])?.status.code(),
            Some(4)
        );
        let fresh = dir.0.join("fresh");
        assert!(run_in(&root, &fresh, &["index"])?.status.success());
        assert_eq!(
            derived_image(&index)?,
            derived_image(&fresh)?,
            "incremental differs from fresh"
        );
        let before = index_snapshot(&index)?;
        fs::File::options()
            .write(true)
            .open(&source)?
            .set_modified(original_mtime)?;
        let same = run_in(&root, &index, &["show", "Bravo", "--read-only"])?;
        assert_eq!(
            same.status.code(),
            Some(0),
            "metadata-only change should stay readable"
        );
        assert_eq!(index_snapshot(&index)?, before, "read-only wrote metadata");
        let meta = index_for(&index, "repo")?.join("meta");
        for legacy_format in [8_u32, 7] {
            let mut old_format = fs::read(&meta)?;
            old_format[8..12].copy_from_slice(&legacy_format.to_le_bytes());
            fs::write(&meta, old_format)?;
            let before_upgrade = index_snapshot(&index)?;
            assert_eq!(
                run_in(&root, &index, &["show", "Bravo", "--read-only"])?
                    .status
                    .code(),
                Some(5),
                "format {legacy_format}"
            );
            assert_eq!(
                index_snapshot(&index)?,
                before_upgrade,
                "read-only changed format {legacy_format} index"
            );
            let upgraded = run_in(&root, &index, &["index"])?;
            assert_eq!(upgraded.status.code(), Some(0));
            assert_eq!(
                String::from_utf8(upgraded.stdout)?,
                "files 1 changed 1 deleted 0\n"
            );
            assert_eq!(derived_image(&index)?, derived_image(&fresh)?);
        }
    }
    verified_previous_format_cleanup()
}

#[test]
fn incremental_updates_equal_a_fresh_build() -> TestResult {
    const HEADER: &str = "int area(int w);\n";
    const CPP: &str = "int area(int w) { return w; }\n";
    type Step = fn(&Path) -> io::Result<()>;
    // (name, files before, change, expected header type after)
    let scenarios: [(&str, bool, Step, &str); 5] = [
        (
            "add sibling",
            false,
            |root| write_aged(&root.join("widget.cpp"), CPP),
            "cpp",
        ),
        (
            "remove sibling",
            true,
            |root| fs::remove_file(root.join("widget.cpp")),
            "c",
        ),
        (
            "rename sibling away",
            true,
            |root| fs::rename(root.join("widget.cpp"), root.join("other.cpp")),
            "c",
        ),
        (
            "rename to another C++ suffix",
            true,
            |root| fs::rename(root.join("widget.cpp"), root.join("widget.cc")),
            "cpp",
        ),
        (
            "ignore sibling",
            true,
            |root| write_aged(&root.join(".gitignore"), "widget.cpp\n"),
            "c",
        ),
    ];
    for (name, with_sibling, change, expected) in scenarios {
        let dir = TempDir::new("equivalence")?;
        let root = dir.0.join("repo");
        write_aged(&root.join("widget.h"), HEADER)?;
        write_aged(&root.join("main.c"), "int main(void) { return 0; }\n")?;
        if with_sibling {
            write_aged(&root.join("widget.cpp"), CPP)?;
        }
        git(&root, &["init", "-q"])?;
        let incremental = dir.0.join("incremental");
        assert!(run_in(&root, &incremental, &["index"])?.status.success());
        change(&root)?;
        assert!(run_in(&root, &incremental, &["index"])?.status.success());
        let show = run_in(&root, &incremental, &["show", "widget.h", "--read-only"])?;
        let show = String::from_utf8(show.stdout)?;
        assert!(
            show.starts_with(&format!("widget.h {expected} ")),
            "{name}: {show}"
        );
        let fresh = dir.0.join("fresh");
        assert!(run_in(&root, &fresh, &["index"])?.status.success());
        assert!(
            derived_image(&incremental)? == derived_image(&fresh)?,
            "{name}: incremental and fresh derived images differ"
        );
    }
    Ok(())
}

#[test]
fn final_tree_decides_the_index_regardless_of_change_order_and_threads() -> TestResult {
    let dir = TempDir::new("change-order")?;
    let root = dir.0.join("repo");
    write_aged(&root.join("widget.h"), "int area(int w);\n")?;
    write_aged(&root.join("main.c"), "int main(void) { return 0; }\n")?;
    write_aged(&root.join("util.c"), "int util(void) { return 1; }\n")?;
    git(&root, &["init", "-q"])?;
    let orders: [&[&str]; 2] = [&["add", "edit", "delete"], &["delete", "add", "edit"]];
    let mut images = Vec::new();
    for (position, order) in orders.iter().enumerate() {
        let index = dir.0.join(format!("order{position}"));
        // Reset the tree to the starting state, then index it.
        write_aged(&root.join("util.c"), "int util(void) { return 1; }\n")?;
        write_aged(&root.join("main.c"), "int main(void) { return 0; }\n")?;
        let _ = fs::remove_file(root.join("widget.cpp"));
        assert!(run_in(&root, &index, &["index"])?.status.success());
        for step in *order {
            match *step {
                "add" => write_aged(&root.join("widget.cpp"), "int area(int w) { return w; }\n")?,
                "edit" => write_aged(&root.join("main.c"), "int main(void) { return area(2); }\n")?,
                _ => fs::remove_file(root.join("util.c"))?,
            }
            assert!(run_in(&root, &index, &["index"])?.status.success());
        }
        images.push(derived_image(&index)?);
    }
    for threads in ["1", "4"] {
        let index = dir.0.join(format!("threads{threads}"));
        let output = support::command(BINARY, &dir.0)
            .args(["index", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(&index)
            .env("HOME", &index)
            .env("RAYON_NUM_THREADS", threads)
            .output()?;
        assert!(output.status.success());
        images.push(derived_image(&index)?);
    }
    assert!(
        images.windows(2).all(|pair| pair[0] == pair[1]),
        "change order or thread count changed the derived image"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn git_diff_handles_special_paths_from_a_subdirectory() -> TestResult {
    let dir = TempDir::new("git-special-diff")?;
    let repo = dir.0.join("repo");
    let root = repo.join(" \tü\n");
    fs::create_dir_all(&root)?;
    let changed = [
        ("space file.go", "BySpace", "space file.go"),
        ("üñ.go", "ByUnicode", "üñ.go"),
        ("tab\tfile.go", "ByTab", "tab\\tfile.go"),
        ("line\nfile.go", "ByNewline", "line\\nfile.go"),
        (
            "quote\"back\\slash.go",
            "ByQuote",
            "quote\\\"back\\\\slash.go",
        ),
    ];
    for (path, name, _) in changed {
        fs::write(
            root.join(path),
            format!("package sample\nfunc {name}() int {{ return 1 }}\n"),
        )?;
    }
    fs::write(root.join("delete\t.go"), "package sample\nfunc Gone() {}\n")?;
    fs::write(root.join("type.go"), "package sample\nfunc GoneType() {}\n")?;
    fs::write(
        root.join("binary.go"),
        b"package sample\nfunc GoneBinary() {}\n\0",
    )?;
    fs::write(
        root.join("plus.ts"),
        "export function First() {\nlet x = 1;\n++ x;\nreturn x;\n}\nexport function Later() { return 1; }\n",
    )?;
    fs::write(
        root.join("before ü.go"),
        "package sample\nfunc Moved() {}\n",
    )?;
    fs::write(
        repo.join("outside.go"),
        "package outside\nfunc Outside() {}\n",
    )?;
    git(&repo, &["init", "-q"])?;
    git(&repo, &["config", "core.quotePath", "true"])?;
    git(&repo, &["add", "."])?;
    git(&repo, &["commit", "-q", "-m", "initial"])?;
    for (path, name, _) in changed {
        fs::write(
            root.join(path),
            format!("package sample\nfunc {name}() int {{ return 2 }}\n"),
        )?;
    }
    fs::remove_file(root.join("delete\t.go"))?;
    fs::remove_file(root.join("type.go"))?;
    std::os::unix::fs::symlink("target", root.join("type.go"))?;
    fs::remove_file(root.join("binary.go"))?;
    fs::write(
        root.join("plus.ts"),
        "export function First() {\nlet x = 1;\n++ y;\nreturn x;\n}\nexport function Later() { return 2; }\n",
    )?;
    fs::rename(root.join("before ü.go"), root.join("after ü.go"))?;
    fs::write(root.join("added\t.go"), "package sample\nfunc Added() {}\n")?;
    fs::write(
        repo.join("outside.go"),
        "package outside\nfunc ChangedOutside() {}\n",
    )?;
    git(&repo, &["add", "-A"])?;
    fs::write(
        root.join("untracked\n.go"),
        "package sample\nfunc Untracked() {}\n",
    )?;
    let output = run_in(&root, &dir.0.join("index"), &["impact", "--diff", "--json"])?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    for (_, name, escaped_path) in changed {
        assert!(
            text.contains(&format!("\"path\":\"{escaped_path}\"")),
            "missing {name}: {text}"
        );
        assert!(text.contains(name), "missing {name}: {text}");
    }
    for escaped_path in [
        "delete\\t.go",
        "after ü.go",
        "added\\t.go",
        "untracked\\n.go",
    ] {
        assert!(
            text.contains(&format!("\"path\":\"{escaped_path}\"")),
            "missing {escaped_path}: {text}"
        );
    }
    assert!(
        text.contains("\"name\":\"Gone\"") && text.contains("\"kind\":\"removed\""),
        "{text}"
    );
    assert!(
        text.contains("GoneType") && text.contains("GoneBinary"),
        "{text}"
    );
    assert_eq!(text.matches("\"name\":\"GoneType\"").count(), 1, "{text}");
    assert!(text.contains("First") && text.contains("Later"), "{text}");
    assert!(
        !text.contains("\"name\":\"Moved\",\"path\":\"before ü.go\",\"kind\":\"removed\""),
        "{text}"
    );
    assert!(!text.contains("Outside"), "{text}");
    git(&repo, &["config", "core.quotePath", "false"])?;
    let unquoted = run_in(&root, &dir.0.join("index"), &["impact", "--diff", "--json"])?;
    assert_eq!(unquoted.status.code(), Some(0));
    assert_eq!(String::from_utf8(unquoted.stdout)?, text);
    Ok(())
}

#[cfg(unix)]
#[test]
fn git_history_handles_special_paths_from_a_subdirectory() -> TestResult {
    let dir = TempDir::new("git-special-history")?;
    let repo = dir.0.join("repo");
    let root = repo.join(" \tü\n");
    fs::create_dir_all(&root)?;
    git(&repo, &["init", "-q"])?;
    git(&repo, &["config", "core.quotePath", "true"])?;
    for version in 0..4 {
        fs::write(
            root.join("tab\t.go"),
            format!("package sample\nfunc One() int {{ return {version} }}\n"),
        )?;
        fs::write(
            root.join("üñ\n.go"),
            format!("package sample\nfunc Two() int {{ return {version} }}\n"),
        )?;
        fs::write(
            repo.join("\nroot.go"),
            format!("package outside\nfunc Outside() int {{ return {version} }}\n"),
        )?;
        git(&repo, &["add", "."])?;
        git(&repo, &["commit", "-q", "-m", "change"])?;
    }
    let output = run_in(&root, &dir.0.join("index"), &["audit", "--json"])?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert_json_lines_parse(&text, &dir.0)?;
    assert!(
        text.contains("\"path\":\"tab\\t.go\",\"commits\":4"),
        "{text}"
    );
    assert!(
        text.contains("\"path\":\"üñ\\n.go\",\"commits\":4"),
        "{text}"
    );
    assert!(
        text.contains("\"path\":\"tab\\t.go\",\"other_path\":\"üñ\\n.go\",\"n\":4"),
        "{text}"
    );
    assert!(!text.contains("root.go"), "{text}");
    let top = run_in(&repo, &dir.0.join("top-index"), &["audit", "--json"])?;
    assert_eq!(top.status.code(), Some(0));
    let top = String::from_utf8(top.stdout)?;
    assert_json_lines_parse(&top, &dir.0)?;
    assert!(
        top.contains("\"path\":\"\\nroot.go\",\"commits\":4"),
        "{top}"
    );
    Ok(())
}

#[test]
fn impact_diff_does_not_confuse_other_modules_with_real_moves() -> TestResult {
    let dir = TempDir::new("impact-identity-modules")?;
    let root = dir.0.join("repo");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write(
        "repo/a/old.go",
        "package a\n\nfunc Shared() {}\n\nfunc Moved() {}\n",
    )?;
    dir.write("repo/a/current.go", "package a\n\nfunc Keep() {}\n")?;
    dir.write(
        "repo/b/current.go",
        "package b\n\nfunc Shared() {}\n\nfunc Other() int { return 1 }\n",
    )?;
    dir.write(
        "repo/module-one/src/Example.java",
        "package shared;\nclass Example {\n  void relocate() {}\n}\n",
    )?;
    dir.write(
        "repo/module-two/src/Example.java",
        "package shared;\nclass Example {\n  void keep() {}\n}\n",
    )?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    git(&root, &["commit", "-q", "--allow-empty", "-m", "empty"])?;
    fs::remove_file(root.join("a/old.go"))?;
    fs::remove_file(root.join("module-one/src/Example.java"))?;
    fs::write(
        root.join("a/current.go"),
        "package a\n\nfunc Keep() {}\n\nfunc Moved() {}\n",
    )?;
    fs::write(
        root.join("b/current.go"),
        "package b\n\nfunc Shared() {}\n\nfunc Other() int { return 2 }\n",
    )?;
    fs::write(
        root.join("module-two/src/Example.java"),
        "package shared;\nclass Example {\n  void keep() {}\n  void relocate() {}\n}\n",
    )?;
    let index = dir.0.join("index");
    for rev in ["HEAD", "HEAD~1"] {
        let output = run_in(&root, &index, &["impact", "--diff", rev, "--json"])?;
        assert_eq!(output.status.code(), Some(0));
        let text = String::from_utf8(output.stdout)?;
        assert!(
            text.contains(
                "\"name\":\"Shared\",\"path\":\"a/old.go\",\"line\":3,\"kind\":\"removed\""
            ),
            "{text}"
        );
        assert!(
            text.contains("\"name\":\"Moved\",\"path\":\"a/old.go\",\"line\":5,\"kind\":\"moved\""),
            "{text}"
        );
        assert!(
            !text.contains(
                "\"name\":\"Shared\",\"path\":\"b/current.go\",\"line\":3,\"kind\":\"removed\""
            ),
            "{text}"
        );
        assert!(text.contains("\"name\":\"Example.relocate\",\"path\":\"module-one/src/Example.java\",\"line\":3,\"kind\":\"uncertain\",\"conf\":\"inferred\""), "{text}");
    }
    Ok(())
}

#[test]
fn impact_diff_distinguishes_method_owners_and_symbol_kinds() -> TestResult {
    let dir = TempDir::new("impact-identity-owners")?;
    let root = dir.0.join("repo");
    dir.write("repo/service.go", "package service\ntype First struct{}\ntype Second struct{}\nfunc (First) Run() int { return 1 }\nfunc (Second) Run() int { return 1 }\n")?;
    dir.write("repo/switch.ts", "export class Switch {}\n")?;
    dir.write("repo/guide.md", "# Overview\n\n## Details\nOld text.\n")?;
    dir.write("repo/app/page.tsx", "import { NextRequest } from 'next/server';\nexport default function Page() { return <main>Old</main>; }\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    fs::write(
        root.join("service.go"),
        "package service\ntype First struct{}\ntype Second struct{}\nfunc (Second) Run() int { return 2 }\n",
    )?;
    fs::write(root.join("switch.ts"), "export function Switch() {}\n")?;
    fs::write(
        root.join("guide.md"),
        "# Overview\n\n## Details\nNew text.\n",
    )?;
    fs::write(
        root.join("app/page.tsx"),
        "import { NextRequest } from 'next/server';\nexport default function Page() { return <main>New</main>; }\n",
    )?;
    let output = run_in(&root, &dir.0.join("index"), &["impact", "--diff", "--json"])?;
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.contains(
            "\"name\":\"First.Run\",\"path\":\"service.go\",\"line\":4,\"kind\":\"removed\""
        ),
        "{text}"
    );
    assert!(
        !text.contains(
            "\"name\":\"Second.Run\",\"path\":\"service.go\",\"line\":5,\"kind\":\"removed\""
        ),
        "{text}"
    );
    assert!(
        text.contains("\"name\":\"Switch\",\"path\":\"switch.ts\",\"line\":1,\"kind\":\"removed\""),
        "{text}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn impact_diff_never_reads_deleted_sensitive_revision_content() -> TestResult {
    let dir = TempDir::new("impact-sensitive-revision")?;
    let root = dir.0.join("repo");
    let sensitive_paths = [
        ".env",
        ".env.local",
        "certs/server.pem",
        "certs/service.key",
        "certs/identity.p12",
        "keys/id_rsa_backup",
        "keys/id_ed25519_backup",
        "private/.ssh/id_rsa",
        "cloud/.aws/credentials",
        "accounts/.gnupg/private-key",
    ];
    let mut markers = Vec::new();
    for (index, path) in sensitive_paths.iter().enumerate() {
        let marker = format!("SENSITIVE_REVISION_MARKER_{index}");
        dir.write(&format!("repo/{path}"), &format!("{marker}\n"))?;
        markers.push(marker);
    }
    dir.write(
        "repo/safe.go",
        "package sample\n// SAFE_REVISION_MARKER\nfunc Safe() {}\n",
    )?;
    git(&root, &["init", "-q"])?;
    let mut add = vec!["add", "-f"];
    add.extend(sensitive_paths);
    add.push("safe.go");
    git(&root, &add)?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    let mut remove = vec!["rm", "-q"];
    remove.extend(sensitive_paths);
    remove.push("safe.go");
    git(&root, &remove)?;
    dir.write("repo/fresh.go", "package sample\nfunc Fresh() {}\n")?;
    git(&root, &["add", "fresh.go"])?;
    let probe = install_git_probe(&dir)?;
    let output = support::command(BINARY, &dir.0)
        .args(["impact", "--diff", "--json", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(dir.0.join("index"))
        .env("HOME", dir.0.join("home"))
        .env("PATH", &probe.path)
        .env("REAL_GIT", &probe.real_git)
        .env("TRACE_PATH", &probe.trace)
        .env("CAPTURE_PATH", &probe.captured)
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(&probe.trace)?;
    let captured = fs::read_to_string(&probe.captured)?;
    for marker in markers {
        assert!(
            !captured.contains(&marker),
            "read protected content: {marker}"
        );
    }
    assert!(captured.contains("SAFE_REVISION_MARKER"), "{captured}");
    assert!(!calls.contains("HEAD:.env"), "{calls}");
    let rename_call = calls
        .lines()
        .find(|line| line.contains("--find-renames"))
        .ok_or("safe deletion and addition should request rename detection")?;
    assert!(
        rename_call.contains(":(top,literal)safe.go"),
        "{rename_call}"
    );
    assert!(
        rename_call.contains(":(top,literal)fresh.go"),
        "{rename_call}"
    );
    for path in sensitive_paths {
        assert!(!rename_call.contains(path), "{rename_call}");
    }
    let json = String::from_utf8(output.stdout)?;
    assert!(json.contains("\"name\":\"Safe\""), "{json}");
    for path in sensitive_paths {
        assert!(
            json.contains(&format!(
                "\"name\":\"path-only revision\",\"path\":\"{path}\""
            )),
            "{json}"
        );
    }
    Ok(())
}

#[test]
fn impact_diff_handles_modified_sensitive_files_as_path_only() -> TestResult {
    let dir = TempDir::new("impact-sensitive-current")?;
    let root = dir.0.join("repo");
    let index = dir.0.join("index");
    dir.write("repo/.gitignore", "!.env*\n")?;
    dir.write("repo/.env", "DUMMY=before\n")?;
    dir.write("repo/safe.go", "package sample\nfunc Safe() {}\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", ".gitignore", ".env", "safe.go"])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    let indexed = run_in(&root, &index, &["index"])?;
    assert_eq!(
        indexed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    dir.write("repo/.env", "DUMMY=after\n")?;
    dir.write("repo/.env.new", "DUMMY=new\n")?;
    git(&root, &["add", "-f", ".env.new"])?;

    let output = run_in(&root, &index, &["impact", "--diff", "--json"])?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(
        !text.contains("DUMMY=before") && !text.contains("DUMMY=after"),
        "{text}"
    );
    assert!(
        text.contains("\"name\":\"path-only revision\",\"path\":\".env\""),
        "{text}"
    );
    assert!(
        text.contains("\"name\":\"path-only current\",\"path\":\".env.new\""),
        "{text}"
    );
    assert!(!text.contains("\"kind\":\"new 0\""), "{text}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn impact_diff_keeps_oversized_revision_blobs_path_only() -> TestResult {
    const LARGE_REVISION_BYTES: usize = 8 * 1024 * 1024 + 1;

    let dir = TempDir::new("impact-large-revision")?;
    let root = dir.0.join("repo");
    let mut large = b"LARGE_REVISION_MARKER\n".to_vec();
    large.resize(LARGE_REVISION_BYTES, b'x');
    dir.write_bytes("repo/large.txt", &large)?;
    dir.write("repo/safe.go", "package sample\nfunc Safe() {}\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "large.txt", "safe.go"])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    git(&root, &["rm", "-q", "large.txt", "safe.go"])?;
    let probe = install_git_probe(&dir)?;

    let output = support::command(BINARY, &dir.0)
        .args(["impact", "--diff", "--json", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(dir.0.join("index"))
        .env("HOME", dir.0.join("home"))
        .env("PATH", &probe.path)
        .env("REAL_GIT", &probe.real_git)
        .env("TRACE_PATH", &probe.trace)
        .env("CAPTURE_PATH", &probe.captured)
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = String::from_utf8(output.stdout)?;
    let calls = fs::read_to_string(&probe.trace)?;
    let captured = fs::read_to_string(&probe.captured)?;
    assert!(!captured.contains("LARGE_REVISION_MARKER"));
    assert!(
        json.contains("\"name\":\"path-only revision\",\"path\":\"large.txt\""),
        "{json}"
    );
    assert!(json.contains("\"name\":\"Safe\""), "{json}");
    assert!(!calls.contains("HEAD:large.txt"), "{calls}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn impact_diff_keeps_gitlinks_path_only() -> TestResult {
    let dir = TempDir::new("impact-gitlink-revision")?;
    let submodule = dir.0.join("submodule");
    dir.write("submodule/source.go", "package sample\nfunc Nested() {}\n")?;
    git(&submodule, &["init", "-q"])?;
    git(&submodule, &["add", "source.go"])?;
    git(&submodule, &["commit", "-q", "-m", "submodule"])?;
    let revision = support::command("git", &dir.0)
        .arg("-C")
        .arg(&submodule)
        .args(["rev-parse", "HEAD"])
        .output()?;
    assert!(
        revision.status.success(),
        "{}",
        String::from_utf8_lossy(&revision.stderr)
    );
    let revision = String::from_utf8(revision.stdout)?.trim().to_owned();
    let dir_entry = format!("160000,{revision},nested");

    let root = dir.0.join("repo");
    dir.write("repo/safe.go", "package sample\nfunc Safe() {}\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "safe.go"])?;
    git(&root, &["update-index", "--add", "--cacheinfo", &dir_entry])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    git(&root, &["update-index", "--force-remove", "nested"])?;

    let output = run_in(&root, &dir.0.join("index"), &["impact", "--diff", "--json"])?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = String::from_utf8(output.stdout)?;
    assert!(
        json.contains("\"name\":\"path-only revision\",\"path\":\"nested\""),
        "{json}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn impact_diff_reports_existing_revision_read_failures() -> TestResult {
    let dir = TempDir::new("impact-revision-read-error")?;
    let root = dir.0.join("repo");
    dir.write("repo/safe.go", "package sample\nfunc Safe() {}\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["config", "gc.auto", "0"])?;
    git(&root, &["add", "safe.go"])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    let object = support::command("git", &dir.0)
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD:safe.go"])
        .output()?;
    assert!(
        object.status.success(),
        "{}",
        String::from_utf8_lossy(&object.stderr)
    );
    let object = String::from_utf8(object.stdout)?.trim().to_owned();
    assert!(matches!(object.len(), 40 | 64), "{object}");
    assert!(
        object.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{object}"
    );
    let (directory, filename) = object.split_at(2);
    let blob = root.join(".git/objects").join(directory).join(filename);
    assert!(blob.is_file(), "missing loose test blob {blob:?}");
    fs::remove_file(blob)?;
    fs::remove_file(root.join("safe.go"))?;

    let output = run_in(&root, &dir.0.join("index"), &["impact", "--diff", "--json"])?;
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("revision blob"), "{stderr}");
    assert!(String::from_utf8(output.stdout)?.is_empty());
    Ok(())
}

#[test]
fn impact_diff_identifies_a_renamed_file_with_edits() -> TestResult {
    let dir = TempDir::new("impact-edited-rename")?;
    let root = dir.0.join("repo");
    let initial = "def moved(value):\n    first = value + 10\n    second = first * 2\n    third = second - 4\n    fourth = third * 3\n    return fourth\n";
    dir.write("repo/old.py", initial)?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    git(&root, &["mv", "old.py", "new.py"])?;
    let edited = initial.replace("second - 4", "second - 5");
    assert_ne!(edited, initial);
    fs::write(root.join("new.py"), edited)?;
    git(&root, &["add", "new.py"])?;
    let output = run_in(&root, &dir.0.join("index"), &["impact", "--diff", "--json"])?;
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("\"name\":\"moved\",\"path\":\"old.py\",\"line\":1,\"kind\":\"moved\",\"new_path\":\"new.py\",\"new_line\":1,\"conf\":\"inferred\""), "{text}");
    let exact = run_in(
        &root,
        &dir.0.join("index"),
        &["impact", "--diff", "--json", "--exact"],
    )?;
    assert_eq!(exact.status.code(), Some(0));
    assert!(!String::from_utf8(exact.stdout)?.contains("\"kind\":\"moved\""));

    for json in [false, true] {
        let mut old_path = vec!["impact", "--diff"];
        let mut new_path = vec!["impact", "--diff"];
        if json {
            old_path.push("--json");
            new_path.push("--json");
        }
        old_path.extend_from_slice(&["-p", "old.py"]);
        new_path.extend_from_slice(&["-p", "new.py"]);

        let old_output = run_in(&root, &dir.0.join("index"), &old_path)?;
        assert_eq!(old_output.status.code(), Some(0));
        let old_output = String::from_utf8(old_output.stdout)?;
        let old_match = if json {
            old_output
                .contains("\"name\":\"moved\",\"path\":\"old.py\",\"line\":1,\"kind\":\"removed\"")
        } else {
            old_output.contains("~ moved old.py:1 removed")
        };
        assert!(old_match, "{old_output}");
        assert!(!old_output.contains("new.py"), "{old_output}");

        let new_output = run_in(&root, &dir.0.join("index"), &new_path)?;
        assert_eq!(new_output.status.code(), Some(0));
        let new_output = String::from_utf8(new_output.stdout)?;
        assert!(new_output.contains("new.py"), "{new_output}");
        assert!(new_output.contains("new 1"), "{new_output}");
        assert!(!new_output.contains("old.py"), "{new_output}");
    }

    let loose = TempDir::new("impact-untracked-rename")?;
    let loose_root = loose.0.join("repo");
    loose.write("repo/old.py", initial)?;
    git(&loose_root, &["init", "-q"])?;
    git(&loose_root, &["add", "."])?;
    git(&loose_root, &["commit", "-q", "-m", "initial"])?;
    fs::rename(loose_root.join("old.py"), loose_root.join("new.py"))?;
    fs::write(
        loose_root.join("new.py"),
        initial.replace("second - 4", "second - 5"),
    )?;
    let untracked = run_in(
        &loose_root,
        &loose.0.join("index"),
        &["impact", "--diff", "--json"],
    )?;
    assert_eq!(untracked.status.code(), Some(0));
    assert!(String::from_utf8(untracked.stdout)?.contains("\"name\":\"moved\",\"path\":\"old.py\",\"line\":1,\"kind\":\"moved\",\"new_path\":\"new.py\""));
    fs::write(
        loose_root.join("new.py"),
        "def moved(value):\n    return value\n",
    )?;
    let uncertain = run_in(
        &loose_root,
        &loose.0.join("index"),
        &["impact", "--diff", "--json"],
    )?;
    assert_eq!(uncertain.status.code(), Some(0));
    let uncertain = String::from_utf8(uncertain.stdout)?;
    assert!(uncertain.contains("\"name\":\"moved\",\"path\":\"old.py\",\"line\":1,\"kind\":\"uncertain\",\"conf\":\"inferred\""), "{uncertain}");
    Ok(())
}

#[test]
fn impact_diff_distinguishes_overloads_and_reports_ambiguous_changes() -> TestResult {
    let dir = TempDir::new("impact-overloads")?;
    let root = dir.0.join("repo");
    let initial = "class Example {\n  void call(int value) {}\n  void call(String value) {}\n}\n";
    dir.write("repo/Example.java", initial)?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "initial"])?;
    let index = dir.0.join("index");
    fs::write(
        root.join("Example.java"),
        "class Example {\n  void call(String value) {}\n}\n",
    )?;
    let removed = run_in(&root, &index, &["impact", "--diff", "--json"])?;
    assert_eq!(removed.status.code(), Some(0));
    let removed = String::from_utf8(removed.stdout)?;
    assert!(
        removed.contains(
            "\"name\":\"Example.call\",\"path\":\"Example.java\",\"line\":2,\"kind\":\"removed\""
        ),
        "{removed}"
    );
    assert!(
        !removed.contains("\"line\":3,\"kind\":\"removed\""),
        "{removed}"
    );
    fs::write(
        root.join("Example.java"),
        "class Example {\n  void call(boolean value) {}\n  void call(double value) {}\n}\n",
    )?;
    let ambiguous = run_in(&root, &index, &["impact", "--diff", "--json"])?;
    assert_eq!(ambiguous.status.code(), Some(0));
    let ambiguous = String::from_utf8(ambiguous.stdout)?;
    assert!(ambiguous.contains("\"name\":\"Example.call\",\"path\":\"Example.java\",\"line\":2,\"kind\":\"uncertain\",\"conf\":\"inferred\""), "{ambiguous}");
    assert!(ambiguous.contains("\"name\":\"Example.call\",\"path\":\"Example.java\",\"line\":3,\"kind\":\"uncertain\",\"conf\":\"inferred\""), "{ambiguous}");
    assert!(!ambiguous.contains("\"kind\":\"removed\""), "{ambiguous}");
    Ok(())
}

#[test]
fn impact_of_removed_files_and_symbols_names_their_surviving_dependents() -> TestResult {
    const OPEN: &str =
        "package store\n\nfunc Open(path string) int {\n\treturn len(path)\n}\n\nfunc Close() {}\n";
    let dir = TempDir::new("impact-removed")?;
    let root = dir.0.join("repo");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write("repo/store/open.go", OPEN)?;
    dir.write(
        "repo/store/use.go",
        "package store\n\nfunc Warm() int {\n\treturn Open(\"x\")\n}\n\nfunc Stop() {\n\tClose()\n}\n",
    )?;
    dir.write(
        "repo/store/open_test.go",
        "package store\n\nimport \"testing\"\n\nfunc TestOpen(t *testing.T) {\n\tOpen(\"y\")\n}\n",
    )?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-q", "-m", "init"])?;
    let index = dir.0.join("index");
    let impact = |args: &[&str]| -> Result<String, Box<dyn Error>> {
        let output = run_in(&root, &index, args)?;
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        Ok(String::from_utf8(output.stdout)?)
    };
    let restore = || fs::write(root.join("store/open.go"), OPEN);
    // Whole file deleted (staged): removed definitions, inferred survivors, their tests.
    git(&root, &["rm", "-q", "store/open.go"])?;
    let text = impact(&["impact", "--diff", "--depth", "3"])?;
    assert_eq!(
        text,
        "~ Close store/open.go:7 removed\n~ Open store/open.go:3 removed\n< 1 Warm store/use.go:3?\n< 1 Stop :7?\nt TestOpen store/open_test.go:5?\nr go test ./store -run '^(TestOpen)$'\n"
    );
    assert_impact_large_depth_matches(
        &root,
        &index,
        &["impact", "--diff", "--depth", "1000000000"],
        &text,
    )?;
    let json = impact(&["impact", "--diff", "--json"])?;
    assert!(
        json.contains("{\"t\":\"sym\",\"rel\":\"chg\",\"name\":\"Open\",\"path\":\"store/open.go\",\"line\":3,\"kind\":\"removed\"}"),
        "{json}"
    );
    assert!(json.contains("\"name\":\"store.Warm\",\"path\":\"store/use.go\",\"line\":3,\"n\":1,\"conf\":\"inferred\""), "{json}");
    assert!(json.contains("\"name\":\"store.TestOpen\",\"path\":\"store/open_test.go\",\"line\":5,\"conf\":\"inferred\""), "{json}");
    let text = impact(&["impact", "--diff", "--no-tests"])?;
    assert!(
        !text.contains("TestOpen") && !text.contains("go test"),
        "{text}"
    );
    git(&root, &["reset", "-q", "HEAD", "--", "store/open.go"])?;
    restore()?;
    // One function deleted as the last lines, unstaged on top of a staged edit elsewhere.
    fs::write(
        root.join("store/use.go"),
        "package store\n\n// Warm opens.\nfunc Warm() int {\n\treturn Open(\"x\")\n}\n\nfunc Stop() {\n\tClose()\n}\n",
    )?;
    git(&root, &["add", "store/use.go"])?;
    fs::write(
        root.join("store/open.go"),
        "package store\n\nfunc Open(path string) int {\n\treturn len(path)\n}\n",
    )?;
    let text = impact(&["impact", "--diff"])?;
    assert!(text.contains("~ Close store/open.go:7 removed\n"), "{text}");
    assert!(text.contains("< 1 Stop :8?\n"), "{text}");
    assert!(!text.contains("Open store/open.go:3 removed"), "{text}");
    git(&root, &["checkout", "-q", "HEAD", "--", "store/use.go"])?;
    restore()?;
    // Rename: definitions moved, nothing removed, dependents stay exact.
    git(&root, &["mv", "store/open.go", "store/opening.go"])?;
    let text = impact(&["impact", "--diff"])?;
    assert!(text.contains("~ store/opening.go:1 new 2\n"), "{text}");
    assert!(
        !text.contains("removed") && text.contains("moved?"),
        "{text}"
    );
    assert!(text.contains("< 1 Warm store/use.go:3\n"), "{text}");
    git(&root, &["mv", "store/opening.go", "store/open.go"])?;
    // Non-HEAD revision.
    git(&root, &["commit", "-q", "--allow-empty", "-m", "empty"])?;
    fs::remove_file(root.join("store/open.go"))?;
    let text = impact(&["impact", "--diff", "HEAD~1"])?;
    assert!(text.contains("~ Open store/open.go:3 removed\n"), "{text}");
    restore()?;
    // Unchanged tree: no false changes.
    assert_eq!(impact(&["impact", "--diff"])?, "");
    Ok(())
}

#[test]
fn compact_and_minified_symbols_are_found_by_their_own_body_words() -> TestResult {
    let dir = TempDir::new("compact-bodies")?;
    let root = dir.0.join("repo");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write(
        "repo/space.go",
        "package app\n\nfunc Quasar() { launchRocketEngine() }\n\nfunc Nebula() {\n\tcollectStardustSamples()\n}\n\nfunc Other() { brewMorningCoffee() }\n",
    )?;
    dir.write(
        "repo/lib.rs",
        "fn ignite() { start_plasma_coil(); } fn idle() { sleep_quietly_forever(); }\n",
    )?;
    dir.write(
        "repo/tools.py",
        "def purge(): wipe_archive_cache()\ndef keep(): retain_journal_pages()\n",
    )?;
    dir.write(
        "repo/min.js",
        "const a=()=>sendWelcomeEmail();const b=()=>archiveOldInvoices();function c(){rotateSigningKeys()}\n",
    )?;
    git(&root, &["init", "-q"])?;
    let index = dir.0.join("index");
    // (question, first symbol line, neighbor that must not be listed)
    let cases = [
        ("launch rocket engine", "  3 func Quasar()\n", "Other"),
        ("collect stardust samples", "  5 func Nebula()\n", "Quasar"),
        ("brew morning coffee", "  9 func Other()\n", "Quasar"),
        ("start plasma coil", "  1 fn ignite()\n", "idle"),
        ("sleep quietly forever", "  1 fn idle()\n", "ignite"),
        ("wipe archive cache", "  1 def purge():\n", "keep"),
        ("retain journal pages", "  2 def keep():\n", "purge"),
        (
            "send welcome email",
            "  1 const a=()=>sendWelcomeEmail()\n",
            "rotateSigningKeys",
        ),
        (
            "rotate signing keys",
            "  1 function c()\n",
            "sendWelcomeEmail",
        ),
    ];
    for (question, first, neighbor) in cases {
        let mut args = vec!["find"];
        args.extend(question.split(' '));
        let output = run_in(&root, &index, &args)?;
        let text = String::from_utf8(output.stdout)?;
        let symbol = text
            .lines()
            .find(|line| line.starts_with("  "))
            .unwrap_or("");
        assert_eq!(format!("{symbol}\n"), first, "{question}\n{text}");
        assert!(
            !text.contains(neighbor),
            "{question} lists {neighbor}\n{text}"
        );
    }
    Ok(())
}

#[test]
fn hyphenated_ui_text_is_searchable_while_class_lists_are_not() -> TestResult {
    let dir = TempDir::new("class-context")?;
    let root = dir.0.join("repo");
    dir.write(
        "repo/src/MobileNav.tsx",
        "import { cn } from './utils'\n\nexport function MobileNav({ open }: { open: boolean }) {\n  return (\n    <nav aria-label=\"small-screen navigation\" className={cn(\n      \"flex items-center justify-between\",\n      open && \"translate-x-0 duration-300\"\n    )}>\n      <span className=\"text-sm font-medium\">Menu</span>\n    </nav>\n  )\n}\n\nexport function Banner() {\n  console.warn('read-only mode enabled')\n  return <div className=\"rounded-lg border-dashed p-4\">Maintenance</div>\n}\n",
    )?;
    dir.write(
        "repo/src/Toggle.svelte",
        "<script>\n  export let dark = false\n</script>\n\n<button class=\"rounded-full px-3 py-1\" title=\"switch color-scheme preference\" on:click={() => dark = !dark}>Theme</button>\n",
    )?;
    git(&root, &["init", "-q"])?;
    let index = dir.0.join("index");
    let find = |question: &str| -> Result<(Option<i32>, String), Box<dyn Error>> {
        let mut args = vec!["find"];
        args.extend(question.split(' '));
        let output = run_in(&root, &index, &args)?;
        Ok((output.status.code(), String::from_utf8(output.stdout)?))
    };
    for (question, symbol) in [
        ("small screen navigation", "  3 function MobileNav("),
        ("read only mode", "  14 function Banner()"),
        ("color scheme preference", "  1 component Toggle"),
    ] {
        let (code, text) = find(question)?;
        assert_eq!(code, Some(0), "{question}");
        assert!(text.contains(symbol), "{question}\n{text}");
    }
    for classes in [
        "items center justify between",
        "translate duration",
        "rounded full",
        "border dashed",
    ] {
        assert_eq!(find(classes)?.0, Some(4), "{classes} is class noise");
    }
    Ok(())
}

#[test]
fn impact_run_commands_stay_inside_the_budget() -> TestResult {
    let dir = TempDir::new("impact-budget")?;
    let root = dir.0.join("repo");
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write(
        "repo/core/core.go",
        "package core\n\nfunc Ünïcode() int { return 1 }\n",
    )?;
    for package in ["alpha", "beta", "gamma"] {
        dir.write(
            &format!("repo/{package}/use.go"),
            &format!("package {package}\n\nimport \"example.com/app/core\"\n\nfunc Use() int {{ return core.Ünïcode() }}\n"),
        )?;
        dir.write(
            &format!("repo/{package}/use_test.go"),
            &format!("package {package}\n\nimport \"testing\"\n\nfunc TestAVeryLongDescriptiveNameForUsingTheCoreFunctionOne(t *testing.T) {{ Use() }}\nfunc TestAVeryLongDescriptiveNameForUsingTheCoreFunctionTwo(t *testing.T) {{ Use() }}\n"),
        )?;
    }
    git(&root, &["init", "-q"])?;
    let index = dir.0.join("index");
    let impact = |budget: &str, json: bool| -> Result<String, Box<dyn Error>> {
        let mut args = vec!["impact", "core.Ünïcode", "-b", budget];
        if json {
            args.push("--json");
        }
        let output = run_in(&root, &index, &args)?;
        assert_eq!(output.status.code(), Some(0));
        Ok(String::from_utf8(output.stdout)?)
    };
    // A large budget keeps the precise commands.
    let wide = impact("1500", false)?;
    assert!(
        wide.contains("r go test ./alpha -run '^(TestAVeryLong"),
        "{wide}"
    );
    // A small one keeps package-level commands, reserved before dependents and tests.
    for budget in [200usize, 60, 30] {
        for json in [false, true] {
            let text = impact(&budget.to_string(), json)?;
            assert!(
                text.len() <= budget * 3,
                "{budget} {json}: {} bytes\n{text}",
                text.len()
            );
            assert!(!text.contains("-run"), "{budget}\n{text}");
            if json {
                assert!(
                    text.contains("\"rel\":\"run\",\"name\":\"go test ./alpha\""),
                    "{text}"
                );
            } else {
                assert!(text.contains("r go test ./alpha\n"), "{text}");
            }
        }
    }
    // Below one item the answer is the truncation line alone.
    assert_eq!(impact("10", false)?, "+13 more\n");
    Ok(())
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
    assert!(text.contains("~ Open store/store.go:3 sig\n"), "{text}");
    assert!(text.contains("< 1 Warm store/use.go:3\n"), "{text}");
    assert!(
        text.contains("t TestWarm store/store_test.go:5\n"),
        "{text}"
    );
    assert!(
        text.contains("r go test ./store -run '^(TestWarm)$'\n"),
        "{text}"
    );
    let named = run_in(&root, &dir.0.join("index"), &["impact", "store.Warm"])?;
    let text = String::from_utf8(named.stdout)?;
    assert!(text.starts_with("~ Warm store/use.go:3\n"), "{text}");
    assert!(text.contains("t TestWarm"), "{text}");
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
    assert!(text.contains("t StoreTest.opens "), "{text}");
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
    assert!(text.contains("t StoreTests.Opens "), "{text}");
    assert!(
        text.contains("r dotnet test tests/App.Tests --filter 'FullyQualifiedName~App.Tests.StoreTests.Opens'\n"),
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

#[test]
fn map_and_audit_json_preserve_typed_facts_and_provenance() -> TestResult {
    let dir = TempDir::new("json-facts")?;
    let root = dir.0.join("repo-ü");
    let index = dir.0.join("index");
    dir.write("repo-ü/.env.example", "UNUSED_FLAG=1\n")?;
    dir.write("repo-ü/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write(
        "repo-ü/Makefile",
        "build: gen\n\tgo build ./...\n\nrun: build\n\tgo run ./cmd/server\n",
    )?;
    dir.write(
        "repo-ü/api/openapi.yaml",
        "openapi: 3.0.0\npaths:\n  /users/{id}:\n    get:\n      operationId: getUser\n",
    )?;
    dir.write(
        "repo-ü/src/ü.go",
        "package server\n\nimport \"os\"\n\nfunc GetUser(id string) string { return id }\nfunc ReadConfig() string { return os.Getenv(\"MISSING_TOKEN\") }\n",
    )?;
    dir.write(
        "repo-ü/tests/ü_test.go",
        "package server\n\nimport (\"os\"; \"testing\")\n\nfunc TestReadConfig(t *testing.T) { _ = os.Getenv(\"TEST_TOKEN\") }\n",
    )?;
    dir.write(
        "repo-ü/docs/guide.md",
        "# Guide\n\nSee [missing](docs/missing.md).\n",
    )?;

    let map_text = run_in(&root, &index, &["map"])?;
    assert_eq!(map_text.status.code(), Some(0));
    let map_text = String::from_utf8(map_text.stdout)?;
    assert!(
        map_text.starts_with("repo-ü 7 files 7 symbols go:2\n"),
        "{map_text}"
    );
    assert!(map_text.contains("src 1f 2s"), "{map_text}");
    assert!(map_text.contains("run Makefile:4 > build"), "{map_text}");
    assert!(
        map_text.contains("api.getUser GET /users/{id} api/openapi.yaml:4 < src.GetUser"),
        "{map_text}"
    );
    assert!(
        map_text.contains("MISSING_TOKEN 1r - src/ü.go:6"),
        "{map_text}"
    );

    let map_json = run_in(&root, &index, &["map", "--json"])?;
    assert_eq!(map_json.status.code(), Some(0));
    let map_json = String::from_utf8(map_json.stdout)?;
    assert_json_lines_parse(&map_json, &dir.0)?;
    assert!(
        map_json.lines().any(|line| {
            line.starts_with("{\"t\":\"root\"")
                && line.contains("\"n\":7")
                && line.contains("\"symbols\":7")
                && line.contains("repo-ü")
        }),
        "{map_json}"
    );
    assert!(
        map_json.lines().any(|line| {
            line.starts_with("{\"t\":\"mod\"")
                && line.contains("\"name\":\"src\"")
                && line.contains("\"files\":1")
        }),
        "{map_json}"
    );
    assert!(map_json.contains("\"runs\":[\"build\"]"), "{map_json}");
    assert!(
        map_json.contains("\"sig\":\"GET /users/{id}\",\"handlers\":[\"src.GetUser\"]"),
        "{map_json}"
    );
    assert!(
        map_json.contains(
            "\"name\":\"MISSING_TOKEN\",\"n\":1,\"sig\":\"-\",\"path\":\"src/ü.go\",\"line\":6"
        ),
        "{map_json}"
    );
    assert_eq!(
        map_json,
        String::from_utf8(run_in(&root, &index, &["map", "--json"])?.stdout)?
    );

    let audit_text = run_in(&root, &index, &["audit"])?;
    assert_eq!(audit_text.status.code(), Some(0));
    let audit_text = String::from_utf8(audit_text.stdout)?;
    assert!(
        audit_text.contains("src.ReadConfig src/ü.go:6"),
        "{audit_text}"
    );
    assert!(
        audit_text.contains("docs/missing.md docs/guide.md:3"),
        "{audit_text}"
    );
    assert!(
        audit_text.contains("read MISSING_TOKEN src/ü.go:6 undefined"),
        "{audit_text}"
    );

    let audit_json = run_in(&root, &index, &["audit", "--json"])?;
    assert_eq!(audit_json.status.code(), Some(0));
    let audit_json = String::from_utf8(audit_json.stdout)?;
    assert_json_lines_parse(&audit_json, &dir.0)?;
    for record in [
        "\"rel\":\"unused\",\"name\":\"src.ReadConfig\",\"path\":\"src/ü.go\",\"line\":6",
        "\"rel\":\"stale\",\"name\":\"docs/missing.md\",\"path\":\"docs/guide.md\",\"line\":3",
        "\"rel\":\"drift\",\"name\":\"MISSING_TOKEN\",\"path\":\"src/ü.go\",\"line\":6,\"kind\":\"read\",\"status\":\"undefined\"",
        "\"rel\":\"drift\",\"name\":\"UNUSED_FLAG\",\"path\":\".env.example\",\"line\":1,\"kind\":\"defined\",\"status\":\"unread\"",
        "\"rel\":\"drift\",\"name\":\"TEST_TOKEN\",\"path\":\"tests/ü_test.go\",\"line\":5,\"kind\":\"read\",\"status\":\"undefined\"",
    ] {
        assert!(
            audit_json.contains(record),
            "missing {record}\n{audit_json}"
        );
    }
    assert_eq!(
        audit_json,
        String::from_utf8(run_in(&root, &index, &["audit", "--json"])?.stdout)?
    );

    let filtered_map = run_in(&root, &index, &["map", "-p", "src/", "--json"])?;
    let filtered_map = String::from_utf8(filtered_map.stdout)?;
    assert_json_lines_parse(&filtered_map, &dir.0)?;
    assert!(filtered_map.contains("\"n\":1"), "{filtered_map}");
    assert!(filtered_map.contains("\"symbols\":2"), "{filtered_map}");
    assert!(
        filtered_map.contains("\"path\":\"src/ü.go\""),
        "{filtered_map}"
    );
    assert!(!filtered_map.contains("UNUSED_FLAG"), "{filtered_map}");

    let filtered_audit = run_in(&root, &index, &["audit", "-p", "src/", "--json"])?;
    let filtered_audit = String::from_utf8(filtered_audit.stdout)?;
    assert_json_lines_parse(&filtered_audit, &dir.0)?;
    assert!(
        filtered_audit.contains("\"path\":\"src/ü.go\""),
        "{filtered_audit}"
    );
    assert!(
        !filtered_audit.contains("docs/guide.md"),
        "{filtered_audit}"
    );
    assert!(!filtered_audit.contains("UNUSED_FLAG"), "{filtered_audit}");

    let no_tests = run_in(&root, &index, &["audit", "--no-tests", "--json"])?;
    let no_tests = String::from_utf8(no_tests.stdout)?;
    assert_json_lines_parse(&no_tests, &dir.0)?;
    assert!(!no_tests.contains("TEST_TOKEN"), "{no_tests}");

    for command in ["map", "audit"] {
        for json in [false, true] {
            for budget in [1, 10, 100] {
                let budget = budget.to_string();
                let mut args = vec![command, "-b", &budget];
                if json {
                    args.push("--json");
                }
                let output = run_in(&root, &index, &args)?;
                let output = String::from_utf8(output.stdout)?;
                assert!(output.len() <= budget.parse::<usize>()? * 3, "{output}");
                if json {
                    assert_json_lines_parse(&output, &dir.0)?;
                }
            }
        }
    }
    Ok(())
}

#[test]
fn show_attributes_environment_reads_to_exact_symbol_or_file_in_text_and_json() -> TestResult {
    let dir = TempDir::new("env-read-ownership")?;
    let root = dir.0.join("repo");
    dir.write(
        "repo/src/env.ts",
        "process.env.FILE_ONLY;\n\
         function outer() {\n\
           function unused() { return process.env.NESTED_ONLY; }\n\
           return process.env.OUTER_ONLY;\n\
         }\n\
         class Config {\n\
           read() { return process.env.METHOD_ONLY; }\n\
         }\n\
         function first() { return process.env.FIRST_ONLY; } function second() { return process.env.SECOND_ONLY; }\n",
    )?;
    let index = dir.0.join("index");
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "src/env.ts",
            &["FILE_ONLY"],
            &[
                "NESTED_ONLY",
                "OUTER_ONLY",
                "METHOD_ONLY",
                "FIRST_ONLY",
                "SECOND_ONLY",
            ],
        ),
        ("outer", &["OUTER_ONLY"], &["NESTED_ONLY", "FILE_ONLY"]),
        ("unused", &["NESTED_ONLY"], &["OUTER_ONLY", "FILE_ONLY"]),
        ("Config", &[], &["METHOD_ONLY"]),
        ("Config.read", &["METHOD_ONLY"], &["FILE_ONLY"]),
        ("first", &["FIRST_ONLY"], &["SECOND_ONLY"]),
        ("second", &["SECOND_ONLY"], &["FIRST_ONLY"]),
    ];

    for json in [false, true] {
        for (target, expected, forbidden) in cases {
            let mut args = vec!["show", target];
            if json {
                args.push("--json");
            }
            let output = run_in(&root, &index, &args)?;
            assert_eq!(
                output.status.code(),
                Some(0),
                "show {target:?}, json={json}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let rendered = String::from_utf8(output.stdout)?;
            for name in *expected {
                assert!(
                    rendered.contains(name),
                    "show {target:?}, json={json}, missing {name}: {rendered}"
                );
            }
            for name in *forbidden {
                assert!(
                    !rendered.contains(name),
                    "show {target:?}, json={json}, leaked {name}: {rendered}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn map_lists_only_external_dependencies_of_filtered_modules() -> TestResult {
    let dir = TempDir::new("map-external-filter")?;
    dir.write("repo/.git/HEAD", "ref: refs/heads/main\n")?;
    dir.write("repo/go.mod", "module example.com/app\n\ngo 1.23\n")?;
    dir.write(
        "repo/a/a.go",
        "package a\n\nimport \"github.com/acme/left\"\n\nfunc A() { left.Run() }\n",
    )?;
    dir.write(
        "repo/b/b.go",
        "package b\n\nimport \"github.com/acme/right\"\n\nfunc B() { right.Run() }\n",
    )?;
    let map = |filter: &[&str]| -> Result<String, Box<dyn Error>> {
        let output = support::command(BINARY, &dir.0)
            .args(["map", "-C"])
            .arg(dir.0.join("repo"))
            .arg("--index-dir")
            .arg(dir.0.join("index"))
            .args(filter)
            .output()?;
        assert_eq!(output.status.code(), Some(0));
        Ok(String::from_utf8(output.stdout)?)
    };
    let all = map(&[])?;
    assert!(all.contains("left") && all.contains("right"), "{all}");
    let filtered = map(&["-p", "a/"])?;
    assert!(filtered.contains("left"), "{filtered}");
    assert!(!filtered.contains("right"), "{filtered}");
    Ok(())
}

#[test]
fn install_warns_when_another_binary_shadows_the_target() -> TestResult {
    let dir = TempDir::new("install-shadow")?;
    dir.write("shadow/xcalygraph", "#!/bin/sh\nexit 0\n")?;
    let bin = dir.0.join("bin");
    let path = std::env::join_paths([dir.0.join("shadow"), bin.clone()])?;
    let output = support::command(BINARY, &dir.0)
        .args(["install", "--no-link"])
        .env("HOME", dir.0.join("home"))
        .env("XCALYGRAPH_INSTALL_DIR", &bin)
        .env("PATH", path)
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("shadows"), "{text}");
    assert!(text.contains("first on PATH"), "{text}");
    Ok(())
}

#[test]
fn building_a_new_index_removes_indexes_of_vanished_roots() -> TestResult {
    let dir = TempDir::new("prune-orphans")?;
    dir.write("gone/main.go", "package main\n")?;
    dir.write("kept/main.go", "package main\n")?;
    dir.write("next/main.go", "package main\n")?;
    // The default base lives under TMPDIR, which the test support points at the fixture.
    let index = |root: &str| -> io::Result<Output> {
        support::command(BINARY, &dir.0)
            .args(["index", "-C"])
            .arg(dir.0.join(root))
            .output()
    };
    assert_eq!(index("gone")?.status.code(), Some(0));
    assert_eq!(index("kept")?.status.code(), Some(0));
    let base = fs::read_dir(&dir.0)?
        .map(|entry| Ok(entry?.path()))
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("xcalygraph-"))
        })
        .ok_or("default base missing")?;
    fs::write(base.join("foreign-file"), "keep")?;
    let names = || -> io::Result<Vec<String>> {
        let mut names: Vec<String> = fs::read_dir(&base)?
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect::<io::Result<_>>()?;
        names.sort();
        Ok(names)
    };
    assert_eq!(names()?.len(), 3);
    fs::remove_dir_all(dir.0.join("gone"))?;
    // An update of an existing index changes nothing; only a new index triggers the pruning.
    assert_eq!(index("kept")?.status.code(), Some(0));
    assert_eq!(names()?.len(), 3);
    assert_eq!(index("next")?.status.code(), Some(0));
    let after = names()?;
    assert!(
        after.iter().any(|name| name.starts_with("kept-")),
        "{after:?}"
    );
    assert!(
        after.iter().any(|name| name.starts_with("next-")),
        "{after:?}"
    );
    assert!(after.contains(&"foreign-file".to_owned()), "{after:?}");
    assert!(
        !after.iter().any(|name| name.starts_with("gone-")),
        "{after:?}"
    );
    Ok(())
}

#[test]
fn a_clear_top_hit_answers_with_its_source_and_neighbours() -> TestResult {
    let dir = TempDir::new("find-answer")?;
    let long_body: String = (0..30)
        .map(|index| format!("    step{index}()\n"))
        .collect();
    dir.write(
        "pay.go",
        &format!(
            "package pay\n\nfunc ChargeInvoice() {{\n{long_body}    audit()\n}}\n\n\
             func audit() {{}}\n\nfunc Checkout() {{\n    ChargeInvoice()\n}}\n"
        ),
    )?;
    dir.write(
        "other.go",
        "package pay\n\nfunc Refund() {}\n\nfunc Rebate() {}\n",
    )?;
    dir.write(
        "pay_test.go",
        "package pay\n\nimport \"testing\"\n\nfunc TestCharge(t *testing.T) {\n    ChargeInvoice()\n}\n",
    )?;
    let root = dir.0.to_string_lossy().into_owned();
    let find = |terms: &[&str]| -> io::Result<String> {
        let mut args = vec!["find", "-C", &root];
        args.extend_from_slice(terms);
        let output = support::command(BINARY, &dir.0).args(&args).output()?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let clear = find(&["ChargeInvoice"])?;
    assert!(clear.contains("  3 func ChargeInvoice()"), "{clear}");
    assert!(
        clear.contains("    4     step0()"),
        "body follows the hit: {clear}"
    );
    assert!(
        clear.contains("    +"),
        "the rest of the body is counted: {clear}"
    );
    assert!(clear.contains("    < Checkout pay.go:"), "{clear}");
    assert!(clear.contains("    > audit :"), "{clear}");
    assert!(clear.contains("    t TestCharge pay_test.go:"), "{clear}");
    assert!(clear.contains("    r go test"), "{clear}");
    let output = support::command(BINARY, &dir.0)
        .args(["ctx", "-C", &root, "ChargeInvoice"])
        .output()?;
    let bundle = String::from_utf8_lossy(&output.stdout);
    assert!(bundle.contains("\nt TestCharge pay_test.go:"), "{bundle}");
    assert!(bundle.contains("\nr go test"), "{bundle}");
    // Query words pull their lines out of a long body.
    let named = find(&["ChargeInvoice", "step20"])?;
    assert!(named.contains("    24     step20()"), "{named}");
    // Two equal hits: the list serves better than one body.
    let tied = find(&["Re"])?;
    assert!(!tied.lines().any(|line| line.starts_with("    ")), "{tied}");
    // JSON carries the same facts.
    let output = support::command(BINARY, &dir.0)
        .args(["find", "-C", &root, "--json", "ChargeInvoice"])
        .output()?;
    let json = String::from_utf8_lossy(&output.stdout);
    assert!(
        json.lines()
            .any(|line| line.contains("\"t\":\"source\"") && line.contains("step0")),
        "{json}"
    );
    Ok(())
}

#[test]
fn every_printed_location_resolves_to_its_symbol() -> TestResult {
    let dir = TempDir::new("short-names")?;
    dir.write(
        "pkg/store.go",
        "package pkg\n\ntype Store struct{}\n\nfunc (s Store) Get() {}\n\nfunc Open() Store {\n\treturn Store{}\n}\n",
    )?;
    dir.write(
        "pkg/other.go",
        "package pkg\n\nfunc Open2() {}\n\nfunc Load() {\n\tOpen().Get()\n\tOpen2()\n}\n",
    )?;
    dir.write(
        "web/a.ts",
        "export function open() {}\nexport function run() { open(); }\n",
    )?;
    dir.write(
        "web/b.ts",
        "export function open() {}\nexport function go() { open(); }\n",
    )?;
    dir.write(
        "app/a.py",
        "def open():\n    pass\n\ndef run():\n    open()\n",
    )?;
    let root = dir.0.to_string_lossy().into_owned();
    let show = |name: &str| -> io::Result<String> {
        let output = support::command(BINARY, &dir.0)
            .args(["show", "-C", &root, name])
            .output()?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let mut resolved = 0;
    for start in [
        "Load",
        "Open",
        "pkg/store.go:5",
        "web/a.ts:2",
        "web/b.ts:2",
        "app/a.py:4",
    ] {
        let card = show(start)?;
        let mut last_path = String::new();
        for line in card.lines().skip(1) {
            let mut fields = line.split_whitespace();
            let (Some(marker), Some(label), Some(location)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if !matches!(marker, "<" | ">" | "m") {
                continue;
            }
            let Some((path, number)) = location.split_once(':') else {
                continue;
            };
            if !path.is_empty() {
                last_path = path.to_owned();
            }
            let number = number.trim_end_matches('?');
            let target = show(&format!("{last_path}:{number}"))?;
            let header = target.lines().next().unwrap_or_default();
            let expected = if marker == "<" { "" } else { label };
            assert!(
                header.starts_with(&format!("{last_path}:")) && header.contains(expected),
                "{start}: {line} resolved to {header}"
            );
            resolved += 1;
        }
    }
    assert!(resolved >= 6, "only {resolved} locations checked");
    Ok(())
}

#[test]
fn an_unreferenced_symbol_says_what_its_absence_proves() -> TestResult {
    let dir = TempDir::new("absence")?;
    dir.write(
        "app.go",
        "package app\n\nfunc Used() {}\n\nfunc unused() {}\n\nfunc Exported() {}\n\nfunc Caller() {\n\tUsed()\n}\n",
    )?;
    let root = dir.0.to_string_lossy().into_owned();
    let query = |command: &str, name: &str| -> io::Result<(String, String)> {
        let output = support::command(BINARY, &dir.0)
            .args([command, "-C", &root, name])
            .output()?;
        Ok((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    };
    let (unused, _) = query("show", "unused")?;
    assert!(
        unused.contains("! no static reference in the repository; complete analysis\n"),
        "{unused}"
    );
    let (exported, _) = query("impact", "Exported")?;
    assert!(
        exported.contains("! no static reference in the repository; complete analysis, exported\n"),
        "{exported}"
    );
    let (used, _) = query("show", "Used")?;
    assert!(!used.contains("no static reference"), "{used}");
    // Incomplete analysis: absence is unproven, not asserted.
    dir.write("broken.go", "package app\n\nfunc ( {\n")?;
    let (unproven, _) = query("show", "unused")?;
    assert!(unproven.contains("absence unproven"), "{unproven}");
    assert!(!unproven.contains("complete analysis"), "{unproven}");
    // A miss names what was searched.
    let (_, miss) = query("find", "zzznothinghere")?;
    assert!(miss.contains("rg checks exact text"), "{miss}");
    Ok(())
}

#[test]
fn map_lists_every_top_level_area_and_what_is_excluded() -> TestResult {
    let dir = TempDir::new("areas")?;
    dir.write("repo/src/a.go", "package src\n\nfunc A() {}\n")?;
    dir.write("repo/src/b.go", "package src\n\nfunc B() {}\n")?;
    dir.write("repo/docs/guide.md", "# Guide\n")?;
    dir.write("repo/assets/data.bin", "x")?;
    dir.write("repo/top.go", "package top\n")?;
    dir.write("repo/node_modules/dep/index.js", "module.exports = 1\n")?;
    let root = dir.0.join("repo").to_string_lossy().into_owned();
    let map = |extra: &[&str]| -> io::Result<String> {
        let mut args = vec!["map", "-C", &root];
        args.extend_from_slice(extra);
        let output = support::command(BINARY, &dir.0).args(&args).output()?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let text = map(&[])?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|line| *line == "areas")
        .ok_or("no areas")?;
    assert!(lines[start + 1].starts_with("src 2f 2s go:2"), "{text}");
    assert!(text.contains("\ndocs 1f 1s markdown:1"), "{text}");
    assert!(text.contains("\nassets 1f 0s path-only:1"), "{text}");
    assert!(text.contains("\n. 1f 0s go:1"), "{text}");
    assert!(text.contains("\nexcluded node_modules\n"), "{text}");
    // A filtered view cannot claim what is excluded.
    let filtered = map(&["-p", "src/"])?;
    assert!(!filtered.contains("excluded"), "{filtered}");
    Ok(())
}
