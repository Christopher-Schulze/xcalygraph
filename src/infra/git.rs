//! Git through the `git` CLI (`docs/decisions.md` D12): changed line ranges and file content at
//! a revision. Every function returns `None` when git or the work tree is unavailable.

use std::collections::HashMap;
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
    /// The file existed at the revision and is gone from the work tree; `ranges` is empty.
    pub deleted: bool,
}

fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Path of the root inside the work tree (`""` at the top level), with a trailing `/`.
fn prefix(root: &Path) -> Option<String> {
    let text = String::from_utf8(git(root, &["rev-parse", "--show-prefix"])?).ok()?;
    Some(text.strip_suffix('\n')?.to_owned())
}

/// Changes of the work tree against `rev` (default `HEAD`), plus untracked files.
pub fn diff(root: &Path, rev: &str) -> Option<Vec<FileDiff>> {
    let prefix = prefix(root)?;
    let output = git(
        root,
        &[
            "diff",
            "--patch-with-raw",
            "-z",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--no-relative",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            rev,
            "--",
            ".",
        ],
    )?;
    let mut diffs = parse_diff(&output, &prefix)?;
    let untracked = git(
        root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
    )?;
    for path in untracked.split(|&byte| byte == 0) {
        if let Ok(path) = std::str::from_utf8(path)
            && !path.is_empty()
        {
            diffs.push(FileDiff {
                path: path.to_owned(),
                ranges: Vec::new(),
                untracked: true,
                deleted: false,
            });
        }
    }
    diffs.sort_by(|a, b| a.path.cmp(&b.path));
    Some(diffs)
}

/// Git's similarity-based file rename candidates, restricted to the queried root.
pub fn renames(root: &Path, rev: &str) -> Option<Vec<(String, String)>> {
    let prefix = prefix(root)?;
    let output = git(
        root,
        &[
            "diff",
            "--name-status",
            "-z",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "--no-relative",
            rev,
            "--",
            ".",
        ],
    )?;
    let mut fields = output.split(|&byte| byte == 0);
    let mut renames = Vec::new();
    while let Some(status) = fields.next() {
        if status.is_empty() {
            break;
        }
        let old = fields.next()?;
        if status.first() == Some(&b'R') {
            let new = fields.next()?;
            if let (Some(old), Some(new)) = (
                old.strip_prefix(prefix.as_bytes()),
                new.strip_prefix(prefix.as_bytes()),
            ) && let (Ok(old), Ok(new)) = (std::str::from_utf8(old), std::str::from_utf8(new))
            {
                renames.push((old.to_owned(), new.to_owned()));
            }
        } else if !matches!(status, b"A" | b"M" | b"D" | b"T") {
            return None;
        }
    }
    Some(renames)
}

/// Read NUL-delimited raw paths and associate patch hunks by their decoded Git path.
fn parse_diff(output: &[u8], prefix: &str) -> Option<Vec<FileDiff>> {
    if output.is_empty() {
        return Some(Vec::new());
    }
    let boundary = output.windows(2).position(|bytes| bytes == [0, 0])?;
    let mut fields = output[..boundary].split(|&byte| byte == 0);
    let mut diffs = Vec::new();
    let mut current = HashMap::new();
    while let Some(header) = fields.next() {
        let [status] = header.rsplit(|&byte| byte == b' ').next()? else {
            return None;
        };
        let status = *status;
        if !header.starts_with(b":") || !matches!(status, b'A' | b'M' | b'D' | b'T') {
            return None;
        }
        let path = fields.next()?.strip_prefix(prefix.as_bytes())?;
        let Ok(path) = std::str::from_utf8(path) else {
            continue;
        };
        if status == b'D' || status == b'T' {
            diffs.push(FileDiff {
                path: path.to_owned(),
                ranges: Vec::new(),
                untracked: false,
                deleted: true,
            });
        }
        if status != b'D' {
            current.insert(path.to_owned(), diffs.len());
            diffs.push(FileDiff {
                path: path.to_owned(),
                ranges: Vec::new(),
                untracked: false,
                deleted: false,
            });
        }
    }
    patch_ranges(&output[boundary + 2..], prefix, &current, &mut diffs)?;
    diffs.retain(|diff| diff.deleted || !diff.ranges.is_empty());
    Some(diffs)
}

