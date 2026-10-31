//! `ls` and `clean`: the indexes in the index base, for humans tidying up. Indexes are
//! disposable; deleting one only costs the next query a full build.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::error::AppError;
use super::workspace::{self, LOCAL_DIR};
use crate::cli::Options;
use crate::infra::store::{self, Store};

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

/// Deletes the indexes of the given roots (default: the current root), `--all` of every root,
/// `--missing` of roots that no longer exist; one line per deleted index.
pub fn clean(options: &Options, roots: &[String]) -> Result<String, AppError> {
    let base = base(options)?;
    let cwd = env::current_dir().map_err(AppError::io("current directory"))?;
    let targets: Vec<PathBuf> = if options.all || options.missing {
        entries(&base)
            .into_iter()
            .filter(|entry| options.all || entry.root.as_ref().is_none_or(|root| !root.is_dir()))
            .map(|entry| entry.dir)
            .collect()
    } else {
        let wanted: Vec<PathBuf> = if roots.is_empty() {
            vec![workspace::detect_root(&cwd)]
        } else {
            roots.iter().map(|root| cwd.join(root)).collect()
        };
        let mut dirs = Vec::new();
        for root in wanted {
            let canonical = root.canonicalize().unwrap_or(root);
            dirs.push(store::index_dir(&base, &canonical));
            // An index the root holds itself (confined agents) goes too.
            let local = canonical.join(LOCAL_DIR);
            if local.is_dir() {
                dirs.push(local);
            }
        }
        dirs
    };
    let mut out = String::new();
    for dir in targets {
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
                root: store.read_meta().ok().map(|meta| PathBuf::from(meta.root)),
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
        .map(|entry| match entry.metadata() {
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
}
