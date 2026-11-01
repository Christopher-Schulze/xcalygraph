//! End-to-end checks of the binary: stdout, stderr and exit codes.

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

type TestResult = Result<(), Box<dyn Error>>;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

/// Runs the binary with a HOME that never points at the real one.
fn run(args: &[&str]) -> io::Result<Output> {
    Command::new(BINARY)
        .args(args)
        .env(
            "HOME",
            std::env::temp_dir().join("xcalygraph-it-empty-home"),
        )
        .output()
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
        Command::new(BINARY)
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
    Command::new(BINARY)
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
    let dir = TempDir::new("clean-previous-format")?;
    dir.write("repo/main.go", "package main\n")?;
    let root = dir.0.join("repo");
    let base = dir.0.join("index");
    assert!(run_in(&root, &base, &["index"])?.status.success());
    let store = index_for(&base, "repo")?;
    let mut converted = 0;
    for (path, mut content, _) in index_snapshot(&store)? {
        if content.starts_with(b"XCALYGR\0") {
            content[8..12].copy_from_slice(&7u32.to_le_bytes());
            fs::write(path, content)?;
            converted += 1;
        }
    }
    assert!(converted > 2);
    let listed = Command::new(BINARY)
        .arg("ls")
        .arg("--index-dir")
        .arg(&base)
        .env("HOME", dir.0.join("home"))
        .output()?;
    assert!(
        String::from_utf8(listed.stdout)?.contains(&root.canonicalize()?.display().to_string())
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
    let output = Command::new(BINARY)
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
    let with_home = |args: &[&str]| Command::new(BINARY).args(args).env("HOME", &home).output();
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
    let copied_version = Command::new(&copied).arg("version").output()?;
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
fn update_check_compares_with_the_latest_release() -> TestResult {
    let dir = TempDir::new("update")?;
    dir.write("latest.json", "{\"tag_name\": \"v0.0.1\", \"assets\": []}")?;
    let output = Command::new(BINARY)
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
    let refreshed = Command::new(BINARY)
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
        fs::read_to_string(skill.join(".version"))?.trim(),
        env!("CARGO_PKG_VERSION")
    );
    dir.write("latest.json", "{\"tag_name\": \"v99.0.0\", \"assets\": []}")?;
    let newer = Command::new(BINARY)
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

/// A release directory whose binary reports `reported_version` and runs `install` through
/// `delegate` (a shell command line receiving the arguments).
fn fake_release(
    dir: &TempDir,
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
    let release = dir.0.join("release");
    fs::create_dir_all(&release)?;
    fs::write(release.join(&name), &archive)?;
    let sum: String = sha2::Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    fs::write(release.join("SHA256SUMS"), format!("{sum}  {name}\n"))?;
    fs::write(
        release.join("latest.json"),
        format!(
            "{{\"tag_name\": \"v99.0.0\", \"assets\": [{{\"browser_download_url\": \"file://{0}/{name}\"}}, {{\"browser_download_url\": \"file://{0}/SHA256SUMS\"}}]}}",
            release.display()
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
    Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/utils/install.sh"))
        .args(["--version", "99.0.0", "--no-skill"])
        .env(
            "XCALYGRAPH_RELEASE_BASE",
            format!("file://{}", dir.0.join("release").display()),
        )
        .env("XCALYGRAPH_INSTALL_DIR", dir.0.join("bin"))
        .env("HOME", dir.0.join("home"))
        .env("TMPDIR", dir.0.join("tmp"))
        .output()
}

#[cfg(unix)]
#[test]
fn installer_without_skill_preserves_foreign_temporary_entries() -> TestResult {
    use std::os::unix::fs::symlink;

    for linked in [true, false] {
        let dir = TempDir::new(if linked {
            "installer-temp-link"
        } else {
            "installer-temp-file"
        })?;
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
    Ok(script)
}

#[test]
fn release_builder_preserves_foreign_dist_on_preparation_failure() -> TestResult {
    let dir = TempDir::new("release-dist-preservation")?;
    let script = release_builder_fixture(&dir)?;
    dir.write("project/dist/foreign.txt", "preserve release data")?;
    let output = Command::new("/bin/bash")
        .arg(&script)
        .env("CARGO_NET_OFFLINE", "true")
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
    let output = Command::new("/bin/bash")
        .arg(&script)
        .env("CARGO_NET_OFFLINE", "true")
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
    let unsupported = Command::new("/bin/bash")
        .arg(&script)
        .arg("unsupported-target")
        .env("CARGO_NET_OFFLINE", "true")
        .env("HOME", dir.0.join("home"))
        .current_dir(dir.0.join("project"))
        .output()?;
    assert!(!unsupported.status.success());
    assert!(String::from_utf8(unsupported.stderr)?.contains("unsupported target"));
    assert_eq!(fs::read_dir(dir.0.join("project/dist"))?.count(), 2);
    let duplicate = TempDir::new("release-duplicate-target")?;
    let duplicate_script = release_builder_fixture(&duplicate)?;
    let output = Command::new("/bin/bash")
        .arg(duplicate_script)
        .args(["aarch64-apple-darwin", "aarch64-apple-darwin"])
        .env("CARGO_NET_OFFLINE", "true")
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
        Command::new("/bin/bash")
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
    let listing = Command::new("tar").arg("-tzf").arg(&archive).output()?;
    assert!(listing.status.success());
    let listing = String::from_utf8(listing.stdout)?;
    for file in ["xcalygraph", "LICENSE", "THIRD-PARTY-LICENSES.txt"] {
        assert!(listing.contains(&format!("{name}/{file}")), "{listing}");
    }
    let extracted = dir.0.join("extracted");
    fs::create_dir(&extracted)?;
    let unpacked = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&extracted)
        .status()?;
    assert!(unpacked.success());
    let packaged = extracted.join(name);
    assert!(
        Command::new("cmp")
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
    let check = Command::new("/bin/bash")
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
        let check = Command::new("/bin/bash")
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
        let latest = fake_release(&dir, &delegate, "99.0.0")?;
        let exe = dir.0.join("bin/xcalygraph");
        fs::create_dir_all(dir.0.join("bin"))?;
        fs::copy(BINARY, &exe)?;
        let home = dir.0.join("home");
        let skills = home.join(".agents/skills");
        fs::create_dir_all(&skills)?;
        if read_only {
            fs::set_permissions(&skills, fs::Permissions::from_mode(0o555))?;
        }
        let output = Command::new(&exe)
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
    let status = Command::new("git")
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
    assert!(show.contains("> FileStore.Write app.go:9?\n"), "{show}");
    // Directed path through interface dispatch: the dispatch hop is inferred in text and JSON.
    let (_, text) = query(&["path", "Send", "FileStore.Write"])?;
    assert_eq!(text, ". Send app.go:13\n> FileStore.Write app.go:9?\n");
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
        ". Top app.go:31\n> Run app.go:27\n> Send app.go:13\n> FileStore.Write app.go:9?\n"
    );
    // An equally short all-exact route wins over an inferred one listed first in source.
    let (_, text) = query(&["path", "Both", "FileStore.Write"])?;
    assert_eq!(
        text,
        ". Both app.go:22\n> Direct app.go:17\n> FileStore.Write app.go:9\n"
    );
    // --exact drops the inferred dispatch and never falls back to an undirected path.
    let (code, text) = query(&["path", "Send", "FileStore.Write", "--exact"])?;
    assert_eq!((code, text.as_str()), (Some(4), ""));
    // Undirected fallback: the reversed hop is inferred.
    let (_, text) = query(&["path", "FileStore.Write", "Direct"])?;
    assert_eq!(text, ". FileStore.Write app.go:9\n< Direct app.go:17?\n");
    // Cycles terminate and stay exact.
    let (_, text) = query(&["path", "Ping", "Pong"])?;
    assert_eq!(text, ". Ping app.go:35\n> Pong app.go:39\n");
    // Impact: uncertainty propagates downstream; an exact chain at the same depth wins.
    let (_, text) = query(&["impact", "FileStore.Write"])?;
    for line in [
        "< 1 Send app.go:13?\n",
        "< 1 Direct app.go:17\n",
        "< 2 Both app.go:22\n",
        "< 2 Run app.go:27?\n",
        "< 3 Top app.go:31?\n",
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
    for line in [
        "< 1 Both app.go:22\n",
        "< 1 Run app.go:27\n",
        "< 2 Top app.go:31\n",
    ] {
        assert!(text.contains(line), "{line}{text}");
    }
    // Deterministic across runs.
    assert_eq!(
        query(&["impact", "FileStore.Write"])?.1,
        query(&["impact", "FileStore.Write"])?.1
    );
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
        (&["path", "a.A", "c.C"], 0, &["> b.B b/b.go:5\n"], &[]),
        (
            &["path", "a.A", "c.C", "-p", "a/", "-p", "c/"],
            4,
            &[],
            &["b.B"],
        ),
        (&["show", "c/c.go:6"], 0, &["fn c.C"], &[]),
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
        (&["show", "c.C"], 0, &["t c.TestC", "< b.B"], &[]),
        (&["show", "c.C", "--no-tests"], 0, &["< b.B"], &["TestC"]),
        (
            &["impact", "c.C"],
            0,
            &["< 1 b.B", "< 2 a.A", "t c.TestC", "r go test ./c"],
            &[],
        ),
        (
            &["impact", "c.C", "--no-tests"],
            0,
            &["< 1 b.B", "< 2 a.A"],
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
            &["< 2 a.A", "t c.TestC"],
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
        let mut old_format = fs::read(&meta)?;
        old_format[8..12].copy_from_slice(&7u32.to_le_bytes());
        fs::write(&meta, old_format)?;
        let before_upgrade = index_snapshot(&index)?;
        assert_eq!(
            run_in(&root, &index, &["show", "Bravo", "--read-only"])?
                .status
                .code(),
            Some(5)
        );
        assert_eq!(
            index_snapshot(&index)?,
            before_upgrade,
            "read-only changed an old index"
        );
        let upgraded = run_in(&root, &index, &["index"])?;
        assert_eq!(upgraded.status.code(), Some(0));
        assert_eq!(
            String::from_utf8(upgraded.stdout)?,
            "files 1 changed 1 deleted 0\n"
        );
        assert_eq!(derived_image(&index)?, derived_image(&fresh)?);
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
        let output = Command::new(BINARY)
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
    assert!(text.contains("tab\\t.go 4 commits"), "{text}");
    assert!(text.contains("üñ\\n.go 4 commits"), "{text}");
    assert!(text.contains("tab\\t.go üñ\\n.go 4"), "{text}");
    assert!(!text.contains("root.go"), "{text}");
    let top = run_in(&repo, &dir.0.join("top-index"), &["audit", "--json"])?;
    assert_eq!(top.status.code(), Some(0));
    assert!(String::from_utf8(top.stdout)?.contains("\\nroot.go 4 commits"));
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
    let text = impact(&["impact", "--diff"])?;
    assert_eq!(
        text,
        "~ Close store/open.go removed\n~ Open store/open.go removed\n< 1 store.Warm store/use.go:3?\n< 1 store.Stop store/use.go:7?\nt store.TestOpen store/open_test.go:5\nr go test ./store -run '^(TestOpen)$'\n"
    );
    let json = impact(&["impact", "--diff", "--json"])?;
    assert!(
        json.contains("{\"t\":\"sym\",\"rel\":\"chg\",\"name\":\"Open\",\"path\":\"store/open.go\",\"kind\":\"removed\"}"),
        "{json}"
    );
    assert!(json.contains("\"name\":\"store.Warm\",\"path\":\"store/use.go\",\"line\":3,\"n\":1,\"conf\":\"inferred\""), "{json}");
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
    assert!(text.contains("~ Close store/open.go removed\n"), "{text}");
    assert!(text.contains("< 1 store.Stop store/use.go:8?\n"), "{text}");
    assert!(!text.contains("Open store/open.go removed"), "{text}");
    git(&root, &["checkout", "-q", "HEAD", "--", "store/use.go"])?;
    restore()?;
    // Rename: definitions moved, nothing removed, dependents stay exact.
    git(&root, &["mv", "store/open.go", "store/opening.go"])?;
    let text = impact(&["impact", "--diff"])?;
    assert!(
        text.contains("~ store/opening.go store/opening.go:1 new 2\n"),
        "{text}"
    );
    assert!(!text.contains("removed") && !text.contains('?'), "{text}");
    git(&root, &["mv", "store/opening.go", "store/open.go"])?;
    // Non-HEAD revision.
    git(&root, &["commit", "-q", "--allow-empty", "-m", "empty"])?;
    fs::remove_file(root.join("store/open.go"))?;
    let text = impact(&["impact", "--diff", "HEAD~1"])?;
    assert!(text.contains("~ Open store/open.go removed\n"), "{text}");
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
    assert!(
        text.contains("~ store.Open store/store.go:3 sig\n"),
        "{text}"
    );
    assert!(text.contains("< 1 store.Warm store/use.go:3\n"), "{text}");
    assert!(
        text.contains("t store.TestWarm store/store_test.go:5\n"),
        "{text}"
    );
    assert!(
        text.contains("r go test ./store -run '^(TestWarm)$'\n"),
        "{text}"
    );
    let named = run_in(&root, &dir.0.join("index"), &["impact", "store.Warm"])?;
    let text = String::from_utf8(named.stdout)?;
    assert!(text.starts_with("~ store.Warm store/use.go:3\n"), "{text}");
    assert!(text.contains("t store.TestWarm"), "{text}");
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
    assert!(text.contains("t com.example.StoreTest.opens "), "{text}");
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
    assert!(text.contains("t App.Tests.StoreTests.Opens "), "{text}");
    assert!(
        text.contains("r dotnet test tests/App.Tests --filter \"FullyQualifiedName~App.Tests.StoreTests.Opens\"\n"),
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
