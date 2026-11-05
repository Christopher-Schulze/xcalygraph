//! Freshness check: walk, stat comparison, parallel hashing and classification. A writing scan
//! also extracts changed files from the bytes it hashed, so every source is read once.

use std::cell::Cell;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use xxhash_rust::xxh3::xxh3_128;

use crate::domain::corpus::{self, ChangeSet, Decision, FileRecord, Stat};
use crate::domain::facts::Facts;
use crate::domain::role::{self, FileFacts};
use crate::infra::walk;

/// C++ suffixes that make a same-stem `.h` file C++.
const CPP_SUFFIXES: [&str; 6] = ["cpp", "cc", "cxx", "hpp", "hh", "hxx"];

/// Largest read buffer a worker keeps for its next file.
const KEPT_BUFFER: usize = 4 << 20;

thread_local! {
    static READ_BUFFER: Cell<Vec<u8>> = const { Cell::new(Vec::new()) };
}

/// Extracts a file from its bytes; `None` means permission to read it was denied.
pub type Extract<'a> = &'a (dyn Fn(&FileRecord, Option<&[u8]>) -> Facts + Sync);

/// What a scan learned about a file's bytes.
enum Content {
    Read(Vec<u8>),
    Denied,
    NotRead,
}

/// Walks `root` and compares it with `stored` (sorted by path) as of `write_time_ns`; with
/// `extract`, also extracts every changed file.
pub fn scan(
    root: &Path,
    excluded: &[PathBuf],
    stored: &[FileRecord],
    write_time_ns: i64,
    extract: Option<Extract>,
) -> io::Result<ChangeSet> {
    let walked = walk::walk(root, excluded)?;
    let (decisions, deleted) = corpus::compare(stored, &walked, write_time_ns);
    let paths: HashSet<&str> = walked.iter().map(|stat| stat.path.as_str()).collect();
    let hashed: Vec<io::Result<(FileRecord, bool, Option<Facts>)>> = decisions
        .par_iter()
        .zip(walked.par_iter())
        .map(|(decision, stat)| match decision {
            Decision::Unchanged(index) if !sibling_context_changed(&paths, stored, &stat.path) => {
                Ok((stored[*index].clone(), false, None))
            }
            Decision::Unchanged(index) => {
                let (record, content) = inspect(root, stat, has_cpp_sibling(&paths, &stat.path))?;
                let old = &stored[*index];
                let changed = old.hash != record.hash
                    || old.role != record.role
                    || old.file_type != record.file_type;
                Ok(finish(record, content, changed, extract))
            }
            Decision::Hash(previous) => {
                let (record, content) = inspect(root, stat, has_cpp_sibling(&paths, &stat.path))?;
                let changed = previous.is_none_or(|index| {
                    let old = &stored[index];
                    old.hash != record.hash
                        || old.role != record.role
                        || old.file_type != record.file_type
                        || record.hash == 0
                            && (old.size, old.mtime_ns) != (record.size, record.mtime_ns)
                });
                Ok(finish(record, content, changed, extract))
            }
        })
        .collect();
    // The buffers serve this pass only: the next stages need the memory.
    rayon::broadcast(|_| READ_BUFFER.take());
    let mut changes = ChangeSet {
        deleted: deleted
            .iter()
            .map(|&index| stored[index].path.clone())
            .collect(),
        ..ChangeSet::default()
    };
    for candidate in hashed {
        let (record, changed, facts) = candidate?;
        if changed {
            changes.changed.push(changes.records.len());
            changes.facts.push(facts);
        }
        changes.records.push(record);
    }
    changes.deleted.sort_unstable();
    changes.records_changed = changes.records.len() != stored.len()
        || changes
            .records
            .iter()
            .zip(stored)
            .any(|(new, old)| new != old);
    Ok(changes)
}

/// Extracts a changed file and hands its read buffer back to the worker.
fn finish(
    record: FileRecord,
    content: Content,
    changed: bool,
    extract: Option<Extract>,
) -> (FileRecord, bool, Option<Facts>) {
    let facts = extract
        .filter(|_| changed && record.role.is_extracted())
        .and_then(|extract| match &content {
            Content::Read(bytes) => Some(extract(&record, Some(bytes))),
            Content::Denied => Some(extract(&record, None)),
            Content::NotRead => None,
        });
    if let Content::Read(buffer) = content
        && buffer.capacity() <= KEPT_BUFFER
    {
        READ_BUFFER.set(buffer);
    }
    (record, changed, facts)
}

/// The whole file in the worker's buffer, taken out so a nested job cannot share it.
fn read_file(path: &Path, size: u64) -> io::Result<Vec<u8>> {
    let mut buffer = READ_BUFFER.take();
    buffer.clear();
    buffer.reserve(usize::try_from(size).unwrap_or(0));
    File::open(path)?.read_to_end(&mut buffer)?;
    Ok(buffer)
}

