//! Freshness check: walk, stat comparison, parallel hashing and classification. A writing scan
//! also extracts changed files from the bytes it hashed, so every source is read once.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use rayon::prelude::*;
use xxhash_rust::xxh3::xxh3_128;

use crate::domain::corpus::{self, ChangeSet, Decision, FileRecord, Stat};
use crate::domain::facts::Facts;
use crate::domain::filetype::FileType;
use crate::domain::role::{self, ContentNeed, FileFacts, Role};
use crate::infra::{alloc_stats, walk};

/// C++ suffixes that make a same-stem `.h` file C++.
const CPP_SUFFIXES: [&str; 6] = ["cpp", "cc", "cxx", "hpp", "hh", "hxx"];

/// Largest read buffer a worker keeps for its next file.
const KEPT_BUFFER: usize = 4 << 20;

thread_local! {
    static READ_BUFFER: Cell<Vec<u8>> = const { Cell::new(Vec::new()) };
    /// The directory of the worker's last file: siblings skip the path walk of a full open.
    #[cfg(unix)]
    static LAST_DIRECTORY: std::cell::RefCell<Option<(String, File)>> =
        const { std::cell::RefCell::new(None) };
}

/// Opens a source file, relative to the worker's last directory when it is the same one. A directory
/// that cannot be opened (execute permission only) still allows the full path.
#[cfg(unix)]
fn open_source(root: &Path, relative: &str) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let (parent, name) = relative.rsplit_once('/').unwrap_or(("", relative));
    let opened = LAST_DIRECTORY.with_borrow_mut(|last| {
        if last.as_ref().is_none_or(|(known, _)| known != parent) {
            *last = File::open(root.join(parent))
                .ok()
                .map(|directory| (parent.to_owned(), directory));
        }
        let (_, directory) = last.as_ref()?;
        let name = std::ffi::CString::new(std::ffi::OsStr::new(name).as_bytes()).ok()?;
        // SAFETY: the descriptor and the C string outlive the call.
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        // SAFETY: a non-negative result is a new descriptor owned by nothing else.
        (descriptor >= 0).then(|| unsafe { File::from_raw_fd(descriptor) })
    });
    match opened {
        Some(file) => Ok(file),
        None => File::open(root.join(relative)),
    }
}

#[cfg(not(unix))]
fn open_source(root: &Path, relative: &str) -> io::Result<File> {
    File::open(root.join(relative))
}

/// Extracts a file from its bytes; `None` means permission to read it was denied.
pub type Extract<'a> = &'a (dyn Fn(&FileRecord, Option<&[u8]>) -> Facts + Sync);

