//! Open existing index control files and create replacement files without following or
//! modifying foreign entries.

use std::fs::{self, File, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const TEMP_ATTEMPTS: u32 = 64;

/// Creates a new file `.<name>.<tag>.<pid>.<n>` beside `target` with `O_CREAT | O_EXCL`, so a
/// pre-existing entry, including a symlink, is never opened, truncated or followed; occupied
/// names are skipped. `mode` applies at creation on Unix (subject to the umask).
pub fn create_beside(target: &Path, tag: &str, mode: u32) -> io::Result<(PathBuf, File)> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("target path without name"))?;
    let pid = std::process::id();
    for attempt in 0..TEMP_ATTEMPTS {
        let temp = target.with_file_name(format!(".{name}.{tag}.{pid}.{attempt}"));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, mode);
        match options.open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no unused temporary name",
    ))
}

/// Atomically replaces `target` with `content`: a new exclusive file beside it (see
/// `create_beside`), then rename. On failure only the file created here is removed.
pub fn replace_with(target: &Path, content: &[u8], mode: u32) -> io::Result<()> {
    let (temp, mut file) = create_beside(target, "tmp", mode)?;
    let result = file
        .write_all(content)
        .and_then(|()| fs::rename(&temp, target));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// As [`replace_with`] for content in several parts.
pub fn replace_with_parts(target: &Path, parts: &[&[u8]], mode: u32) -> io::Result<()> {
    let (temp, mut file) = create_beside(target, "tmp", mode)?;
    let result = parts
        .iter()
        .try_for_each(|part| file.write_all(part))
        .and_then(|()| fs::rename(&temp, target));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// A regular file with no other hard link; symbolic links are rejected by `symlink_metadata`.
pub fn is_unshared_regular(metadata: &Metadata) -> bool {
    if !metadata.file_type().is_file() {
        return false;
    }
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() == 1
}

/// Opens only an existing regular file, without creation or truncation.
pub fn open_existing_regular(path: &Path, writable: bool) -> io::Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let before = fs::symlink_metadata(path)?;
    if !is_unshared_regular(&before) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "not an unshared regular file",
        ));
    }
    let mut options = fs::OpenOptions::new();
    options
        .read(!writable)
        .write(writable)
        .custom_flags(safe_open_flags()?);
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !is_unshared_regular(&opened) || (opened.dev(), opened.ino()) != (before.dev(), before.ino())
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "file changed while opening",
        ));
    }
    Ok(file)
}

// O_NOFOLLOW | O_NONBLOCK from each platform's fcntl headers. Linux architectures differ in
// O_NOFOLLOW; O_NONBLOCK prevents hanging on a swapped FIFO.
#[cfg(unix)]
fn safe_open_flags() -> io::Result<i32> {
    const DARWIN: i32 = 0x0100 | 0x0004;
    const LINUX_X86_64: i32 = 0x20000 | 0x0800;
    const LINUX_AARCH64: i32 = 0x8000 | 0x0800;
    if cfg!(target_os = "macos") {
        Ok(DARWIN)
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok(LINUX_X86_64)
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Ok(LINUX_AARCH64)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unsupported secure-open platform",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::infra::testdir::TestDir;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn replacement_never_touches_pre_existing_temp_entries()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("replace-temp")?;
        let target = dir.path().join("derived");
        let foreign = dir.write("outside/foreign", "foreign")?;
        let pid = std::process::id();
        let linked = dir.path().join(format!(".derived.tmp.{pid}.0"));
        symlink(&foreign, &linked)?;
        let occupied = dir.write(&format!(".derived.tmp.{pid}.1"), "occupied")?;
        replace_with(&target, b"image", 0o644)?;
        assert_eq!(fs::read(&target)?, b"image");
        assert_eq!(fs::read_to_string(&foreign)?, "foreign");
        assert!(fs::symlink_metadata(&linked)?.file_type().is_symlink());
        assert_eq!(fs::read_to_string(&occupied)?, "occupied");
        assert!(!dir.path().join(format!(".derived.tmp.{pid}.2")).exists());
        Ok(())
    }

    #[test]
    fn created_file_has_the_requested_mode_and_failure_removes_only_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("replace-mode")?;
        let target = dir.path().join("xcalygraph");
        let (temp, file) = create_beside(&target, "update", 0o755)?;
        drop(file);
        assert_eq!(fs::metadata(&temp)?.permissions().mode() & 0o111, 0o111);
        fs::create_dir(dir.path().join("blocked"))?;
        dir.write("blocked/inner", "keep")?;
        let error = replace_with(&dir.path().join("blocked"), b"image", 0o644);
        assert!(error.is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("blocked/inner"))?,
            "keep"
        );
        let leftovers: Vec<_> = fs::read_dir(dir.path())?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".blocked."))
            .collect();
        assert!(leftovers.is_empty());
        Ok(())
    }
}
