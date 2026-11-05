//! Parallel directory walk with ignore rules and pruning (`docs/spec.md` section 3).

use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::domain::corpus::{FileIdentity, Stat};

/// Directory names never descended into.
pub(super) const PRUNED_NAMES: [&str; 6] = [
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "__pycache__",
    "graphify-out",
];

/// Walks `root` in parallel and returns its files sorted by path. `excluded` directories (for
/// example an index directory inside the root) are pruned.
pub fn walk(root: &Path, excluded: &[PathBuf]) -> io::Result<Vec<Stat>> {
    #[cfg(target_os = "macos")]
    if let Some(files) = super::walk_bulk::walk(root, excluded) {
        return Ok(files);
    }
    walk_portable(root, excluded)
}

fn walk_portable(root: &Path, excluded: &[PathBuf]) -> io::Result<Vec<Stat>> {
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
    let failure: Mutex<Option<io::Error>> = Mutex::new(None);
    builder.build_parallel().run(|| {
        let failure = &failure;
        let mut collector = Collector {
            local: Vec::new(),
            shared: &found,
        };
        Box::new(move |result| {
            let file = match result {
                // A rule file that cannot be read costs its rules, not the walk.
                Ok(entry) => match entry.error().filter(|error| !is_denied(error)) {
                    Some(error) => Err(io::Error::other(error.to_string())),
                    None => walked_file(root, &entry),
                },
                Err(error) if is_denied(&error) => Ok(denied_folder(root, &error)),
                Err(error) => Err(io::Error::other(error)),
            };
            match file {
                Ok(Some(file)) => collector.push(file),
                Ok(None) => {}
                Err(error) => {
                    let mut first = failure
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if first.is_none() {
                        *first = Some(error);
                    }
                    return WalkState::Quit;
                }
            }
            WalkState::Continue
        })
    });
    if let Some(error) = failure
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
    {
        return Err(error);
    }
    let mut files = found
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    files.dedup_by(|left, right| left.path == right.path);
    Ok(files)
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

fn is_denied(error: &ignore::Error) -> bool {
    error
        .io_error()
        .is_some_and(|error| error.kind() == io::ErrorKind::PermissionDenied)
}

/// A folder the walk may not enter stays visible as `path/`, an entry without content, so an
/// answer can say what it did not see.
fn denied_folder(root: &Path, error: &ignore::Error) -> Option<Stat> {
    let mut current = error;
    let path = loop {
        match current {
            ignore::Error::WithPath { path, .. } => break path,
            ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
                current = err;
            }
            _ => return None,
        }
    };
    let relative = path.strip_prefix(root).ok()?;
    Some(Stat {
        path: format!("{}/", relative_path(relative)?),
        size: 0,
        mtime_ns: 0,
        identity: None,
    })
}