/// What a scan learned about a file's bytes.
enum Content {
    Read(Vec<u8>),
    /// The head only: the file is not extracted and its content has no hash.
    Head(Vec<u8>),
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
    // Only C headers ask for their siblings.
    let paths: HashSet<&str> = if walked.iter().any(|stat| stat.path.ends_with(".h")) {
        walked.iter().map(|stat| stat.path.as_str()).collect()
    } else {
        HashSet::new()
    };
    let extracted = ExtractedContent::default();
    let hashed: Vec<io::Result<(FileRecord, bool, Option<Facts>)>> = decisions
        .par_iter()
        .zip(walked.par_iter())
        .map(|(decision, stat)| match decision {
            Decision::Unchanged(index) if !sibling_context_changed(&paths, stored, &stat.path) => {
                Ok((stored[*index].clone(), false, None))
            }
            Decision::Unchanged(index) => {
                let (record, content) = inspect(root, stat, has_cpp_sibling(&paths, &stat.path))?;
                let changed = record_changed(&stored[*index], &record);
                Ok(finish(record, content, changed, extract, &extracted))
            }
            Decision::Hash(previous) => {
                let (record, content) = inspect(root, stat, has_cpp_sibling(&paths, &stat.path))?;
                let changed = previous.is_none_or(|index| record_changed(&stored[index], &record));
                Ok(finish(record, content, changed, extract, &extracted))
            }
        })
        .collect();
    // The buffers serve this pass only: the next stages need the memory.
    rayon::broadcast(|_| {
        READ_BUFFER.take();
        #[cfg(unix)]
        LAST_DIRECTORY.with_borrow_mut(Option::take);
    });
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

/// A record changed for the graph: content of an extracted file, or what the graph shows of any
/// file. A touched binary only updates the file table.
fn record_changed(old: &FileRecord, new: &FileRecord) -> bool {
    if old.role != new.role || old.file_type != new.file_type {
        return true;
    }
    if new.role.is_extracted() {
        return old.hash != new.hash
            || (new.hash == 0 && (old.size, old.mtime_ns) != (new.size, new.mtime_ns));
    }
    old.size.div_ceil(1024) != new.size.div_ceil(1024)
}

/// Facts of byte-identical documents of one scan, extracted once. Only binary documents: nothing
/// in their extraction reads the path, which `first_extract_of_identical_documents_is_reused` and
/// the fact comparison over all corpora prove.
type ExtractedContent = Mutex<HashMap<(u128, Option<FileType>, Role), Arc<OnceLock<Facts>>>>;

/// Extracts a changed file and hands its read buffer back to the worker.
fn finish(
    record: FileRecord,
    content: Content,
    changed: bool,
    extract: Option<Extract>,
    extracted: &ExtractedContent,
) -> (FileRecord, bool, Option<Facts>) {
    let facts = extract
        .filter(|_| changed && record.role.is_extracted())
        .and_then(|extract| match &content {
            Content::Read(bytes) if record.file_type.is_some_and(FileType::is_binary_document) => {
                let slot = {
                    let mut known = extracted.lock().unwrap_or_else(PoisonError::into_inner);
                    Arc::clone(
                        known
                            .entry((record.hash, record.file_type, record.role))
                            .or_default(),
                    )
                };
                Some(slot.get_or_init(|| extract(&record, Some(bytes))).clone())
            }
            Content::Read(bytes) => Some(extract(&record, Some(bytes))),
            Content::Denied => Some(extract(&record, None)),
            Content::Head(_) | Content::NotRead => None,
        });
    if let Content::Read(buffer) | Content::Head(buffer) = content
        && buffer.capacity() <= KEPT_BUFFER
    {
        READ_BUFFER.set(buffer);
    }
    (record, changed, facts)
}

/// The file in the worker's buffer, taken out so a nested job cannot share it; `Head` stops after
/// the bytes classification looks at unless they make the file extracted.
fn read_file(
    root: &Path,
    relative: &str,
    stat_size: u64,
    need: ContentNeed,
    classify_head: impl Fn(&[u8]) -> bool,
) -> io::Result<(Vec<u8>, bool)> {
    let mut buffer = READ_BUFFER.take();
    buffer.clear();
    let mut file = open_source(root, relative)?;
    if need == ContentNeed::Head {
        let head = usize::try_from(stat_size)
            .unwrap_or(usize::MAX)
            .min(role::HEAD_LEN);
        buffer.resize(head, 0);
        let mut filled = 0;
        while filled < head {
            match file.read(&mut buffer[filled..])? {
                0 => break,
                read => filled += read,
            }
        }
        buffer.truncate(filled);
        if !classify_head(&buffer) {
            return Ok((buffer, false));
        }
        file.read_to_end(&mut buffer)?;
        return Ok((buffer, true));
    }
    // One read: a result of exactly the walked size is that snapshot; anything else (growth, a
    // short read, a pseudo file) takes the loop, and the next walk sees a changed stat.
    let size = usize::try_from(stat_size).unwrap_or(0);
    buffer.resize(size + 1, 0);
    let read = file.read(&mut buffer)?;
    if read == size {
        buffer.truncate(read);
    } else {
        buffer.truncate(read);
        file.read_to_end(&mut buffer)?;
    }
    Ok((buffer, true))
}

/// Reads, hashes and classifies one file; sensitive and large files are never read, a file only
/// its head can classify is not read further unless the head makes it extracted.
fn inspect(root: &Path, stat: &Stat, cpp_sibling: bool) -> io::Result<(FileRecord, Content)> {
    let need = role::content_need(&stat.path, stat.size);
    let head_extracted = |head: &[u8]| {
        role::classify(&FileFacts {
            path: &stat.path,
            size: stat.size,
            content: Some(head),
            cpp_sibling,
        })
        .role
        .is_extracted()
    };
    let (hash, content) = if need == ContentNeed::None {
        (0, Content::NotRead)
    } else {
        match read_file(root, &stat.path, stat.size, need, head_extracted) {
            Ok((content, true)) => {
                alloc_stats::read(content.len());
                (xxh3_128(&content), Content::Read(content))
            }
            Ok((head, false)) => {
                alloc_stats::read(head.len());
                (0, Content::Head(head))
            }
            // Classified by path as empty content; extraction records the file as unreadable.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => (0, Content::Denied),
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("read source {}: {error}", stat.path),
                ));
            }
        }
    };
    let bytes = match &content {
        Content::Read(bytes) | Content::Head(bytes) => Some(&bytes[..]),
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

    #[test]
    fn a_file_only_its_head_classifies_is_not_hashed_or_read_further() -> io::Result<()> {
        let dir = TestDir::new("scan-head")?;
        let mut binary = vec![0_u8; 64];
        binary.extend(vec![7_u8; 64 * 1024]);
        std::fs::write(dir.path().join("blob"), &binary)?;
        dir.write("data.unknownext", "plain text with a late nul follows")?;
        let script = format!("#!/bin/sh\necho hi\n{}", "# pad\n".repeat(4000));
        dir.write("tool", &script)?;
        let changes = scan(dir.path(), &[], &[], FAR_FUTURE, None)?;
        assert_eq!(
            record(&changes, "blob").map(|r| (r.role, r.hash)),
            Some((Role::Binary, 0))
        );
        assert_eq!(
            record(&changes, "data.unknownext").map(|r| (r.role, r.hash)),
            Some((Role::Other, 0))
        );
        let tool = record(&changes, "tool");
        assert_eq!(tool.map(|r| r.hash), Some(xxh3_128(script.as_bytes())));
        assert!(tool.is_some_and(|r| r.role.is_extracted()));
        Ok(())
    }

    #[test]
    fn a_touched_binary_is_no_graph_change_but_a_new_size_is() -> io::Result<()> {
        let dir = TestDir::new("scan-touched")?;
        let mut binary = vec![0_u8; 8];
        binary.extend(vec![7_u8; 4096]);
        std::fs::write(dir.path().join("blob"), &binary)?;
        let first = scan(dir.path(), &[], &[], FAR_FUTURE, None)?;
        // Same size, other bytes, inside the racy window: the file table alone changes.
        binary[100] = 9;
        std::fs::write(dir.path().join("blob"), &binary)?;
        let touched = scan(dir.path(), &[], &first.records, FAR_FUTURE, None)?;
        assert!(touched.changed.is_empty());
        binary.extend(vec![7_u8; 4096]);
        std::fs::write(dir.path().join("blob"), &binary)?;
        let grown = scan(dir.path(), &[], &first.records, FAR_FUTURE, None)?;
        assert_eq!(grown.changed, [0]);
        Ok(())
    }

    #[test]
    fn a_stale_walked_size_still_reads_the_whole_file() -> io::Result<()> {
        let dir = TestDir::new("scan-stale-size")?;
        dir.write("a.go", "package a\n")?;
        for walked in [0, 3, 10, 11, 4096] {
            let (bytes, whole) =
                read_file(dir.path(), "a.go", walked, ContentNeed::Whole, |_| true)?;
            assert!(whole);
            assert_eq!(bytes, b"package a\n", "walked size {walked}");
        }
        Ok(())
    }

    #[test]
    fn first_extract_of_identical_documents_is_reused() -> Result<(), Box<dyn std::error::Error>> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = TestDir::new("scan-identical")?;
        for path in ["a/x.rtf", "b/y.rtf", "c/x.rtf"] {
            dir.write(path, "{\\rtf1\\ansi hello}")?;
        }
        dir.write("d/z.rtf", "{\\rtf1\\ansi other}")?;
        let calls = AtomicUsize::new(0);
        let extract = |_: &FileRecord, _: Option<&[u8]>| {
            calls.fetch_add(1, Ordering::Relaxed);
            Facts::default()
        };
        let changes = scan(dir.path(), &[], &[], FAR_FUTURE, Some(&extract))?;
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(changes.facts.iter().flatten().count(), 4);
        Ok(())
    }
}
