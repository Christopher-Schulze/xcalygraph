//! `ls` and `clean`: the indexes in the index base, for humans tidying up. Indexes are
//! disposable; deleting one only costs the next query a full build.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::error::AppError;
use super::workspace::{self, LOCAL_DIR};
use crate::cli::Options;
use crate::infra::file_access;
use crate::infra::store::container::{Container, FORMAT_VERSION, Kind};
use crate::infra::store::{self, SHARDS, Store};

// Versions 7 and 8 use the same container envelope and meta encoding; other versions need review.
const CLEANUP_FORMATS: [u32; 3] = [FORMAT_VERSION, 8, 7];

struct Entry {
    dir: PathBuf,
    root: Option<PathBuf>,
    bytes: u64,
    used: Option<SystemTime>,
}

/// `<root> <size> <age>` per index, most recently used first; `missing` marks a deleted root.
pub fn list(options: &Options) -> Result<String, AppError> {
    let base = base(options)?;
    let mut entries = entries(&base);
    if entries.is_empty() {
        return Ok(format!("no indexes in {}\n", base.display()));
    }
    entries.sort_by(|a, b| b.used.cmp(&a.used).then(a.dir.cmp(&b.dir)));
    let total: u64 = entries.iter().map(|entry| entry.bytes).sum();
    let mut out = String::new();
    for entry in &entries {
        let root = entry
            .root
            .as_ref()
            .map_or_else(|| "?".to_owned(), |root| root.display().to_string());
        let missing = entry.root.as_ref().is_some_and(|root| !root.is_dir());
        out.push_str(&format!(
            "{root} {} {}{}\n",
            super::progress::byte_count(entry.bytes),
            age(entry.used),
            if missing { " missing" } else { "" }
        ));
    }
    out.push_str(&format!(
        "{} indexes, {} in {}\n",
        entries.len(),
        super::progress::byte_count(total),
        base.display()
    ));
    Ok(out)
}

/// Deletes verified indexes, preserving entries whose ownership cannot be established.
pub fn clean(options: &Options, roots: &[String]) -> Result<String, AppError> {
    let base = base(options)?;
    let cwd = env::current_dir().map_err(AppError::io("current directory"))?;
    let mut targets: Vec<(PathBuf, Option<PathBuf>)> = if roots.is_empty() {
        candidates(&base)
            .map_err(AppError::io(format!("read {}", base.display())))?
            .into_iter()
            .map(|dir| (dir, None))
            .collect()
    } else {
        let mut dirs = Vec::new();
        for root in roots.iter().map(|root| cwd.join(root)) {
            let canonical = root.canonicalize().unwrap_or(root);
            dirs.push((store::index_dir(&base, &canonical), Some(canonical.clone())));
            let local = canonical.join(LOCAL_DIR);
            match fs::symlink_metadata(&local) {
                Ok(metadata) if metadata.is_dir() => dirs.push((
                    store::index_dir(&local, &canonical),
                    Some(canonical.clone()),
                )),
                Ok(_) => dirs.push((local, Some(canonical))),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(AppError::io(format!("read {}", local.display()))(error)),
            }
        }
        dirs
    };
    targets.sort();
    targets.dedup();
    let mut out = String::new();
    for (dir, expected_root) in targets {
        let metadata = match fs::symlink_metadata(&dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(AppError::io(format!("read {}", dir.display()))(error)),
        };
        let root = metadata.is_dir().then(|| owned_root(&dir)).flatten();
        let Some(root) = root.filter(|root| expected_root.as_ref().is_none_or(|want| want == root))
        else {
            out.push_str(&format!(
                "skipped {}: ownership unverified\n",
                dir.display()
            ));
            continue;
        };
        if options.missing && root.is_dir() {
            continue;
        }
        let bytes = size(&dir);
        if Store::new(dir.clone())
            .drop_all()
            .map_err(AppError::io(format!("delete {}", dir.display())))?
        {
            out.push_str(&format!(
                "deleted {} {}\n",
                dir.display(),
                super::progress::byte_count(bytes)
            ));
        }
    }
    if out.is_empty() {
        out.push_str("nothing to delete\n");
    }
    Ok(out)
}

