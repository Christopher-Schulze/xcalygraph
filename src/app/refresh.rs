//! Index refresh: lock, scan, extract changed files, rewrite changed shards, file table and meta
//! (`docs/spec.md` section 4).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use rayon::prelude::*;

use super::error::AppError;
use super::workspace::Workspace;
use crate::domain::corpus::{ChangeSet, FileRecord};
use crate::domain::facts::{Coverage, Facts};
use crate::domain::hash::FastMap;
use crate::extract;
use crate::infra::lock::{self, WriterLock};
use crate::infra::scan;
use crate::infra::store::container::{Kind, OpenError};
use crate::infra::store::derived;
use crate::infra::store::{self, Meta, SHARDS};
use crate::resolve;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Refresh under the writer lock when anything changed.
    Write,
    /// Never write; fail with exit 5 when the index is missing or stale.
    ReadOnly,
}

/// What a refresh did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub files: usize,
    pub changed: usize,
    pub deleted: usize,
    pub bytes_written: u64,
    pub rebuilt: bool,
}

pub fn refresh(workspace: &Workspace, mode: Mode) -> Result<Report, AppError> {
    let store = &workspace.store;
    if mode == Mode::ReadOnly {
        let (stored, meta) = stored_state(workspace).ok_or(AppError::Stale("index missing"))?;
        if meta.binary_version != derive_version() {
            return Err(AppError::Stale("index stale"));
        }
        let changes = scan::scan(
            &workspace.root,
            &workspace.excluded(),
            &stored,
            meta.write_time_ns,
        )
        .map_err(AppError::io("scan source"))?;
        if changes.has_content_changes() {
            return Err(AppError::Stale("index stale"));
        }
        return Ok(Report {
            files: changes.records.len(),
            ..Report::default()
        });
    }
    store.create_dirs().map_err(AppError::index_io(format!(
        "create index {}",
        store.dir.display()
    )))?;
    let _lock =
        WriterLock::acquire(&store.dir, lock::WAIT).map_err(AppError::index_io("lock index"))?;
    let previous = stored_state(workspace);
    let rebuilt = previous.is_none();
    let (stored, write_time_ns) = previous.map_or((Vec::new(), 0), |(stored, meta)| {
        (stored, meta.write_time_ns)
    });
    let scan_start = now_ns();
    let changes = scan::scan(
        &workspace.root,
        &workspace.excluded(),
        &stored,
        write_time_ns,
    )
    .map_err(AppError::io("scan source"))?;
    let mut report = Report {
        files: changes.records.len(),
        changed: changes.changed.len(),
        deleted: changes.deleted.len(),
        rebuilt,
        bytes_written: 0,
    };
    let derived_valid = derived_is_valid(workspace);
    if changes.is_empty() && !rebuilt && derived_valid {
        return Ok(report);
    }
    let meta = Meta {
        extraction_version: extract::extraction_version(),
        binary_version: derive_version(),
        root: workspace.root.to_string_lossy().into_owned(),
        write_time_ns: scan_start,
    };
    if changes.has_content_changes() || rebuilt || !derived_valid {
        report.bytes_written += invalidate_meta(workspace, &meta)?;
    }
    report.bytes_written += write_shards(workspace, &changes, rebuilt)?;
    if changes.has_content_changes() || rebuilt || !derived_valid {
        report.bytes_written += write_derived(workspace, &changes.records)?;
    }
    report.bytes_written += store
        .write_files(&changes.records)
        .map_err(AppError::index_io("write file table"))?;
    report.bytes_written += store
        .write_meta(&meta)
        .map_err(AppError::index_io("write meta"))?;
    Ok(report)
}

/// Binary version plus resolver version: derived data is only valid for the build that wrote it.
fn derive_version() -> String {
    format!(
        "{}+r{}",
        env!("CARGO_PKG_VERSION"),
        resolve::RESOLVE_VERSION
    )
}

/// Cheap validity for the refresh decision: written by this build and present with a valid
/// header. The full checksum is verified when a query opens the file; a corrupt file is then
/// re-derived by [`rederive`].
fn derived_is_valid(workspace: &Workspace) -> bool {
    workspace
        .store
        .read_meta()
        .is_ok_and(|meta| meta.binary_version == derive_version())
        && workspace.store.has_valid_header("derived", Kind::Derived)
}

