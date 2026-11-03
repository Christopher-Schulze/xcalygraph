//! `install` and the version check of the embedded skill (`docs/spec.md` section 10).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::error::AppError;
use super::progress::Progress;

pub const SKILL: &str = include_str!("../../skills/xcalygraph/SKILL.md");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Install target of the skill below `home`.
pub fn skill_dir(home: &Path) -> PathBuf {
    home.join(".agents").join("skills").join("xcalygraph")
}

pub fn claude_link(home: &Path) -> PathBuf {
    home.join(".claude").join("skills").join("xcalygraph")
}

pub fn home() -> Result<PathBuf, AppError> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|name| std::env::var_os(name).filter(|home| !home.is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| AppError::Invalid("HOME is not set".to_owned()))
}

/// File name of the binary on this platform (`xcalygraph.exe` on Windows).
pub fn binary_name() -> String {
    format!("xcalygraph{}", std::env::consts::EXE_SUFFIX)
}

/// Version of the installed skill, if any.
pub fn installed_version(home: &Path) -> Option<String> {
    fs::read_to_string(skill_dir(home).join(".version"))
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// One notice line when the installed skill stamp or bytes differ from this binary.
pub fn mismatch_line(home: &Path) -> String {
    let Some(installed) = installed_version(home) else {
        return String::new();
    };
    if installed != VERSION {
        return format!(
            "! installed skill {installed}, binary {VERSION}; run 'xcalygraph install'\n"
        );
    }
    if fs::read(skill_dir(home).join("SKILL.md"))
        .is_ok_and(|content| content.as_slice() == SKILL.as_bytes())
    {
        return String::new();
    }
    format!("! installed skill content differs from binary {VERSION}; run 'xcalygraph install'\n")
}

/// What `install` puts in place.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Copy the running executable to the install directory.
    pub binary: bool,
    /// Link the skill for Claude Code when `~/.claude` exists.
    pub link: bool,
}

/// Install directory of the binary: `$XCALYGRAPH_INSTALL_DIR`, else `~/.local/bin`.
pub fn install_dir(home: &Path) -> PathBuf {
    std::env::var_os("XCALYGRAPH_INSTALL_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| home.join(".local").join("bin"), PathBuf::from)
}

/// Installs the running binary, the embedded skill with its version stamp and the Claude Code
/// link, each as one progress phase; returns one result line per artifact plus `!` notices.
pub fn install(home: &Path, options: Options, progress: &mut Progress) -> Result<String, AppError> {
    let mut out = String::new();
    if options.binary {
        progress.start("Binary");
        let line = install_binary(home).inspect_err(|_| progress.fail("not installed"))?;
        progress.pass(line.trim_end().trim_start_matches("binary "));
        out.push_str(&line);
        out.push_str(&path_notice(&install_dir(home)));
    }
    progress.start("Skill");
    let line = install_skill(home).inspect_err(|_| progress.fail("not written"))?;
    progress.pass(line.trim_end().trim_start_matches("skill "));
    out.push_str(&line);
    if options.link && home.join(".claude").is_dir() {
        progress.start("Claude link");
        let line = link(&claude_link(home), &skill_dir(home))
            .inspect_err(|_| progress.fail("not linked"))?;
        if line.starts_with('!') {
            progress.fail(line.trim_end().trim_start_matches("! "));
        } else {
            progress.pass(line.trim_end().trim_start_matches("link "));
        }
        out.push_str(&line);
    }
    Ok(out)
}

/// Writes the embedded skill and its version stamp atomically.
pub fn install_skill(home: &Path) -> Result<String, AppError> {
    let dir = skill_dir(home);
    fs::create_dir_all(&dir).map_err(AppError::io(format!("create {}", dir.display())))?;
    let changed = write_if_changed(&dir.join("SKILL.md"), SKILL.as_bytes(), 0o644)
        .map_err(AppError::io("write skill"))?;
    let stamped = write_if_changed(
        &dir.join(".version"),
        format!("{VERSION}\n").as_bytes(),
        0o644,
    )
    .map_err(AppError::io("write skill version"))?;
    Ok(format!(
        "skill {VERSION} {} {}\n",
        dir.join("SKILL.md").display(),
        if changed || stamped {
            "written"
        } else {
            "unchanged"
        }
    ))
}

/// Copies the running executable to the install directory by temp file plus rename, unless it
/// already runs from there or the installed bytes are equal.
fn install_binary(home: &Path) -> Result<String, AppError> {
    let dir = install_dir(home);
    let target = dir.join(binary_name());
    let exe = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(AppError::io("locate the running binary"))?;
    if target
        .canonicalize()
        .is_ok_and(|installed| installed == exe)
    {
        return Ok(format!("binary {} unchanged\n", target.display()));
    }
    let bytes = fs::read(&exe).map_err(AppError::io(format!("read {}", exe.display())))?;
    fs::create_dir_all(&dir).map_err(AppError::io(format!("create {}", dir.display())))?;
    let changed = write_if_changed(&target, &bytes, 0o755)
        .map_err(AppError::io(format!("write {}", target.display())))?;
    #[cfg(unix)]
    if !changed {
        use std::os::unix::fs::PermissionsExt;
        let existing = fs::symlink_metadata(&target)
            .map_err(AppError::io(format!("inspect {}", target.display())))?;
        if existing.file_type().is_file() && existing.permissions().mode() & 0o111 != 0o111 {
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).map_err(
                AppError::io(format!("make {} executable", target.display())),
            )?;
        }
    }
    Ok(format!(
        "binary {} {}\n",
        target.display(),
        if changed { "installed" } else { "unchanged" }
    ))
}

