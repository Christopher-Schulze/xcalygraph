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
    /// Implicit shared base, retained when using the local fallback.
    pub default_base: Option<PathBuf>,
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
        let base = index_base(options.index_dir.as_deref(), &cwd);
        let default_base = (options.index_dir.is_none()
            && env::var_os("XCALYGRAPH_INDEX_DIR").is_none_or(|value| value.is_empty()))
        .then(|| base.clone());
        Ok(Self {
            store: Store::new(store::index_dir(&base, &root)),
            root,
            default_base,
        })
    }

    /// Creates the implicit shared base only when the command actually needs to write there.
    pub fn prepare_base(&self) -> Result<(), AppError> {
        if let Some(base) = &self.default_base
            && self.store.dir == store::index_dir(base, &self.root)
        {
            create_private(base).map_err(AppError::index_io("create shared index base"))?;
        }
        Ok(())
    }

    /// Local index for an implicit shared location; explicit locations have no fallback.
    pub fn local_fallback(&self) -> Option<Self> {
        let default_base = self.default_base.as_ref()?;
        let local = store::index_dir(&self.root.join(LOCAL_DIR), &self.root);
        (self.store.dir != local).then(|| Self {
            root: self.root.clone(),
            store: Store::new(local),
            default_base: Some(default_base.clone()),
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
        if let Some(base) = &self.default_base {
            for path in [base.clone(), self.root.join(LOCAL_DIR)] {
                if path.starts_with(&self.root) && path != self.root && !excluded.contains(&path) {
                    excluded.push(path);
                }
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
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(base)
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
            default_base: None,
        };
        assert_eq!(
            workspace.excluded(),
            [workspace.store.dir.clone(), root.join(".cache/index")]
        );
        let outside = Workspace {
            store: Store::new(PathBuf::from("/elsewhere/x")),
            root,
            default_base: None,
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

    #[test]
    fn both_implicit_index_locations_are_excluded_from_both_walks() -> std::io::Result<()> {
        let dir = TestDir::new("workspace-default-excluded")?;
        let root = dir.path().to_path_buf();
        let base = root.join("tmp/shared");
        let shared = Workspace {
            store: Store::new(store::index_dir(&base, &root)),
            root: root.clone(),
            default_base: Some(base.clone()),
        };
        let local = shared
            .local_fallback()
            .ok_or_else(|| std::io::Error::other("missing fallback"))?;
        for workspace in [&shared, &local] {
            assert!(workspace.excluded().contains(&base));
            assert!(workspace.excluded().contains(&root.join(LOCAL_DIR)));
        }
        assert!(local.local_fallback().is_none());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn new_base_is_private_and_existing_permissions_are_preserved() -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = TestDir::new("workspace-private-base")?;
        let base = dir.path().join("base");
        create_private(&base)?;
        assert_eq!(std::fs::metadata(&base)?.permissions().mode() & 0o077, 0);
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o750))?;
        create_private(&base)?;
        assert_eq!(
            std::fs::metadata(&base)?.permissions().mode() & 0o777,
            0o750
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn colliding_probe_file_is_never_replaced_or_removed() -> std::io::Result<()> {
        use std::os::unix::fs::MetadataExt;

        let dir = TestDir::new("workspace-probe-file")?;
        let base = dir.path().join("base");
        std::fs::create_dir(&base)?;
        let probe = base.join(format!(".probe.{}", std::process::id()));
        std::fs::write(&probe, b"preserve")?;
        let before = std::fs::metadata(&probe)?;
        create_private(&base)?;
        let after = std::fs::metadata(&probe)?;
        assert_eq!(std::fs::read(&probe)?, b"preserve");
        assert_eq!(
            (after.ino(), after.modified()?),
            (before.ino(), before.modified()?)
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn colliding_probe_symlink_and_target_are_never_changed() -> std::io::Result<()> {
        use std::os::unix::fs::{MetadataExt, symlink};

        let dir = TestDir::new("workspace-probe-link")?;
        let base = dir.path().join("base");
        std::fs::create_dir(&base)?;
        let target = dir.path().join("target");
        std::fs::write(&target, b"preserve")?;
        let probe = base.join(format!(".probe.{}", std::process::id()));
        symlink(&target, &probe)?;
        let before_link = std::fs::symlink_metadata(&probe)?;
        let before_target = std::fs::metadata(&target)?;
        create_private(&base)?;
        let after_link = std::fs::symlink_metadata(&probe)?;
        let after_target = std::fs::metadata(&target)?;
        assert!(after_link.file_type().is_symlink());
        assert_eq!(after_link.ino(), before_link.ino());
        assert_eq!(std::fs::read(&target)?, b"preserve");
        assert_eq!(after_target.modified()?, before_target.modified()?);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn existing_file_at_index_base_is_not_replaced() -> std::io::Result<()> {
        let dir = TestDir::new("workspace-base-file")?;
        let base = dir.path().join("base");
        std::fs::write(&base, b"preserve")?;
        assert!(create_private(&base).is_err());
        assert_eq!(std::fs::read(&base)?, b"preserve");
        Ok(())
    }

    #[test]
    fn concurrent_base_creation_does_not_interfere() -> std::io::Result<()> {
        let dir = TestDir::new("workspace-probe-concurrent")?;
        let base = dir.path().join("base");
        let barrier = std::sync::Barrier::new(32);
        std::thread::scope(|scope| {
            let handles = (0..32)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        create_private(&base)
                    })
                })
                .collect::<Vec<_>>();
            for handle in handles {
                handle
                    .join()
                    .map_err(|_| std::io::Error::other("base creation thread panicked"))??;
            }
            Ok::<(), std::io::Error>(())
        })?;
        assert_eq!(std::fs::read_dir(&base)?.count(), 0);
        Ok(())
    }

    #[test]
    fn exhausted_probe_names_leave_all_existing_files_untouched() -> std::io::Result<()> {
        let dir = TestDir::new("workspace-probe-exhausted")?;
        let base = dir.path().join("base");
        std::fs::create_dir(&base)?;
        for attempt in 0..128 {
            let name = if attempt == 0 {
                format!(".probe.{}", std::process::id())
            } else {
                format!(".probe.{}.{attempt}", std::process::id())
            };
            std::fs::write(base.join(name), b"preserve")?;
        }
        create_private(&base)?;
        for entry in std::fs::read_dir(&base)? {
            assert_eq!(std::fs::read(entry?.path())?, b"preserve");
        }
        assert_eq!(std::fs::read_dir(&base)?.count(), 128);
        Ok(())
    }
}
