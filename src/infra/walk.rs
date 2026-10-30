//! Parallel directory walk with ignore rules and pruning (`docs/spec.md` section 3).

use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::domain::corpus::Stat;

/// Directory names never descended into.
const PRUNED_NAMES: [&str; 6] = [
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "__pycache__",
    "graphify-out",
];

/// Walks `root` in parallel and returns its files sorted by path. `excluded` directories (for
/// example an index directory inside the root) are pruned.
pub fn walk(root: &Path, excluded: &[PathBuf]) -> Vec<Stat> {
    let excluded: Arc<Vec<PathBuf>> = Arc::new(excluded.to_vec());
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .parents(true)
        .git_global(true)
        .git_exclude(true)
        .git_ignore(true)
        .require_git(false)
        .add_custom_ignore_filename(".xcalygraphignore");
    builder.current_dir(root.to_path_buf());
    builder.filter_entry(move |entry| keep(entry, &excluded));
    let found: Mutex<Vec<Stat>> = Mutex::new(Vec::new());
    builder.build_parallel().run(|| {
        let mut collector = Collector {
            local: Vec::new(),
            shared: &found,
        };
        Box::new(move |result| {
            if let Ok(entry) = result
                && let Some(file) = walked_file(root, &entry)
            {
                collector.push(file);
            }
            WalkState::Continue
        })
    });
    let mut files = found
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    files.dedup_by(|left, right| left.path == right.path);
    files
}

/// Per-thread buffer that moves its files into the shared list when full and when dropped.
struct Collector<'a> {
    local: Vec<Stat>,
    shared: &'a Mutex<Vec<Stat>>,
}

impl Collector<'_> {
    fn push(&mut self, file: Stat) {
        self.local.push(file);
        if self.local.len() >= 1024 {
            self.flush();
        }
    }

    fn flush(&mut self) {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        shared.append(&mut self.local);
    }
}

impl Drop for Collector<'_> {
    fn drop(&mut self) {
        self.flush();
    }
}

fn keep(entry: &DirEntry, excluded: &[PathBuf]) -> bool {
    let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
    if !is_dir || entry.depth() == 0 {
        return true;
    }
    let path = entry.path();
    let name = entry.file_name().to_string_lossy();
    if PRUNED_NAMES.contains(&name.as_ref()) {
        return false;
    }
    if path.join("CACHEDIR.TAG").exists() || path.join("pyvenv.cfg").exists() {
        return false;
    }
    if name == "vendor"
        && path.join("modules.txt").is_file()
        && path
            .parent()
            .is_some_and(|parent| parent.join("go.mod").is_file())
    {
        return false;
    }
    !excluded.iter().any(|dir| dir == path)
}

fn walked_file(root: &Path, entry: &DirEntry) -> Option<Stat> {
    if !entry.file_type()?.is_file() {
        return None;
    }
    let relative = entry.path().strip_prefix(root).ok()?;
    let path = relative_path(relative)?;
    let metadata = entry.metadata().ok()?;
    Some(Stat {
        path,
        size: metadata.len(),
        mtime_ns: mtime_ns(&metadata),
    })
}

/// `/`-separated UTF-8 path; non-UTF-8 paths are skipped because output must stay text.
fn relative_path(relative: &Path) -> Option<String> {
    let mut path = String::new();
    for component in relative.components() {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(component.as_os_str().to_str()?);
    }
    (!path.is_empty()).then_some(path)
}

pub fn mtime_ns(metadata: &Metadata) -> i64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata
            .mtime()
            .saturating_mul(1_000_000_000)
            .saturating_add(metadata.mtime_nsec())
    }
    #[cfg(not(unix))]
    {
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |duration| {
                i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::testdir::TestDir;

    fn paths(files: &[Stat]) -> Vec<&str> {
        files.iter().map(|file| file.path.as_str()).collect()
    }

    #[test]
    fn ignore_rules_apply_in_precedence_order() -> std::io::Result<()> {
        let dir = TestDir::new("walk-ignore")?;
        dir.write(".gitignore", "*.log\nbuild/\n!keep.log\n")?;
        dir.write(".xcalygraphignore", "keep.log\nsecret/\n")?;
        dir.write("a.log", "")?;
        dir.write("keep.log", "")?;
        dir.write("build/out.go", "")?;
        dir.write("secret/x.go", "")?;
        dir.write("sub/.gitignore", "local.go\n")?;
        dir.write("sub/local.go", "")?;
        dir.write("sub/kept.go", "")?;
        dir.write(".hidden/visible.go", "")?;
        dir.write("main.go", "")?;
        let files = walk(dir.path(), &[]);
        assert_eq!(
            paths(&files),
            [
                ".gitignore",
                ".hidden/visible.go",
                ".xcalygraphignore",
                "main.go",
                "sub/.gitignore",
                "sub/kept.go"
            ]
        );
        Ok(())
    }

    #[test]
    fn pruned_directories_are_skipped() -> std::io::Result<()> {
        let dir = TestDir::new("walk-prune")?;
        for path in [
            ".git/config",
            "node_modules/x/index.js",
            "py/__pycache__/a.pyc",
            "graphify-out/graph.json",
            "target/CACHEDIR.TAG",
            "target/debug/x",
            "venv/pyvenv.cfg",
            "venv/lib/site.py",
            "vendor/modules.txt",
            "vendor/dep/dep.go",
            "index/files",
            "go.mod",
            "app.go",
        ] {
            dir.write(path, "x")?;
        }
        let files = walk(dir.path(), &[dir.path().join("index")]);
        assert_eq!(paths(&files), ["app.go", "go.mod"]);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() -> std::io::Result<()> {
        let dir = TestDir::new("walk-symlink")?;
        dir.write("real/a.go", "x")?;
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link"))?;
        std::os::unix::fs::symlink(dir.path().join("real/a.go"), dir.path().join("b.go"))?;
        assert_eq!(paths(&walk(dir.path(), &[])), ["real/a.go"]);
        Ok(())
    }

    #[test]
    fn records_size_and_mtime() -> std::io::Result<()> {
        let dir = TestDir::new("walk-stat")?;
        dir.write("a.txt", "hello")?;
        let files = walk(dir.path(), &[]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].size, 5);
        assert!(files[0].mtime_ns > 0);
        Ok(())
    }
}
