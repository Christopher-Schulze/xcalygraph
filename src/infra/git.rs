//! Git through the `git` CLI (`docs/decisions.md` D12): changed path metadata, line ranges and
//! eligible blob content at a revision. Content reads preserve Git failures as errors.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Output};

/// Changed new-side line ranges; an empty list means Git produced no text hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Root-relative path.
    pub path: String,
    /// Inclusive new-side line ranges; a pure deletion is the line after which it happened.
    pub ranges: Vec<(u32, u32)>,
    pub untracked: bool,
    /// The file existed at the revision and is gone from the work tree; `ranges` is empty.
    pub deleted: bool,
    /// Blob identity at the revision; `None` means the path was absent there.
    pub old_object: Option<String>,
    /// Whether the old object is a regular-file blob eligible for content extraction.
    pub old_regular: bool,
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
            "--raw",
            "-z",
            "--no-abbrev",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--no-relative",
            rev,
            "--",
            ".",
        ],
    )?;
    let mut diffs = parse_raw_diff(&output, &prefix)?;
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
                old_object: None,
                old_regular: false,
            });
        }
    }
    diffs.sort_by(|a, b| a.path.cmp(&b.path));
    Some(diffs)
}

/// Changed new-side line ranges for the exact root-relative paths that may be read.
pub fn ranges(
    root: &Path,
    rev: &str,
    paths: &[String],
) -> Option<HashMap<String, Vec<(u32, u32)>>> {
    if paths.is_empty() {
        return Some(HashMap::new());
    }
    let prefix = prefix(root)?;
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args([
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--no-relative",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--unified=0",
        rev,
        "--",
    ]);
    for path in paths {
        command.arg(format!(":(top,literal){prefix}{path}"));
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_patch_ranges(&output.stdout, &prefix, paths)
}

/// Git's similarity-based rename candidates, restricted to the supplied eligible paths.
pub fn renames(root: &Path, rev: &str, paths: &[String]) -> Option<Vec<(String, String)>> {
    if paths.is_empty() {
        return Some(Vec::new());
    }
    let prefix = prefix(root)?;
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args([
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
    ]);
    for path in paths {
        command.arg(format!(":(top,literal){prefix}{path}"));
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut fields = output.stdout.split(|&byte| byte == 0);
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

/// Read NUL-delimited raw changes without requesting or capturing blob contents.
fn parse_raw_diff(output: &[u8], prefix: &str) -> Option<Vec<FileDiff>> {
    let mut diffs = Vec::new();
    let mut fields = output.split(|&byte| byte == 0);
    while let Some(header) = fields.next().filter(|header| !header.is_empty()) {
        let metadata = header.strip_prefix(b":")?;
        let mut parts = metadata
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|part| !part.is_empty());
        let old_mode = parts.next()?;
        let _new_mode = parts.next()?;
        let old_id = parts.next()?;
        let new_id = parts.next()?;
        let status = parts.next()?;
        if parts.next().is_some()
            || !matches!(status, b"A" | b"M" | b"D" | b"T")
            || !valid_object_id(old_id)
            || !valid_object_id(new_id)
        {
            return None;
        }
        let old_object = if old_id.iter().all(|&byte| byte == b'0') {
            None
        } else {
            Some(String::from_utf8(old_id.to_vec()).ok()?)
        };
        let old_regular = matches!(old_mode, b"100644" | b"100755");
        let path = fields.next()?.strip_prefix(prefix.as_bytes())?;
        let Ok(path) = std::str::from_utf8(path) else {
            continue;
        };
        if status == b"D" || status == b"T" {
            diffs.push(FileDiff {
                path: path.to_owned(),
                ranges: Vec::new(),
                untracked: false,
                deleted: true,
                old_object: old_object.clone(),
                old_regular,
            });
        }
        if status != b"D" {
            diffs.push(FileDiff {
                path: path.to_owned(),
                ranges: Vec::new(),
                untracked: false,
                deleted: false,
                old_object,
                old_regular,
            });
        }
    }
    Some(diffs)
}

fn valid_object_id(object: &[u8]) -> bool {
    matches!(object.len(), 40 | 64) && object.iter().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_patch_ranges(
    patch: &[u8],
    prefix: &str,
    paths: &[String],
) -> Option<HashMap<String, Vec<(u32, u32)>>> {
    let allowed: HashSet<&str> = paths.iter().map(String::as_str).collect();
    let mut ranges = HashMap::<String, Vec<(u32, u32)>>::new();
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
                    Ok(path) if allowed.contains(path) => Some(path.to_owned()),
                    Ok(_) => return None,
                    Err(_) => None,
                }
            };
        } else if let Some(hunk) = line.strip_prefix(b"@@ ") {
            in_hunk = true;
            if current.is_none() {
                continue;
            }
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
            ranges.entry(current.clone()?).or_default().push(range);
        }
    }
    Some(ranges)
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