/// Rebuilds `derived` from the stored facts, under the writer lock.
pub fn rederive(workspace: &Workspace) -> Result<(), AppError> {
    let _lock = WriterLock::acquire(&workspace.store.dir, lock::WAIT)
        .map_err(AppError::index_io("lock index"))?;
    let (records, meta) = stored_state(workspace)
        .ok_or_else(|| AppError::Invalid("index changed during recovery; retry".to_owned()))?;
    invalidate_meta(workspace, &meta)?;
    write_derived(workspace, &records)?;
    workspace
        .store
        .write_meta(&meta)
        .map_err(AppError::index_io("finish index recovery"))?;
    Ok(())
}

/// Invalidate the current generation before changing any of its shards.
fn invalidate_meta(workspace: &Workspace, meta: &Meta) -> Result<u64, AppError> {
    let mut pending = meta.clone();
    pending.extraction_version ^= 1;
    pending.binary_version.clear();
    workspace
        .store
        .write_meta(&pending)
        .map_err(AppError::index_io("begin index update"))
}

/// Resolves the facts of every file and writes the `derived` file.
fn write_derived(workspace: &Workspace, records: &[FileRecord]) -> Result<u64, AppError> {
    let mut needed: Vec<Vec<&FileRecord>> = (0..SHARDS).map(|_| Vec::new()).collect();
    for record in records.iter().filter(|record| record.role.is_extracted()) {
        needed[store::shard_of(&record.path)].push(record);
    }
    let shards = needed
        .into_par_iter()
        .enumerate()
        .map(|(shard, files)| -> Result<_, AppError> {
            if files.is_empty() {
                return Ok((Vec::new(), 0));
            }
            if let Ok(entries) = workspace.store.read_shard(shard)
                && entries.len() == files.len()
                && entries
                    .iter()
                    .zip(&files)
                    .all(|((path, content_hash, _), file)| {
                        path == &file.path && *content_hash == file.hash
                    })
            {
                return Ok((
                    entries
                        .into_iter()
                        .map(|(path, _, facts)| (path, facts))
                        .collect(),
                    0,
                ));
            }
            let facts: Vec<Facts> = files
                .par_iter()
                .map(|record| extract_file(workspace, record))
                .collect::<Result<_, _>>()?;
            let entries: Vec<(&str, u128, &Facts)> = files
                .iter()
                .zip(&facts)
                .map(|(record, facts)| (record.path.as_str(), record.hash, facts))
                .collect();
            let written = workspace
                .store
                .write_shard(shard, &entries)
                .map_err(AppError::index_io(format!("recover fact shard {shard:02}")))?;
            Ok((
                files
                    .into_iter()
                    .map(|record| record.path.clone())
                    .zip(facts)
                    .collect(),
                written,
            ))
        })
        .collect::<Vec<_>>();
    let mut by_path = FastMap::default();
    let mut recovered_bytes = 0;
    for shard in shards {
        let (entries, written) = shard?;
        by_path.extend(entries);
        recovered_bytes += written;
    }
    let facts: Vec<Option<Facts>> = records
        .iter()
        .map(|record| {
            if !record.role.is_extracted() {
                return Ok(None);
            }
            by_path.remove(&record.path).map(Some).ok_or_else(|| {
                AppError::Invalid(format!(
                    "fact missing for {}; cannot rebuild derived index",
                    record.path
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    let graph = resolve::build(&resolve::Input {
        files: records,
        facts: &facts,
    });
    // Freeing the facts of every file costs as much as building `derived`: overlap the two.
    let ((), image) = rayon::join(
        move || facts.into_par_iter().for_each(drop),
        || derived::build(&graph),
    );
    let ((), written) = rayon::join(
        move || drop(graph),
        || workspace.store.write_container("derived", &image),
    );
    written
        .map(|bytes| bytes + recovered_bytes)
        .map_err(AppError::index_io("write derived data"))
}

/// Stored file table and meta when both are valid and current; `None` forces a full rebuild.
fn stored_state(workspace: &Workspace) -> Option<(Vec<FileRecord>, Meta)> {
    let meta = workspace.store.read_meta().ok()?;
    if meta.binary_version.is_empty() || meta.extraction_version != extract::extraction_version() {
        return None;
    }
    let files = workspace.store.read_files().ok()?;
    Some((files, meta))
}

/// Rewrites every shard holding a changed or deleted file; a shard that fails to open is rebuilt
/// from all of its files.
fn write_shards(
    workspace: &Workspace,
    changes: &ChangeSet,
    rebuilt: bool,
) -> Result<u64, AppError> {
    let store = &workspace.store;
    let mut affected: BTreeSet<usize> = changes
        .changed
        .iter()
        .map(|&index| store::shard_of(&changes.records[index].path))
        .chain(changes.deleted.iter().map(|path| store::shard_of(path)))
        .collect();
    if rebuilt {
        affected = (0..SHARDS).collect();
    }
    let changed: BTreeSet<&str> = changes
        .changed
        .iter()
        .map(|&index| changes.records[index].path.as_str())
        .collect();
    let mut members: BTreeMap<usize, Vec<&FileRecord>> = BTreeMap::new();
    for record in changes
        .records
        .iter()
        .filter(|record| record.role.is_extracted())
    {
        let shard = store::shard_of(&record.path);
        if affected.contains(&shard) {
            members.entry(shard).or_default().push(record);
        }
    }
    // Shards are independent: extracting, deflating and writing them in parallel keeps every
    // core busy across shard boundaries (a full build touches all 64).
    let work: Vec<(usize, Vec<&FileRecord>)> = affected
        .into_iter()
        .map(|shard| (shard, members.remove(&shard).unwrap_or_default()))
        .collect();
    let written: Vec<Result<u64, AppError>> = work
        .into_par_iter()
        .map(|(shard, files)| {
            let mut old: HashMap<String, (u128, Facts)> = if rebuilt {
                HashMap::new()
            } else {
                match store.read_shard(shard) {
                    Ok(entries)
                        if entries.len() == files.len()
                            && entries
                                .iter()
                                .zip(&files)
                                .all(|((path, _, _), file)| path == &file.path) =>
                    {
                        entries
                            .into_iter()
                            .map(|(path, content_hash, facts)| (path, (content_hash, facts)))
                            .collect()
                    }
                    Err(OpenError::Missing | OpenError::Invalid(_) | OpenError::Io(_)) => {
                        HashMap::new()
                    }
                    Ok(_) => HashMap::new(),
                }
            };
            let reusable: Vec<Option<Facts>> = files
                .iter()
                .map(|record| match old.remove(&record.path) {
                    Some((content_hash, facts))
                        if content_hash == record.hash
                            && !changed.contains(record.path.as_str()) =>
                    {
                        Some(facts)
                    }
                    _ => None,
                })
                .collect();
            let facts: Vec<Facts> = files
                .par_iter()
                .zip(reusable.into_par_iter())
                .map(|(record, cached)| match cached {
                    Some(facts) => Ok(facts),
                    None => extract_file(workspace, record),
                })
                .collect::<Result<_, AppError>>()?;
            let entries: Vec<(&str, u128, &Facts)> = files
                .iter()
                .zip(facts.iter())
                .map(|(record, facts)| (record.path.as_str(), record.hash, facts))
                .collect();
            store
                .write_shard(shard, &entries)
                .map_err(AppError::index_io(format!("write fact shard {shard:02}")))
        })
        .collect();
    written.into_iter().sum()
}

fn extract_file(workspace: &Workspace, record: &FileRecord) -> Result<Facts, AppError> {
    let content = match std::fs::read(workspace.root.join(&record.path)) {
        Ok(content) => content,
        // Other files stay answerable; the coverage note names this one.
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return Ok(Facts {
                coverage: Coverage::Unreadable,
                ..Facts::default()
            });
        }
        Err(error) => return Err(AppError::io(format!("read source {}", record.path))(error)),
    };
    if xxhash_rust::xxh3::xxh3_128(&content) != record.hash {
        return Err(AppError::Invalid(format!(
            "source changed while indexing {}; retry",
            record.path
        )));
    }
    // A parser fault on one malformed file costs that file's facts, never the index.
    Ok(
        std::panic::catch_unwind(|| extract::extract(record, &content)).unwrap_or_else(|_| Facts {
            coverage: Coverage::Failed,
            ..Facts::default()
        }),
    )
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::store::Store;
    use crate::infra::testdir::TestDir;

    fn workspace(dir: &TestDir) -> Workspace {
        Workspace {
            root: dir.path().join("root"),
            store: Store::new(dir.path().join("index")),
            default_base: None,
        }
    }

    #[test]
    fn first_refresh_builds_then_quiet_refresh_writes_nothing()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("refresh")?;
        dir.write("root/a.go", "package a\n")?;
        dir.write("root/docs/x.md", "# X\n")?;
        let workspace = workspace(&dir);
        assert!(matches!(
            refresh(&workspace, Mode::ReadOnly),
            Err(AppError::Stale("index missing"))
        ));
        let first = refresh(&workspace, Mode::Write)?;
        assert!(first.rebuilt && first.files == 2 && first.bytes_written > 0);
        let quiet_or_racy = refresh(&workspace, Mode::Write)?;
        assert_eq!(quiet_or_racy.changed, 0);
        assert!(refresh(&workspace, Mode::ReadOnly).is_ok());
        let shard = store::shard_of("a.go");
        let entries = workspace
            .store
            .read_shard(shard)
            .map_err(|e| e.to_string())?;
        assert!(
            entries
                .iter()
                .any(|(path, _, facts)| path == "a.go" && facts.lines == 1)
        );
        Ok(())
    }

    #[test]
    fn read_only_rejects_a_different_binary_or_resolver_version()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("refresh-read-only-version")?;
        dir.write("root/a.go", "package a\n")?;
        let workspace = workspace(&dir);
        refresh(&workspace, Mode::Write)?;
        let mut meta = workspace
            .store
            .read_meta()
            .map_err(|error| error.to_string())?;
        meta.binary_version.push_str("-different");
        workspace.store.write_meta(&meta)?;
        assert!(matches!(
            refresh(&workspace, Mode::ReadOnly),
            Err(AppError::Stale("index stale"))
        ));
        Ok(())
    }

    #[test]
    fn read_only_rejects_a_different_extraction_version() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = TestDir::new("refresh-read-only-extraction")?;
        dir.write("root/a.go", "package a\n")?;
        let workspace = workspace(&dir);
        refresh(&workspace, Mode::Write)?;
        let mut meta = workspace
            .store
            .read_meta()
            .map_err(|error| error.to_string())?;
        meta.extraction_version += 1;
        workspace.store.write_meta(&meta)?;
        assert!(matches!(
            refresh(&workspace, Mode::ReadOnly),
            Err(AppError::Stale("index missing"))
        ));
        Ok(())
    }

    #[test]
    fn changes_rewrite_only_affected_shards_and_read_only_sees_staleness()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("refresh-change")?;
        for index in 0..40 {
            dir.write(&format!("root/f{index}.go"), "package f\n")?;
        }
        let workspace = workspace(&dir);
        refresh(&workspace, Mode::Write)?;
        dir.write("root/f3.go", "package f\n\nfunc X() {}\n")?;
        assert!(matches!(
            refresh(&workspace, Mode::ReadOnly),
            Err(AppError::Stale("index stale"))
        ));
        let changed_shard = store::shard_of("f3.go");
        let other = (0..40)
            .map(|index| format!("f{index}.go"))
            .find(|path| store::shard_of(path) != changed_shard)
            .ok_or("no second shard")?;
        let other_path = workspace
            .store
            .dir
            .join(format!("facts/{:02}", store::shard_of(&other)));
        let before = std::fs::metadata(&other_path)?.modified()?;
        let report = refresh(&workspace, Mode::Write)?;
        assert_eq!(
            (report.changed, report.deleted, report.rebuilt),
            (1, 0, false)
        );
        assert_eq!(std::fs::metadata(&other_path)?.modified()?, before);
        let entries = workspace
            .store
            .read_shard(changed_shard)
            .map_err(|e| e.to_string())?;
        assert!(
            entries
                .iter()
                .any(|(path, _, facts)| path == "f3.go" && facts.lines == 3)
        );
        std::fs::remove_file(dir.path().join("root/f3.go"))?;
        let report = refresh(&workspace, Mode::Write)?;
        assert_eq!(report.deleted, 1);
        let entries = workspace
            .store
            .read_shard(changed_shard)
            .map_err(|e| e.to_string())?;
        assert!(entries.iter().all(|(path, _, _)| path != "f3.go"));
        Ok(())
    }

    #[test]
    fn corrupt_shard_is_rebuilt_from_its_files() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("refresh-corrupt")?;
        dir.write("root/a.go", "package a\n")?;
        dir.write("root/b.go", "package b\n")?;
        let workspace = workspace(&dir);
        refresh(&workspace, Mode::Write)?;
        let shard = store::shard_of("a.go");
        std::fs::write(
            workspace.store.dir.join(format!("facts/{shard:02}")),
            b"garbage",
        )?;
        dir.write("root/a.go", "package a\n\n")?;
        refresh(&workspace, Mode::Write)?;
        let entries = workspace
            .store
            .read_shard(shard)
            .map_err(|e| e.to_string())?;
        assert!(
            entries
                .iter()
                .any(|(path, _, facts)| path == "a.go" && facts.lines == 2)
        );
        Ok(())
    }

    #[test]
    fn corrupt_file_table_triggers_full_rebuild() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("refresh-rebuild")?;
        dir.write("root/a.go", "package a\n")?;
        let workspace = workspace(&dir);
        refresh(&workspace, Mode::Write)?;
        std::fs::write(workspace.store.dir.join("files"), b"x")?;
        let report = refresh(&workspace, Mode::Write)?;
        assert!(report.rebuilt);
        assert_eq!(
            workspace
                .store
                .read_files()
                .map_err(|e| e.to_string())?
                .len(),
            1
        );
        Ok(())
    }

    #[test]
    fn vanished_source_fails_extraction_instead_of_writing_empty_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("refresh-source-error")?;
        dir.write("root/a.go", "package a\n")?;
        let workspace = workspace(&dir);
        refresh(&workspace, Mode::Write)?;
        let records = workspace.store.read_files().map_err(|e| e.to_string())?;
        let shard = workspace
            .store
            .dir
            .join(format!("facts/{:02}", store::shard_of("a.go")));
        let before = std::fs::read(&shard)?;
        std::fs::remove_file(workspace.root.join("a.go"))?;
        let changes = ChangeSet {
            records,
            changed: vec![0],
            ..ChangeSet::default()
        };
        let error = write_shards(&workspace, &changes, false)
            .err()
            .ok_or("expected source error")?;
        assert!(error.to_string().contains("read source a.go"));
        assert!(std::fs::read(shard)? == before);
        Ok(())
    }

    #[test]
    fn source_changed_after_scan_does_not_publish_mismatched_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("refresh-source-race")?;
        dir.write("root/a.go", "package a\n")?;
        let workspace = workspace(&dir);
        refresh(&workspace, Mode::Write)?;
        let records = workspace.store.read_files().map_err(|e| e.to_string())?;
        let shard = workspace
            .store
            .dir
            .join(format!("facts/{:02}", store::shard_of("a.go")));
        let before = std::fs::read(&shard)?;
        dir.write("root/a.go", "package changed\n")?;
        let changes = ChangeSet {
            records,
            changed: vec![0],
            ..ChangeSet::default()
        };
        let error = write_shards(&workspace, &changes, false)
            .err()
            .ok_or("expected race error")?;
        assert!(
            error
                .to_string()
                .contains("source changed while indexing a.go")
        );
        assert!(std::fs::read(shard)? == before);
        Ok(())
    }

    #[test]
    fn incomplete_generation_is_rejected_even_if_extraction_version_matches()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("refresh-incomplete-version")?;
        dir.write("root/a.go", "package a\n")?;
        let workspace = workspace(&dir);
        refresh(&workspace, Mode::Write)?;
        let mut meta = workspace.store.read_meta().map_err(|e| e.to_string())?;
        meta.binary_version.clear();
        workspace.store.write_meta(&meta)?;
        assert!(stored_state(&workspace).is_none());
        Ok(())
    }
}
