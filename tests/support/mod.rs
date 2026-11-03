use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Creates a new, exclusively owned test directory `<repo>/.cache/<label>-<pid>-<n>`; refuses a
/// symlinked `.cache` and never reuses an existing directory.
#[allow(dead_code)]
pub fn exclusive_dir(label: &str) -> io::Result<PathBuf> {
    let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache");
    std::fs::create_dir_all(&cache)?;
    if std::fs::symlink_metadata(&cache)?.file_type().is_symlink() {
        return Err(io::Error::other("test cache must not be a symlink"));
    }
    let cache = cache.canonicalize()?;
    loop {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = cache.join(format!("{label}-{}-{sequence}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Starts a test subprocess inside its fixture, without inherited product or Git overrides.
pub fn command(program: impl AsRef<OsStr>, fixture: &Path) -> Command {
    let mut command = Command::new(program);
    for (name, _) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        if name_text.starts_with("GIT_")
            || name_text.starts_with("XCALYGRAPH_")
            || matches!(
                name_text.as_ref(),
                "BASH_ENV" | "ENV" | "CDPATH" | "TAR_OPTIONS"
            )
        {
            command.env_remove(name);
        }
    }
    command
        .env("HOME", fixture.join("home"))
        .env("TMPDIR", fixture)
        .env("XDG_CONFIG_HOME", fixture.join("config"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("CARGO_TARGET_DIR", fixture.join("cargo-target"));
    command
}