fn patch_ranges(
    patch: &[u8],
    prefix: &str,
    paths: &HashMap<String, usize>,
    diffs: &mut [FileDiff],
) -> Option<()> {
    let mut current = None;
    let mut in_hunk = false;
    for line in patch.split(|&byte| byte == b'\n') {
        if line.starts_with(b"diff --git ") {
            current = None;
            in_hunk = false;
        } else if !in_hunk && let Some(path) = line.strip_prefix(b"+++ ") {
            let path = decode_patch_path(path)?;
            current = if path == b"/dev/null" {
                None
            } else {
                let path = path.strip_prefix(b"b/")?.strip_prefix(prefix.as_bytes())?;
                match std::str::from_utf8(path) {
                    Ok(path) => Some(*paths.get(path)?),
                    Err(_) => None,
                }
            };
        } else if let Some(hunk) = line.strip_prefix(b"@@ ") {
            in_hunk = true;
            let Some(index) = current else {
                continue;
            };
            let side = hunk
                .split(|byte| byte.is_ascii_whitespace())
                .find(|part| part.starts_with(b"+"))?;
            let mut numbers = std::str::from_utf8(&side[1..]).ok()?.split(',');
            let start: u32 = numbers.next()?.parse().ok()?;
            let count: u32 = numbers.next().map_or(Some(1), |value| value.parse().ok())?;
            let range = if count == 0 {
                (start.max(1), start.max(1))
            } else {
                (start, start.saturating_add(count - 1))
            };
            diffs.get_mut(index)?.ranges.push(range);
        }
    }
    Some(())
}

/// Git's C-style quoted patch paths, including octal-encoded UTF-8 bytes.
fn decode_patch_path(path: &[u8]) -> Option<Vec<u8>> {
    if !path.starts_with(b"\"") {
        return Some(path.split(|&byte| byte == b'\t').next()?.to_vec());
    }
    let mut out = Vec::new();
    let mut bytes = path[1..].iter().copied();
    while let Some(byte) = bytes.next() {
        match byte {
            b'"' => return Some(out),
            b'\\' => {
                let escaped = bytes.next()?;
                let value = match escaped {
                    b'"' | b'\\' => escaped,
                    b'a' => 7,
                    b'b' => 8,
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'v' => 11,
                    b'f' => 12,
                    b'r' => b'\r',
                    b'0'..=b'3' => {
                        let second = bytes.next()?;
                        let third = bytes.next()?;
                        if !(b'0'..=b'7').contains(&second) || !(b'0'..=b'7').contains(&third) {
                            return None;
                        }
                        (escaped - b'0') * 64 + (second - b'0') * 8 + (third - b'0')
                    }
                    _ => return None,
                };
                out.push(value);
            }
            _ => out.push(byte),
        }
    }
    None
}

/// HEAD commit id.
pub fn head(root: &Path) -> Option<String> {
    String::from_utf8(git(root, &["rev-parse", "HEAD"])?)
        .ok()
        .map(|text| text.trim().to_owned())
}

/// Root-relative files of each of the last `limit` non-merge commits, newest first.
pub fn history(root: &Path, limit: usize) -> Option<Vec<Vec<String>>> {
    let prefix = prefix(root)?;
    let limit = format!("-n{limit}");
    let output = git(
        root,
        &[
            "log",
            "--no-merges",
            &limit,
            "--name-only",
            "-z",
            "--no-renames",
            "--no-relative",
            "--format=%x00",
            "--",
            ".",
        ],
    )?;
    let mut commits = Vec::new();
    let mut files = Vec::new();
    let mut first = true;
    for field in output.split(|&byte| byte == 0) {
        if field.is_empty() {
            if !files.is_empty() {
                commits.push(std::mem::take(&mut files));
            }
            first = true;
            continue;
        }
        let path = if first {
            field.strip_prefix(b"\n")?
        } else {
            field
        };
        first = false;
        if let Some(path) = path.strip_prefix(prefix.as_bytes())
            && let Ok(path) = std::str::from_utf8(path)
            && !path.is_empty()
        {
            files.push(path.to_owned());
        }
    }
    if !files.is_empty() {
        commits.push(files);
    }
    Some(commits)
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
        let text = b":100644 100644 0000000 0000000 M\0sub/a.go\0:100644 000000 0000000 0000000 D\0sub/old.go\0\0diff --git a/sub/a.go b/sub/a.go\n--- a/sub/a.go\n+++ b/sub/a.go\n@@ -3,0 +4,2 @@ func x\n+a\n+b\n@@ -10 +12 @@\n-x\n+y\n@@ -20,3 +23,0 @@\n-gone\ndiff --git a/sub/old.go b/sub/old.go\n--- a/sub/old.go\n+++ /dev/null\n@@ -1,3 +0,0 @@\n";
        let diffs = parse_diff(text, "sub/");
        assert_eq!(
            diffs,
            Some(vec![
                FileDiff {
                    path: "a.go".to_owned(),
                    ranges: vec![(4, 5), (12, 12), (23, 23)],
                    untracked: false,
                    deleted: false,
                },
                FileDiff {
                    path: "old.go".to_owned(),
                    ranges: Vec::new(),
                    untracked: false,
                    deleted: true,
                }
            ])
        );
    }
}
