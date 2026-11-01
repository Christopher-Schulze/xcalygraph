//! Isolated temporary directories for tests; each is removed with everything it contains.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    /// Creates a new, empty directory unique to this process and call.
    pub fn new(label: &str) -> io::Result<Self> {
        let base = std::env::temp_dir().canonicalize()?;
        for _ in 0..1000 {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(
                "xcalygraph-test-{}-{label}-{unique}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("no unused test directory"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes a file below the directory, creating parent directories.
    pub fn write(&self, relative: &str, content: &str) -> io::Result<PathBuf> {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)?;
        Ok(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn colliding_entries_are_never_removed() -> io::Result<()> {
        if std::env::var_os("XCALYGRAPH_UNIT_COLLISION_CHILD").is_some() {
            let root = std::env::temp_dir();
            let prefix = format!("xcalygraph-test-{}-collision", std::process::id());
            let directory = root.join(format!("{prefix}-0"));
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
            let owned = TestDir::new("collision")?;
            assert_ne!(owned.path(), directory);
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
            return Err(io::Error::other("test cache must not be a symlink"));
        }
        let path = cache.join(format!("unit-collision-fixture-{}", std::process::id()));
        fs::create_dir(&path)?;
        let fixture = TestDir { path };
        let output = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "infra::testdir::tests::colliding_entries_are_never_removed",
            ])
            .env("TMPDIR", fixture.path())
            .env("XCALYGRAPH_UNIT_COLLISION_CHILD", "1")
            .output()?;
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }
}