/// Reads, hashes and classifies one file; sensitive and large files are never read.
fn inspect(root: &Path, stat: &Stat, cpp_sibling: bool) -> io::Result<(FileRecord, Content)> {
    let (hash, content) = if role::needs_content(&stat.path, stat.size) {
        match read_file(&root.join(&stat.path), stat.size) {
            Ok(content) => (xxh3_128(&content), Content::Read(content)),
            // Classified by path as empty content; extraction records the file as unreadable.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => (0, Content::Denied),
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("read source {}: {error}", stat.path),
                ));
            }
        }
    } else {
        (0, Content::NotRead)
    };
    let bytes = match &content {
        Content::Read(bytes) => Some(&bytes[..]),
        Content::Denied => Some(&[][..]),
        Content::NotRead => None,
    };
    let classification = role::classify(&FileFacts {
        path: &stat.path,
        size: stat.size,
        content: bytes,
        cpp_sibling,
    });
    let record = FileRecord {
        path: stat.path.clone(),
        size: stat.size,
        mtime_ns: stat.mtime_ns,
        identity: stat.identity,
        hash,
        role: classification.role,
        file_type: classification.file_type,
    };
    Ok((record, content))
}

/// Whether a header's C++ sibling appeared or disappeared since the stored scan; the only
/// classification input that lies outside the file itself (`role::FileFacts`). `stored` is
/// sorted by path, so the old state costs binary searches, no extra set.
fn sibling_context_changed(paths: &HashSet<&str>, stored: &[FileRecord], path: &str) -> bool {
    let Some(stem) = path.strip_suffix(".h") else {
        return false;
    };
    let before = CPP_SUFFIXES.iter().any(|suffix| {
        let sibling = format!("{stem}.{suffix}");
        stored
            .binary_search_by(|record| record.path.as_str().cmp(&sibling))
            .is_ok()
    });
    before != has_cpp_sibling(paths, path)
}

