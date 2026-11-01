//! Open existing index control files without following or modifying foreign entries.

use std::fs::{self, File, Metadata};
use std::io;
use std::path::Path;

/// A regular file with no other hard link; symbolic links are rejected by `symlink_metadata`.
pub fn is_unshared_regular(metadata: &Metadata) -> bool {
    if !metadata.file_type().is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.nlink() == 1
    }
    #[cfg(not(unix))]
    false
}

/// Opens only an existing regular file, without creation or truncation.
#[cfg(unix)]
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

#[cfg(not(unix))]
pub fn open_existing_regular(_path: &Path, _writable: bool) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure index control files require Unix",
    ))
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
