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
        let mut base = index_base(options.index_dir.as_deref(), &cwd);
        // Agents confined to their repository cannot write the shared location: the index then
        // lives in the root, excluded from the walk.
        if options.index_dir.is_none()
            && env::var_os("XCALYGRAPH_INDEX_DIR").is_none_or(|value| value.is_empty())
        {
            let local = root.join(LOCAL_DIR);
            if options.read_only {
                // Read-only location discovery must not create the base or a writability probe.
                if !store::index_dir(&base, &root).is_dir()
                    && store::index_dir(&local, &root).is_dir()
                {
                    base = local;
                }
            } else if create_private(&base).is_err() {
                base = local;
                eprintln!(
                    "xcalygraph: index in {} (shared location not writable); add {LOCAL_DIR}/ to .gitignore",
                    base.display()
                );
            }
        }
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

/// Index directory inside the root when the shared location is not writable.
pub const LOCAL_DIR: &str = ".xcalygraph";

/// Index base: `--index-dir`, else `$XCALYGRAPH_INDEX_DIR`, else a per-user directory in the
/// system temp directory. Indexes are disposable: a full build takes seconds, so the operating
/// system's temp cleanup (reboot, unused files) bounds their number without a cleanup policy.
pub fn index_base(flag: Option<&Path>, cwd: &Path) -> PathBuf {
    if let Some(dir) = flag {
        return absolute(cwd, dir);
    }
    if let Some(dir) = env::var_os("XCALYGRAPH_INDEX_DIR").filter(|value| !value.is_empty()) {
        return absolute(cwd, &PathBuf::from(dir));
    }
    let user = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|name| env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "user".to_owned());
    absolute(cwd, &env::temp_dir()).join(format!("xcalygraph-{user}"))
}

/// Creates the index base readable by its owner only (the temp directory may be shared).
fn create_private(base: &Path) -> std::io::Result<()> {
    if base.is_dir() {
        // Writable check without leaving anything behind.
        let probe = base.join(format!(".probe.{}", std::process::id()));
        std::fs::write(&probe, b"")?;
        return std::fs::remove_file(&probe);
    }
    std::fs::create_dir_all(base)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(base, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
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
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR"));
        let expected = if plain.starts_with(checkout) && checkout.join(".git").exists() {
            checkout
        } else {
            plain.as_path()
        };
        assert_eq!(detect_root(&plain), expected);
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
        assert_eq!(base, dir.path().join("idx/new"));
        Ok(())
    }
}