/// A `!` line when `dir` is not on `PATH` or another `xcalygraph` comes first; shell files
/// are never edited.
fn path_notice(dir: &Path) -> String {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let entries: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let target = dir.join(binary_name());
    if let Some(first) = entries
        .iter()
        .map(|entry| entry.join(binary_name()))
        .find(|candidate| candidate.is_file())
        && first.canonicalize().ok() != target.canonicalize().ok()
    {
        return format!(
            "! {} shadows {}; put {} first on PATH\n",
            first.display(),
            target.display(),
            dir.display()
        );
    }
    if entries.iter().any(|entry| entry == dir) {
        String::new()
    } else {
        format!(
            "! {} is not on PATH; add: export PATH=\"{}:$PATH\"\n",
            dir.display(),
            dir.display()
        )
    }
}

fn link(link: &Path, target: &Path) -> Result<String, AppError> {
    match fs::symlink_metadata(link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let current =
                fs::read_link(link).map_err(AppError::io(format!("read {}", link.display())))?;
            if current == target {
                Ok(format!(
                    "link {} -> {} unchanged\n",
                    link.display(),
                    target.display()
                ))
            } else {
                Ok(format!(
                    "! {} points to {}; not replaced\n",
                    link.display(),
                    current.display()
                ))
            }
        }
        Ok(_) => Ok(format!(
            "! {} exists and is no link; not replaced\n",
            link.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = link.parent() {
                fs::create_dir_all(parent)
                    .map_err(AppError::io(format!("create {}", parent.display())))?;
            }
            symlink(target, link).map_err(AppError::io(format!("link {}", link.display())))?;
            Ok(format!(
                "link {} -> {} created\n",
                link.display(),
                target.display()
            ))
        }
        Err(error) => Err(AppError::Io(format!("inspect {}", link.display()), error)),
    }
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// Replaces the file by a new exclusive temp file plus rename unless its content is already equal.
pub fn write_if_changed(path: &Path, content: &[u8], mode: u32) -> io::Result<bool> {
    if fs::read(path).is_ok_and(|existing| existing == content) {
        return Ok(false);
    }
    crate::infra::file_access::replace_with(path, content, mode)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::testdir::TestDir;

    #[test]
    fn install_writes_skill_once_and_links_claude() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("install")?;
        let home = dir.path();
        let options = Options {
            binary: false,
            link: true,
        };
        fs::create_dir_all(home.join(".claude"))?;
        let first = install(home, options, &mut Progress::new(false, false))?;
        assert!(
            first.contains("written") && first.contains("created"),
            "{first}"
        );
        assert_eq!(fs::read_to_string(skill_dir(home).join("SKILL.md"))?, SKILL);
        assert_eq!(installed_version(home).as_deref(), Some(VERSION));
        assert_eq!(fs::read_link(claude_link(home))?, skill_dir(home));
        let second = install(home, options, &mut Progress::new(false, false))?;
        assert!(
            second.contains("skill") && second.contains(" unchanged\n") && second.contains("link"),
            "{second}"
        );
        assert!(mismatch_line(home).is_empty());
        fs::write(skill_dir(home).join(".version"), "0.0.1\n")?;
        assert_eq!(
            mismatch_line(home),
            format!("! installed skill 0.0.1, binary {VERSION}; run 'xcalygraph install'\n")
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn skill_install_never_writes_through_a_planted_temp_symlink()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("install-temp-link")?;
        let home = dir.path().join("home");
        let foreign = dir.write("foreign", "foreign")?;
        fs::create_dir_all(skill_dir(&home))?;
        let planted = skill_dir(&home).join(format!(".SKILL.md.tmp.{}.0", std::process::id()));
        std::os::unix::fs::symlink(&foreign, &planted)?;
        install_skill(&home)?;
        assert_eq!(
            fs::read_to_string(skill_dir(&home).join("SKILL.md"))?,
            SKILL
        );
        assert_eq!(fs::read_to_string(&foreign)?, "foreign");
        assert!(fs::symlink_metadata(&planted)?.file_type().is_symlink());
        Ok(())
    }

    #[test]
    fn foreign_claude_entry_is_never_replaced() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("install-foreign")?;
        let home = dir.path();
        fs::create_dir_all(claude_link(home))?;
        let options = Options {
            binary: false,
            link: true,
        };
        let out = install(home, options, &mut Progress::new(false, false))?;
        assert!(out.contains("exists and is no link; not replaced"), "{out}");
        assert!(claude_link(home).is_dir());
        Ok(())
    }

    #[test]
    fn claude_link_needs_an_existing_claude_directory() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("install-no-claude")?;
        let home = dir.path();
        let options = Options {
            binary: false,
            link: true,
        };
        let out = install(home, options, &mut Progress::new(false, false))?;
        assert!(out.contains("written") && !out.contains("link"), "{out}");
        assert!(fs::symlink_metadata(claude_link(home)).is_err());
        Ok(())
    }

    #[test]
    fn skill_stays_within_budget() {
        // docs/spec.md section 10: max 1000 tokens at 3 bytes per token.
        assert!(SKILL.len() <= 1000 * 3, "skill has {} bytes", SKILL.len());
        assert!(SKILL.starts_with("---\nname: xcalygraph\n"));
    }
}