fn candidates(base: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_symlink() {
            candidates.push(path);
        } else if kind.is_dir() {
            match fs::symlink_metadata(path.join("meta")) {
                Ok(_) => candidates.push(path),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(candidates)
}

fn owned_root(dir: &Path) -> Option<PathBuf> {
    let meta = dir.join("meta");
    if !regular_file(&meta) {
        return None;
    }
    let (meta, version) = known_meta(&Store::new(dir.to_path_buf()))?;
    let root = PathBuf::from(meta.root);
    if !root.is_absolute() || dir.parent().map(|base| store::index_dir(base, &root))? != dir {
        return None;
    }
    owned_contents(dir, version).then_some(root)
}

fn known_meta(store: &Store) -> Option<(store::Meta, u32)> {
    CLEANUP_FORMATS.iter().find_map(|&version| {
        store
            .read_meta_version(version)
            .ok()
            .map(|meta| (meta, version))
    })
}

fn owned_contents(dir: &Path, version: u32) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return false;
        };
        let path = entry.path();
        let valid = match name {
            "meta" => owned_container(&path, Kind::Meta, version),
            "files" => owned_container(&path, Kind::Files, version),
            "derived" => owned_container(&path, Kind::Derived, version),
            "git" => owned_container(&path, Kind::Git, version),
            "lock" | "used" => fs::symlink_metadata(&path).is_ok_and(|metadata| {
                file_access::is_unshared_regular(&metadata) && metadata.len() == 0
            }),
            "facts" => owned_shards(&path, version),
            _ => false,
        };
        if !valid {
            return false;
        }
    }
    true
}

fn owned_shards(dir: &Path, version: u32) -> bool {
    if !fs::symlink_metadata(dir).is_ok_and(|metadata| metadata.is_dir()) {
        return false;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return false;
        };
        if name.len() != 2
            || !name.bytes().all(|byte| byte.is_ascii_digit())
            || !name.parse::<usize>().is_ok_and(|shard| shard < SHARDS)
            || !owned_container(&entry.path(), Kind::Facts, version)
        {
            return false;
        }
    }
    true
}

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| file_access::is_unshared_regular(&metadata))
}

fn owned_container(path: &Path, kind: Kind, version: u32) -> bool {
    regular_file(path) && Container::open_version(path, kind, version).is_ok()
}

fn base(options: &Options) -> Result<PathBuf, AppError> {
    let cwd = env::current_dir().map_err(AppError::io("current directory"))?;
    Ok(workspace::index_base(options.index_dir.as_deref(), &cwd))
}

fn entries(base: &Path) -> Vec<Entry> {
    let Ok(read) = fs::read_dir(base) else {
        return Vec::new();
    };
    read.filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|dir| dir.join("meta").is_file())
        .map(|dir| {
            let store = Store::new(dir.clone());
            Entry {
                root: known_meta(&store).map(|(meta, _)| PathBuf::from(meta.root)),
                bytes: size(&dir),
                used: store.last_used(),
                dir,
            }
        })
        .collect()
}

fn size(dir: &Path) -> u64 {
    let Ok(read) = fs::read_dir(dir) else {
        return 0;
    };
    read.filter_map(Result::ok)
        .map(|entry| match fs::symlink_metadata(entry.path()) {
            Ok(meta) if meta.is_dir() => size(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

/// `5m ago`, `3h ago`, `2d ago`.
fn age(time: Option<SystemTime>) -> String {
    let Some(seconds) = time
        .and_then(|time| time.elapsed().ok())
        .map(|elapsed| elapsed.as_secs())
    else {
        return "unused".to_owned();
    };
    match seconds {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86_400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::testdir::TestDir;

    #[test]
    fn ages_read_compactly() {
        assert_eq!(age(None), "unused");
        let now = SystemTime::now();
        assert_eq!(age(Some(now)), "just now");
        let hours = now - std::time::Duration::from_secs(3 * 3600 + 5);
        assert_eq!(age(Some(hours)), "3h ago");
    }

    #[test]
    fn sizes_sum_nested_files() -> std::io::Result<()> {
        let dir = TestDir::new("indexes-size")?;
        dir.write("a/meta", "12345")?;
        dir.write("a/facts/00", "123")?;
        assert_eq!(size(&dir.path().join("a")), 8);
        Ok(())
    }

    #[test]
    fn format_eight_index_remains_verifiable_for_cleanup() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = TestDir::new("indexes-format-eight")?;
        let root = dir.path().join("repo");
        fs::create_dir(&root)?;
        let base = dir.path().join("indexes");
        let store = Store::new(store::index_dir(&base, &root));
        store.create_dirs()?;
        store.write_meta(&store::Meta {
            extraction_version: 1,
            binary_version: "0.1.0+r1".to_owned(),
            root: root.to_string_lossy().into_owned(),
            write_time_ns: 1,
        })?;
        let path = store.dir.join("meta");
        let mut image = fs::read(&path)?;
        image[8..12].copy_from_slice(&8_u32.to_le_bytes());
        fs::write(&path, image)?;
        assert_eq!(known_meta(&store).map(|(_, version)| version), Some(8));
        assert_eq!(owned_root(&store.dir), Some(root));
        Ok(())
    }
}
