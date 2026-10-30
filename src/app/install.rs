//! `install` and the version check of the embedded skill (`docs/spec.md` section 10).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::error::AppError;

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
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| AppError::Invalid("HOME is not set".to_owned()))
}

/// Version of the installed skill, if any.
pub fn installed_version(home: &Path) -> Option<String> {
    fs::read_to_string(skill_dir(home).join(".version"))
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// One notice line when the installed skill differs from this binary; empty when equal or absent.
pub fn mismatch_line(home: &Path) -> String {
    match installed_version(home) {
        Some(installed) if installed != VERSION => {
            format!("! installed skill {installed}, binary {VERSION}; run 'xcalygraph install'\n")
        }
        _ => String::new(),
    }
}

/// Writes the embedded skill and its version stamp atomically; with `link_claude` also creates
/// the Claude Code skill link when absent.
pub fn install(home: &Path, link_claude: bool) -> Result<String, AppError> {
    let dir = skill_dir(home);
    fs::create_dir_all(&dir).map_err(AppError::io(format!("create {}", dir.display())))?;
    let changed = write_if_changed(&dir.join("SKILL.md"), SKILL.as_bytes())
        .map_err(AppError::io("write skill"))?;
    write_if_changed(&dir.join(".version"), format!("{VERSION}\n").as_bytes())
        .map_err(AppError::io("write skill version"))?;
    let mut out = format!(
        "skill {VERSION} {} {}\n",
        dir.join("SKILL.md").display(),
        if changed { "written" } else { "unchanged" }
    );
    if link_claude {
        out.push_str(&link(&claude_link(home), &dir)?);
    }
    Ok(out)
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

/// Replaces the file by temp file plus rename unless its content is already equal.
pub fn write_if_changed(path: &Path, content: &[u8]) -> io::Result<bool> {
    if fs::read(path).is_ok_and(|existing| existing == content) {
        return Ok(false);
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temp = path.with_file_name(format!(".{name}.tmp.{}", std::process::id()));
    fs::write(&temp, content)?;
    fs::rename(&temp, path).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })?;
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
        let first = install(home, true)?;
        assert!(
            first.contains("written") && first.contains("created"),
            "{first}"
        );
        assert_eq!(fs::read_to_string(skill_dir(home).join("SKILL.md"))?, SKILL);
        assert_eq!(installed_version(home).as_deref(), Some(VERSION));
        assert_eq!(fs::read_link(claude_link(home))?, skill_dir(home));
        let second = install(home, true)?;
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

    #[test]
    fn foreign_claude_entry_is_never_replaced() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("install-foreign")?;
        let home = dir.path();
        fs::create_dir_all(claude_link(home))?;
        let out = install(home, true)?;
        assert!(out.contains("exists and is no link; not replaced"), "{out}");
        assert!(claude_link(home).is_dir());
        Ok(())
    }

    #[test]
    fn skill_stays_within_budget() {
        // docs/spec.md section 10: max 1500 tokens at 3 bytes per token.
        assert!(SKILL.len() <= 1500 * 3, "skill has {} bytes", SKILL.len());
        assert!(SKILL.starts_with("---\nname: xcalygraph\n"));
    }
}
