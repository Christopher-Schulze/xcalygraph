//! Root and index directory resolution (`docs/spec.md` section 2, `-C` and `--index-dir`).

use std::env;
use std::path::{Path, PathBuf};

use super::error::AppError;
use crate::cli::Options;
use crate::infra::store::{self, Store};

/// The root being indexed and its index store.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Canonical root directory.
    pub root: PathBuf,
    pub store: Store,
}

impl Workspace {
    pub fn resolve(options: &Options) -> Result<Self, AppError> {
        let cwd = env::current_dir().map_err(AppError::io("current directory"))?;
        let root = match &options.root {
            Some(root) => cwd.join(root),
            None => detect_root(&cwd),
        };
        let root = root
            .canonicalize()
            .map_err(AppError::io(format!("root {}", root.display())))?;
        if !root.is_dir() {
            return Err(AppError::Invalid(format!(
                "root {} is not a directory",
                root.display()
            )));
        }
        let base = index_base(options.index_dir.as_deref(), &cwd).ok_or_else(|| {
            AppError::Invalid("no index directory: set --index-dir or HOME".to_owned())
        })?;
        Ok(Self {
            store: Store::new(store::index_dir(&base, &root)),
            root,
        })
    }

    /// Directories inside the root that the walk must skip (the index itself).
    pub fn excluded(&self) -> Vec<PathBuf> {
        let mut excluded = Vec::new();
        if self.store.dir.starts_with(&self.root) {
            excluded.push(self.store.dir.clone());
            if let Some(base) = self.store.dir.parent()
                && base.starts_with(&self.root)
                && base != self.root
            {
                excluded.push(base.to_path_buf());
            }
        }
        excluded
    }
}

/// Nearest ancestor of `cwd` containing `.git`, else `cwd`.
pub fn detect_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

fn index_base(flag: Option<&Path>, cwd: &Path) -> Option<PathBuf> {
    if let Some(dir) = flag {
        return Some(absolute(cwd, dir));
    }
    let from_env = |name: &str| {
        env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = from_env("XCALYGRAPH_INDEX_DIR") {
        return Some(absolute(cwd, &dir));
    }
    if let Some(cache) = from_env("XDG_CACHE_HOME") {
        return Some(absolute(cwd, &cache).join("xcalygraph"));
    }
    from_env("HOME").map(|home| home.join(".cache").join("xcalygraph"))
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    let joined = cwd.join(path);
    // Canonicalize the existing part so prefix checks against the canonical root hold.
    let mut existing = joined.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return joined;
        };
        rest.push(name.to_os_string());
        existing = parent;
    }
    let mut base = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for name in rest.into_iter().rev() {
        base.push(name);
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::testdir::TestDir;

    #[test]
    fn root_is_nearest_git_ancestor() -> std::io::Result<()> {
        let dir = TestDir::new("workspace")?;
        dir.write("repo/.git/HEAD", "ref")?;
        dir.write("repo/src/deep/x.go", "")?;
        let deep = dir.path().join("repo/src/deep");
        assert_eq!(detect_root(&deep), dir.path().join("repo"));
        let plain = dir.path().join("repo2");
        std::fs::create_dir_all(&plain)?;
        assert_eq!(detect_root(&plain).file_name(), plain.file_name());
        Ok(())
    }

    #[test]
    fn index_inside_root_is_excluded() -> std::io::Result<()> {
        let dir = TestDir::new("workspace-excluded")?;
        let root = dir.path().to_path_buf();
        let workspace = Workspace {
            store: Store::new(store::index_dir(&root.join(".cache/index"), &root)),
            root: root.clone(),
        };
        assert_eq!(
            workspace.excluded(),
            [workspace.store.dir.clone(), root.join(".cache/index")]
        );
        let outside = Workspace {
            store: Store::new(PathBuf::from("/elsewhere/x")),
            root,
        };
        assert!(outside.excluded().is_empty());
        Ok(())
    }

    #[test]
    fn flag_wins_and_relative_paths_are_absolute() -> std::io::Result<()> {
        let dir = TestDir::new("workspace-base")?;
        let base = index_base(Some(Path::new("idx/new")), dir.path());
        assert_eq!(base, Some(dir.path().join("idx/new")));
        Ok(())
    }
}
