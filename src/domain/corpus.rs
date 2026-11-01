//! Corpus records and change detection (`docs/spec.md` section 4, Freshness).

use super::filetype::FileType;
use super::role::Role;

/// Racy-clean window: a file modified this close to the last index write is re-hashed.
pub const RACY_WINDOW_NS: i64 = 2_000_000_000;

/// Filesystem identity and status-change time; absent when the platform cannot provide them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub change_time_ns: i64,
}

/// File metadata from the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    /// Root-relative path with `/` separators.
    pub path: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub identity: Option<FileIdentity>,
}

/// One file of the index: stat record, content hash and classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    pub path: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub identity: Option<FileIdentity>,
    /// XXH3-128 of the content; 0 for files whose content is never read.
    pub hash: u128,
    pub role: Role,
    pub file_type: Option<FileType>,
}

/// What the stat comparison decided for one walked file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Stat record equal and outside the racy window: reuse the stored record.
    Unchanged(usize),
    /// New file, or stat differs, or inside the racy window: hash it; `Some` holds the stored
    /// record to compare the hash with.
    Hash(Option<usize>),
}

/// Compares walked files against stored records; both lists are sorted by path.
/// Returns one decision per walked file and the indices of stored records that were deleted.
pub fn compare(
    stored: &[FileRecord],
    walked: &[Stat],
    write_time_ns: i64,
) -> (Vec<Decision>, Vec<usize>) {
    let mut decisions = Vec::with_capacity(walked.len());
    let mut deleted = Vec::new();
    let mut old = 0;
    for stat in walked {
        while old < stored.len() && stored[old].path < stat.path {
            deleted.push(old);
            old += 1;
        }
        if old < stored.len() && stored[old].path == stat.path {
            let record = &stored[old];
            let same = record.size == stat.size
                && record.mtime_ns == stat.mtime_ns
                && stat.mtime_ns < write_time_ns.saturating_sub(RACY_WINDOW_NS)
                && matches!(
                    (record.identity, stat.identity),
                    (Some(previous), Some(current))
                        if previous == current
                            && current.change_time_ns
                                < write_time_ns.saturating_sub(RACY_WINDOW_NS)
                );
            decisions.push(if same {
                Decision::Unchanged(old)
            } else {
                Decision::Hash(Some(old))
            });
            old += 1;
        } else {
            decisions.push(Decision::Hash(None));
        }
    }
    deleted.extend(old..stored.len());
    (decisions, deleted)
}

/// Result of a freshness check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// Every current file, sorted by path.
    pub records: Vec<FileRecord>,
    /// Indices into `records` of new files and files whose content or role changed.
    pub changed: Vec<usize>,
    /// Paths of stored files that no longer exist.
    pub deleted: Vec<String>,
    /// True when any stored stat record differs from `records` (the file table must be rewritten).
    pub records_changed: bool,
}

impl ChangeSet {
    /// An empty change set answers from the existing index without writes.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.deleted.is_empty() && !self.records_changed
    }

    pub fn has_content_changes(&self) -> bool {
        !self.changed.is_empty() || !self.deleted.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str, size: u64, mtime_ns: i64) -> FileRecord {
        FileRecord {
            path: path.to_owned(),
            size,
            mtime_ns,
            identity: Some(FileIdentity {
                device: 1,
                inode: 1,
                change_time_ns: mtime_ns,
            }),
            hash: 1,
            role: Role::Src,
            file_type: Some(FileType::Go),
        }
    }

    fn stat(path: &str, size: u64, mtime_ns: i64) -> Stat {
        Stat {
            path: path.to_owned(),
            size,
            mtime_ns,
            identity: Some(FileIdentity {
                device: 1,
                inode: 1,
                change_time_ns: mtime_ns,
            }),
        }
    }

    #[test]
    fn stat_comparison_detects_new_changed_deleted_and_racy_files() {
        let write_time = 100 * RACY_WINDOW_NS;
        let old_time = write_time - 2 * RACY_WINDOW_NS;
        let racy_time = write_time - RACY_WINDOW_NS / 2;
        let stored = [
            record("a.go", 1, old_time),
            record("b.go", 1, old_time),
            record("c.go", 1, old_time),
            record("d.go", 1, racy_time),
            record("e.go", 1, old_time),
        ];
        let walked = [
            stat("a.go", 1, old_time),
            stat("b.go", 2, old_time),
            stat("c.go", 1, old_time + 1),
            stat("d.go", 1, racy_time),
            stat("f.go", 1, old_time),
        ];
        let (decisions, deleted) = compare(&stored, &walked, write_time);
        assert_eq!(
            decisions,
            [
                Decision::Unchanged(0),
                Decision::Hash(Some(1)),
                Decision::Hash(Some(2)),
                Decision::Hash(Some(3)),
                Decision::Hash(None),
            ]
        );
        assert_eq!(deleted, [4]);

        let original = record("a.go", 1, old_time);
        let mut unknown = stat("a.go", 1, old_time);
        unknown.identity = None;
        let mut different = stat("a.go", 1, old_time);
        if let Some(identity) = &mut different.identity {
            identity.inode += 1;
        }
        let mut other_device = stat("a.go", 1, old_time);
        if let Some(identity) = &mut other_device.identity {
            identity.device += 1;
        }
        let mut changed_status = stat("a.go", 1, old_time);
        if let Some(identity) = &mut changed_status.identity {
            identity.change_time_ns += 1;
        }
        let mut recent = stat("a.go", 1, old_time);
        if let Some(identity) = &mut recent.identity {
            identity.change_time_ns = racy_time;
        }
        for candidate in [
            unknown,
            different,
            other_device,
            changed_status,
            recent.clone(),
        ] {
            let (decision, _) = compare(
                std::slice::from_ref(&original),
                std::slice::from_ref(&candidate),
                write_time,
            );
            assert_eq!(decision, [Decision::Hash(Some(0))]);
        }
        let mut without_identity = original.clone();
        without_identity.identity = None;
        let (decision, _) = compare(
            std::slice::from_ref(&without_identity),
            &[stat("a.go", 1, old_time)],
            write_time,
        );
        assert_eq!(decision, [Decision::Hash(Some(0))]);
        let mut unknown = stat("a.go", 1, old_time);
        unknown.identity = None;
        let (decision, _) = compare(&[without_identity], &[unknown], write_time);
        assert_eq!(decision, [Decision::Hash(Some(0))]);
        let mut recent_record = original;
        recent_record.identity = recent.identity;
        let (decision, _) = compare(&[recent_record], &[recent], write_time);
        assert_eq!(decision, [Decision::Hash(Some(0))]);
    }

    #[test]
    fn empty_change_set_needs_no_writes() {
        let mut changes = ChangeSet::default();
        assert!(changes.is_empty());
        changes.records_changed = true;
        assert!(!changes.is_empty());
        assert!(!changes.has_content_changes());
        changes.deleted.push("x".to_owned());
        assert!(changes.has_content_changes());
    }
}