fn checked_git(root: &Path, args: &[&str]) -> Result<Output, String> {
    let command = args.first().copied().unwrap_or("command");
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("cannot start git {command}: {error}"))?;
    if !output.status.success() {
        return Err(format!("git {command} exited with {}", output.status));
    }
    Ok(output)
}

/// Size of a validated blob object, without reading its content.
pub fn object_size(root: &Path, object: &str) -> Result<u64, String> {
    if !valid_object_id(object.as_bytes()) {
        return Err("invalid Git blob identity".to_owned());
    }
    let output = checked_git(root, &["cat-file", "-s", object])?;
    std::str::from_utf8(&output.stdout)
        .map_err(|_| "git returned a non-UTF-8 blob size".to_owned())?
        .trim()
        .parse()
        .map_err(|_| "git returned an invalid blob size".to_owned())
}

/// Content of a validated blob object. Callers must enforce the path and size policy first.
pub fn blob(root: &Path, object: &str) -> Result<Vec<u8>, String> {
    if !valid_object_id(object.as_bytes()) {
        return Err("invalid Git blob identity".to_owned());
    }
    Ok(checked_git(root, &["cat-file", "blob", object])?.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_zero_hunks_become_new_side_ranges() -> Result<(), Box<dyn std::error::Error>> {
        let old = "1".repeat(40);
        let current = "2".repeat(40);
        let removed = "3".repeat(40);
        let zero = "0".repeat(40);
        let raw = format!(
            ":100644 100644 {old} {current} M\0sub/a.go\0:100644 000000 {removed} {zero} D\0sub/old.go\0"
        );
        let diffs = parse_raw_diff(raw.as_bytes(), "sub/")
            .ok_or_else(|| std::io::Error::other("raw diff parse failed"))?;
        assert_eq!(
            diffs,
            vec![
                FileDiff {
                    path: "a.go".to_owned(),
                    ranges: Vec::new(),
                    untracked: false,
                    deleted: false,
                    old_object: Some(old),
                    old_regular: true,
                },
                FileDiff {
                    path: "old.go".to_owned(),
                    ranges: Vec::new(),
                    untracked: false,
                    deleted: true,
                    old_object: Some(removed),
                    old_regular: true,
                }
            ]
        );
        let paths = ["a.go".to_owned()];
        let patch = b"diff --git a/sub/a.go b/sub/a.go\n--- a/sub/a.go\n+++ b/sub/a.go\n@@ -3,0 +4,2 @@ func x\n+a\n+b\n@@ -10 +12 @@\n-x\n+y\n@@ -20,3 +23,0 @@\n-gone\n";
        let ranges = parse_patch_ranges(patch, "sub/", &paths)
            .ok_or_else(|| std::io::Error::other("patch range parse failed"))?;
        assert_eq!(
            ranges,
            HashMap::from([("a.go".to_owned(), vec![(4, 5), (12, 12), (23, 23)],)])
        );
        Ok(())
    }
}