fn walked_file(root: &Path, entry: &DirEntry) -> io::Result<Option<Stat>> {
    if !entry.file_type().is_some_and(|kind| kind.is_file()) {
        return Ok(None);
    }
    let relative = entry.path().strip_prefix(root).map_err(io::Error::other)?;
    let Some(path) = relative_path(relative) else {
        return Ok(None);
    };
    let metadata = entry.metadata().map_err(io::Error::other)?;
    Ok(Some(Stat {
        path,
        size: metadata.len(),
        mtime_ns: mtime_ns(&metadata),
        identity: identity(&metadata),
    }))
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

fn identity(metadata: &Metadata) -> Option<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let inode = metadata.ino();
        let nanos = metadata.ctime_nsec();
        let change_time_ns = metadata
            .ctime()
            .checked_mul(1_000_000_000)?
            .checked_add(nanos)?;
        (inode != 0 && change_time_ns != 0 && (0..1_000_000_000).contains(&nanos)).then_some(
            FileIdentity {
                device: metadata.dev(),
                inode,
                change_time_ns,
            },
        )
    }
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
        let files = walk(dir.path(), &[])?;
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

    /// Both routes see the same files with the same stat records, the identity included.
    #[cfg(target_os = "macos")]
    #[test]
    fn bulk_walk_agrees_with_the_portable_walker() -> std::io::Result<()> {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = TestDir::new("walk-routes")?;
        dir.write(".gitignore", "*.log\nbuild/\n!keep.log\n/anchored.go\n")?;
        dir.write(".xcalygraphignore", "keep.log\nsecret/\n")?;
        for path in [
            "a.log",
            "keep.log",
            "build/out.go",
            "secret/x.go",
            "anchored.go",
            "sub/anchored.go",
            "sub/local.go",
            "sub/kept.go",
            ".hidden/visible.go",
            "main.go",
            "ünï/cöde.go",
            "cache/CACHEDIR.TAG",
            "cache/x.go",
            "venv/pyvenv.cfg",
            "venv/y.go",
            "gomod/go.mod",
            "gomod/vendor/modules.txt",
            "gomod/vendor/v.go",
            "other/vendor/o.go",
            "node_modules/m.js",
            ".git/info/exclude",
            "excluded/e.go",
            "sub/deep/er/z.go",
            "open/visible.go",
        ] {
            dir.write(path, "x")?;
        }
        dir.write("sub/.gitignore", "local.go\n!a.log\n")?;
        dir.write(".git/info/exclude", "only-excluded.go\n")?;
        dir.write("only-excluded.go", "")?;
        symlink(dir.path().join("main.go"), dir.path().join("link.go"))?;
        std::fs::create_dir(dir.path().join("closed"))?;
        std::fs::set_permissions(
            dir.path().join("closed"),
            std::fs::Permissions::from_mode(0o000),
        )?;
        let excluded = [dir.path().join("excluded")];
        let bulk = crate::infra::walk_bulk::walk(dir.path(), &excluded)
            .ok_or_else(|| std::io::Error::other("bulk route declined"))?;
        let portable = walk_portable(dir.path(), &excluded)?;
        std::fs::set_permissions(
            dir.path().join("closed"),
            std::fs::Permissions::from_mode(0o700),
        )?;
        assert_eq!(bulk, portable);
        assert!(bulk.iter().any(|file| file.path == "closed/"));
        Ok(())
    }

    /// A `.git` file (linked work tree) goes to the portable walker.
    #[cfg(target_os = "macos")]
    #[test]
    fn bulk_walk_declines_a_linked_work_tree() -> std::io::Result<()> {
        let dir = TestDir::new("walk-linked")?;
        dir.write(".git", "gitdir: elsewhere\n")?;
        dir.write("main.go", "")?;
        assert!(crate::infra::walk_bulk::walk(dir.path(), &[]).is_none());
        Ok(())
    }

    /// Compares both routes on a real tree: `XCALYGRAPH_WALK_ROOT=path`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "development tool; needs XCALYGRAPH_WALK_ROOT"]
    fn walk_routes_agree_on_a_tree() -> std::io::Result<()> {
        let root = PathBuf::from(std::env::var_os("XCALYGRAPH_WALK_ROOT").unwrap_or_default());
        let bulk = crate::infra::walk_bulk::walk(&root, &[])
            .ok_or_else(|| std::io::Error::other("bulk route declined"))?;
        let portable = walk_portable(&root, &[])?;
        let first = bulk
            .iter()
            .zip(&portable)
            .find(|(left, right)| left != right);
        assert_eq!(first, None, "{root:?}");
        assert_eq!(bulk.len(), portable.len(), "{root:?}");
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
        let files = walk(dir.path(), &[dir.path().join("index")])?;
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
        assert_eq!(paths(&walk(dir.path(), &[])?), ["real/a.go"]);
        Ok(())
    }

    #[test]
    fn records_size_and_mtime() -> std::io::Result<()> {
        let dir = TestDir::new("walk-stat")?;
        dir.write("a.txt", "hello")?;
        let files = walk(dir.path(), &[])?;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].size, 5);
        assert!(files[0].mtime_ns > 0);
        Ok(())
    }

    #[test]
    fn a_failed_walk_is_reported_instead_of_returning_an_empty_corpus() -> io::Result<()> {
        let dir = TestDir::new("walk-error")?;
        assert!(walk(&dir.path().join("missing"), &[]).is_err());
        Ok(())
    }
}
