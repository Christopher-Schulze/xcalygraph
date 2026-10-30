//! Git through the `git` CLI (`docs/decisions.md` D12): changed line ranges and file content at
//! a revision. Every function returns `None` when git or the work tree is unavailable.

use std::path::Path;
use std::process::Command;

/// Changed line ranges of one file in the new version; an empty range list means the whole
/// file is new (untracked).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Root-relative path.
    pub path: String,
    /// Inclusive new-side line ranges; a pure deletion is the line after which it happened.
    pub ranges: Vec<(u32, u32)>,
    pub untracked: bool,
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Path of the root inside the work tree (`""` at the top level), with a trailing `/`.
fn prefix(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "--show-prefix"]).map(|text| text.trim().to_owned())
}

/// Changes of the work tree against `rev` (default `HEAD`), plus untracked files.
pub fn diff(root: &Path, rev: &str) -> Option<Vec<FileDiff>> {
    let prefix = prefix(root)?;
    let text = git(
        root,
        &[
            "diff",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            rev,
            "--",
            ".",
        ],
    )?;
    let mut diffs = parse_diff(&text, &prefix);
    if let Some(untracked) = git(
        root,
        &["ls-files", "--others", "--exclude-standard", "--", "."],
    ) {
        for path in untracked.lines().filter(|line| !line.is_empty()) {
            diffs.push(FileDiff {
                path: path.to_owned(),
                ranges: Vec::new(),
                untracked: true,
            });
        }
    }
    diffs.sort_by(|a, b| a.path.cmp(&b.path));
    Some(diffs)
}

/// Parses `git diff --unified=0`; paths are made relative to the root via `prefix`.
pub fn parse_diff(text: &str, prefix: &str) -> Vec<FileDiff> {
    let mut diffs: Vec<FileDiff> = Vec::new();
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            let path = path.strip_prefix("b/").unwrap_or(path);
            if path == "/dev/null" {
                continue;
            }
            let relative = path.strip_prefix(prefix).unwrap_or(path);
            diffs.push(FileDiff {
                path: relative.to_owned(),
                ranges: Vec::new(),
                untracked: false,
            });
        } else if let Some(hunk) = line.strip_prefix("@@ ") {
            let Some(current) = diffs.last_mut() else {
                continue;
            };
            let Some(new_side) = hunk.split_whitespace().find(|part| part.starts_with('+')) else {
                continue;
            };
            let mut numbers = new_side.trim_start_matches('+').split(',');
            let start: u32 = numbers.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            let count: u32 = numbers.next().and_then(|n| n.parse().ok()).unwrap_or(1);
            let range = if count == 0 {
                (start.max(1), start.max(1))
            } else {
                (start, start + count - 1)
            };
            current.ranges.push(range);
        }
    }
    diffs.retain(|diff| !diff.ranges.is_empty());
    diffs
}

/// Content of a root-relative file at `rev`; `None` when it did not exist there.
pub fn show(root: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    let prefix = prefix(root)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &format!("{rev}:{prefix}{path}")])
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_zero_hunks_become_new_side_ranges() {
        let text = "diff --git a/sub/a.go b/sub/a.go\n--- a/sub/a.go\n+++ b/sub/a.go\n@@ -3,0 +4,2 @@ func x\n+a\n+b\n@@ -10 +12 @@\n-x\n+y\n@@ -20,3 +23,0 @@\n-gone\ndiff --git a/sub/old.go b/sub/old.go\n--- a/sub/old.go\n+++ /dev/null\n@@ -1,3 +0,0 @@\n";
        let diffs = parse_diff(text, "sub/");
        assert_eq!(
            diffs,
            [FileDiff {
                path: "a.go".to_owned(),
                ranges: vec![(4, 5), (12, 12), (23, 23)],
                untracked: false,
            }]
        );
    }
}