fn has_cpp_sibling(paths: &HashSet<&str>, path: &str) -> bool {
    let Some(stem) = path.strip_suffix(".h") else {
        return false;
    };
    CPP_SUFFIXES
        .iter()
        .any(|suffix| paths.contains(format!("{stem}.{suffix}").as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::filetype::FileType;
    use crate::domain::role::Role;
    use crate::infra::testdir::TestDir;

    const FAR_FUTURE: i64 = i64::MAX;

    #[test]
    fn a_writing_scan_extracts_changed_files_from_the_hashed_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dir = TestDir::new("scan-extract")?;
        let root = dir.path().join("root");
        dir.write("root/a.go", "package a\n")?;
        dir.write("root/b.go", "package b\n")?;
        dir.write("root/.env", "SECRET=1\n")?;
        let denied = dir.write("root/denied.go", "package denied\n")?;
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000))?;
        let denied_is_enforced = fs::read(&denied).is_err();

        let calls = AtomicUsize::new(0);
        let extract = |record: &FileRecord, content: Option<&[u8]>| {
            calls.fetch_add(1, Ordering::Relaxed);
            let lines = match content {
                Some(bytes) if xxh3_128(bytes) == record.hash => 1,
                Some(_) => 2,
                None => 3,
            };
            Facts {
                lines,
                ..Facts::default()
            }
        };
        let first = scan(&root, &[], &[], 0, Some(&extract))?;
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o644))?;
        assert_eq!(first.facts.len(), first.changed.len());
        let lines = |path: &str| -> Option<u32> {
            let position = first
                .changed
                .iter()
                .position(|&index| first.records[index].path == path)?;
            first.facts[position].as_ref().map(|facts| facts.lines)
        };
        assert_eq!(lines("a.go"), Some(1), "extracted from the bytes it hashed");
        assert_eq!(lines("b.go"), Some(1));
        assert_eq!(lines(".env"), None, "sensitive files are never read");
        if denied_is_enforced {
            assert_eq!(lines("denied.go"), Some(3), "unreadable stays explicit");
        }

        // Unchanged files are not extracted, a read-only scan extracts nothing.
        let before = calls.load(Ordering::Relaxed);
        let records = first.records.clone();
        let write_time = i64::MAX;
        dir.write("root/a.go", "package a\n\nfunc A() {}\n")?;
        let read_only = scan(&root, &[], &records, write_time, None)?;
        assert!(read_only.facts.iter().all(Option::is_none));
        assert_eq!(calls.load(Ordering::Relaxed), before);
        let second = scan(&root, &[], &records, write_time, Some(&extract))?;
        let changed: Vec<&str> = second
            .changed
            .iter()
            .map(|&index| second.records[index].path.as_str())
            .collect();
        assert_eq!(changed.iter().filter(|path| **path == "a.go").count(), 1);
        assert_eq!(
            calls.load(Ordering::Relaxed) - before,
            second.facts.iter().flatten().count()
        );
        Ok(())
    }

    fn record<'a>(changes: &'a ChangeSet, path: &str) -> Option<&'a FileRecord> {
        changes.records.iter().find(|record| record.path == path)
    }

    #[test]
    fn first_scan_hashes_and_classifies_everything() -> io::Result<()> {
        let dir = TestDir::new("scan-first")?;
        dir.write(".gitignore", "!.env*\n")?;
        dir.write("main.go", "package main")?;
        dir.write("lib.h", "int f(void);")?;
        dir.write("view.h", "int g(void);")?;
        dir.write("view.cpp", "int g() { return 1; }")?;
        dir.write(".env", "TOKEN=secret")?;
        let changes = scan(dir.path(), &[], &[], FAR_FUTURE, None)?;
        assert_eq!(changes.records.len(), 6);
        assert_eq!(changes.changed, [0, 1, 2, 3, 4, 5]);
        let main = record(&changes, "main.go");
        assert_eq!(
            main.map(|r| (r.role, r.file_type)),
            Some((Role::Src, Some(FileType::Go)))
        );
        assert_eq!(main.map(|r| r.hash), Some(xxh3_128(b"package main")));
        assert_eq!(
            record(&changes, "lib.h").and_then(|r| r.file_type),
            Some(FileType::C)
        );
        assert_eq!(
            record(&changes, "view.h").and_then(|r| r.file_type),
            Some(FileType::Cpp)
        );
        let secret = record(&changes, ".env");
        assert_eq!(secret.map(|r| (r.role, r.hash)), Some((Role::Sensitive, 0)));
        Ok(())
    }

    #[test]
    fn rescan_reports_only_real_changes() -> io::Result<()> {
        let dir = TestDir::new("scan-rescan")?;
        dir.write("a.go", "package a")?;
        dir.write("b.go", "package b")?;
        dir.write("c.go", "package c")?;
        let first = scan(dir.path(), &[], &[], FAR_FUTURE, None)?;

        // Inside the racy window every file is re-hashed, but equal content is no change.
        let same = scan(dir.path(), &[], &first.records, 0, None)?;
        assert!(same.changed.is_empty() && same.deleted.is_empty());
        assert!(!same.records_changed);

        // Outside the racy window nothing is read at all.
        let quiet = scan(dir.path(), &[], &first.records, FAR_FUTURE, None)?;
        assert!(quiet.is_empty());

        dir.write("b.go", "package b2")?;
        std::fs::remove_file(dir.path().join("c.go"))?;
        dir.write("d.go", "package d")?;
        let changed = scan(dir.path(), &[], &first.records, 0, None)?;
        let changed_paths: Vec<&str> = changed
            .changed
            .iter()
            .map(|&index| changed.records[index].path.as_str())
            .collect();
        assert_eq!(changed_paths, ["b.go", "d.go"]);
        assert_eq!(changed.deleted, ["c.go"]);
        assert!(changed.records_changed);
        Ok(())
    }

    #[test]
    fn touched_file_with_equal_content_updates_only_the_stat_record() -> io::Result<()> {
        let dir = TestDir::new("scan-touch")?;
        dir.write("a.go", "package a")?;
        let first = scan(dir.path(), &[], &[], FAR_FUTURE, None)?;
        let mut stale = first.records.clone();
        stale[0].mtime_ns -= 1_000;
        let touched = scan(dir.path(), &[], &stale, FAR_FUTURE, None)?;
        assert!(touched.changed.is_empty());
        assert!(touched.records_changed);
        assert_eq!(touched.records, first.records);
        Ok(())
    }

    #[test]
    fn sensitive_and_large_content_is_never_read() -> io::Result<()> {
        let dir = TestDir::new("scan-sensitive")?;
        dir.write(".gitignore", "!.env*\n")?;
        dir.write("deploy/id_rsa", "PRIVATE")?;
        dir.write(".env.local", "TOKEN=x")?;
        let changes = scan(dir.path(), &[], &[], FAR_FUTURE, None)?;
        // A read would yield a non-zero content hash.
        for path in ["deploy/id_rsa", ".env.local"] {
            let record = record(&changes, path);
            assert_eq!(
                record.map(|r| (r.role, r.hash)),
                Some((Role::Sensitive, 0)),
                "{path}"
            );
        }
        Ok(())
    }

    #[test]
    fn unreadable_source_is_not_classified_as_other() -> io::Result<()> {
        let dir = TestDir::new("scan-read-error")?;
        std::fs::create_dir(dir.path().join("unreadable.go"))?;
        let stat = Stat {
            path: "unreadable.go".to_owned(),
            size: 0,
            mtime_ns: 0,
            identity: None,
        };
        assert!(inspect(dir.path(), &stat, false).is_err());
        Ok(())
    }
}
