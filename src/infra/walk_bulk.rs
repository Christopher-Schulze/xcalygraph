//! macOS directory walk over `getattrlistbulk`: one call returns a directory's names and metadata,
//! so an unchanged tree costs no per-file `lstat` and no failed probes for rule files. The rules
//! and pruning match [`super::walk`]; anything unusual returns `None` and leaves the tree to the
//! portable walker: a `.git` file, an unreadable or invalid rule file, an entry-level error, a file
//! system without bulk attributes.

use std::ffi::{CString, OsStr};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::slice::ParallelSliceMut;

use super::walk::PRUNED_NAMES;
use crate::domain::corpus::{FileIdentity, Stat};

const ATTR_CMN_ERROR: libc::attrgroup_t = 0x2000_0000;
const VREG: u32 = 1;
const VDIR: u32 = 2;
const BUFFER_BYTES: usize = 128 * 1024;

/// The rule files of one directory, linked to those above it.
struct Level {
    parent: Option<Arc<Level>>,
    custom: Option<Gitignore>,
    git: Option<Gitignore>,
    exclude: Option<Gitignore>,
    has_git: bool,
}

impl Level {
    /// `.xcalygraphignore` wins over `.gitignore`, which wins over `.git/info/exclude`; within
    /// one kind the deepest directory decides, and Git's own rules stop at the repository root.
    fn matched(&self, path: &Path, is_dir: bool) -> Match<()> {
        let (mut custom, mut git, mut exclude) = (Match::None, Match::None, Match::None);
        let mut saw_git = false;
        let mut level = Some(self);
        while let Some(current) = level {
            let probe = |rules: &Option<Gitignore>, found: &mut Match<()>| {
                if found.is_none()
                    && let Some(rules) = rules
                {
                    *found = rules.matched(path, is_dir).map(|_| ());
                }
            };
            probe(&current.custom, &mut custom);
            if !saw_git {
                probe(&current.git, &mut git);
                probe(&current.exclude, &mut exclude);
            }
            saw_git |= current.has_git;
            level = current.parent.as_deref();
        }
        custom.or(git).or(exclude)
    }

    fn has_rules(&self) -> bool {
        let mut level = Some(self);
        while let Some(current) = level {
            if current.custom.is_some() || current.git.is_some() || current.exclude.is_some() {
                return true;
            }
            level = current.parent.as_deref();
        }
        false
    }
}

/// The rule files `listed` says exist in `dir`; `None` sends the tree to the portable walker.
fn level_of(
    dir: &Path,
    parent: Option<Arc<Level>>,
    gitignore: bool,
    custom: bool,
    dot_git: Option<bool>,
) -> Option<Arc<Level>> {
    let rules = |file: PathBuf| -> Option<Gitignore> {
        let mut builder = GitignoreBuilder::new(dir);
        if builder.add(file).is_some() {
            return None;
        }
        builder.build().ok()
    };
    let exclude = match dot_git {
        // A `.git` file points to a linked work tree: the portable walker resolves it.
        Some(false) => return None,
        Some(true) => {
            let file = dir.join(".git/info/exclude");
            if file.is_file() {
                Some(rules(file)?)
            } else {
                None
            }
        }
        None => None,
    };
    Some(Arc::new(Level {
        parent,
        custom: if custom {
            Some(rules(dir.join(".xcalygraphignore"))?)
        } else {
            None
        },
        git: if gitignore {
            Some(rules(dir.join(".gitignore"))?)
        } else {
            None
        },
        exclude,
        has_git: dot_git.is_some(),
    }))
}

/// Rule files of the root's ancestors, outermost first.
fn ancestor_levels(root: &Path) -> Option<Option<Arc<Level>>> {
    let mut ancestors: Vec<&Path> = root.ancestors().skip(1).collect();
    ancestors.reverse();
    let mut level = None;
    for dir in ancestors {
        let git = std::fs::symlink_metadata(dir.join(".git"))
            .ok()
            .map(|metadata| metadata.is_dir());
        let gitignore = dir.join(".gitignore").is_file();
        let custom = dir.join(".xcalygraphignore").is_file();
        if git.is_some() || gitignore || custom {
            level = Some(level_of(dir, level, gitignore, custom, git)?);
        }
    }
    Some(level)
}

struct Walk<'a> {
    excluded: &'a [PathBuf],
    global: Gitignore,
    found: Mutex<Vec<Stat>>,
    fallback: AtomicBool,
}

impl Walk<'_> {
    fn give_up(&self) {
        self.fallback.store(true, Ordering::Relaxed);
    }

    fn visit<'scope>(
        &'scope self,
        scope: &rayon::Scope<'scope>,
        dir: PathBuf,
        relative: String,
        parent: Option<Arc<Level>>,
        parent_has_go_mod: bool,
    ) {
        if self.fallback.load(Ordering::Relaxed) {
            return;
        }
        let Listing { entries, denied } = match list(&dir) {
            Ok(listing) => listing,
            Err(_) => return self.give_up(),
        };
        if denied {
            if relative.is_empty() {
                return self.give_up();
            }
            self.push(vec![Stat {
                path: format!("{relative}/"),
                size: 0,
                mtime_ns: 0,
                identity: None,
            }]);
            return;
        }
        let has = |name: &[u8]| entries.iter().any(|entry| entry.name == name);
        let is_root = relative.is_empty();
        if !is_root
            && (has(b"CACHEDIR.TAG")
                || has(b"pyvenv.cfg")
                || (dir.file_name() == Some(OsStr::new("vendor"))
                    && parent_has_go_mod
                    && has(b"modules.txt")))
        {
            return;
        }
        let dot_git = entries
            .iter()
            .find(|entry| entry.name == b".git")
            .map(|entry| entry.kind == VDIR);
        let (gitignore, custom) = (has(b".gitignore"), has(b".xcalygraphignore"));
        let level = if dot_git.is_some() || gitignore || custom {
            let Some(level) = level_of(&dir, parent, gitignore, custom, dot_git) else {
                return self.give_up();
            };
            Some(level)
        } else {
            parent
        };
        let has_go_mod = has(b"go.mod");
        let rules = level.as_deref().is_some_and(Level::has_rules);
        let mut files = Vec::new();
        for entry in entries {
            if entry.kind != VREG && entry.kind != VDIR {
                continue;
            }
            let name = OsStr::from_bytes(&entry.name);
            let Some(text) = name.to_str() else {
                continue;
            };
            let is_dir = entry.kind == VDIR;
            if is_dir && PRUNED_NAMES.contains(&text) {
                continue;
            }
            let path = dir.join(name);
            if is_dir && self.excluded.contains(&path) {
                continue;
            }
            let matched = if rules {
                level
                    .as_deref()
                    .map_or(Match::None, |level| level.matched(&path, is_dir))
            } else {
                Match::None
            };
            let ignored = if matched.is_none() {
                self.global.matched(&path, is_dir).is_ignore()
            } else {
                matched.is_ignore()
            };
            if ignored {
                continue;
            }
            let child = if relative.is_empty() {
                text.to_owned()
            } else {
                format!("{relative}/{text}")
            };
            if is_dir {
                let level = level.clone();
                scope.spawn(move |scope| self.visit(scope, path, child, level, has_go_mod));
            } else {
                files.push(Stat {
                    path: child,
                    size: entry.size,
                    mtime_ns: entry.mtime_ns,
                    identity: entry.identity,
                });
            }
        }
        self.push(files);
    }

    fn push(&self, mut files: Vec<Stat>) {
        self.found
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .append(&mut files);
    }
}

/// Files below `root`, sorted by path; `None` when the portable walker has to decide.
pub fn walk(root: &Path, excluded: &[PathBuf]) -> Option<Vec<Stat>> {
    let parent = ancestor_levels(root)?;
    // Anchored patterns of the global gitignore are relative to the root, not the working directory.
    let (global, error) = GitignoreBuilder::new(root).build_global();
    if error.is_some() {
        return None;
    }
    let walk = Walk {
        excluded,
        global,
        found: Mutex::new(Vec::new()),
        fallback: AtomicBool::new(false),
    };
    // Directory syscalls wait on the kernel, and efficiency cores slow the shared queue down.
    let start = root.to_path_buf();
    match rayon::ThreadPoolBuilder::new()
        .num_threads(performance_cores())
        .build()
    {
        Ok(pool) => pool.scope(|scope| walk.visit(scope, start, String::new(), parent, false)),
        Err(_) => rayon::scope(|scope| walk.visit(scope, start, String::new(), parent, false)),
    }
    if walk.fallback.load(Ordering::Relaxed) {
        return None;
    }
    let mut files = walk
        .found
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    files.par_sort_unstable_by(|left, right| left.path.cmp(&right.path));
    Some(files)
}

/// Performance cores of this machine, all logical CPUs where the query fails.
fn performance_cores() -> usize {
    let mut count: libc::c_int = 0;
    let mut size = size_of::<libc::c_int>();
    // SAFETY: the name is NUL-terminated and `count` holds `size` writable bytes.
    let status = unsafe {
        libc::sysctlbyname(
            c"hw.perflevel0.physicalcpu".as_ptr(),
            (&raw mut count).cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    usize::try_from(count)
        .ok()
        .filter(|count| status == 0 && *count > 0)
        .unwrap_or_else(rayon::current_num_threads)
}

struct Entry {
    name: Vec<u8>,
    kind: u32,
    size: u64,
    mtime_ns: i64,
    identity: Option<FileIdentity>,
}

struct Listing {
    entries: Vec<Entry>,
    denied: bool,
}

/// Closes the descriptor on every return path.
struct Directory(libc::c_int);

impl Drop for Directory {
    fn drop(&mut self) {
        // SAFETY: the descriptor came from `open` and is closed once, here.
        unsafe { libc::close(self.0) };
    }
}

thread_local! {
    static BUFFER: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// One directory's entries with their metadata.
fn list(dir: &Path) -> io::Result<Listing> {
    let path = CString::new(dir.as_os_str().to_owned().into_vec())?;
    // SAFETY: `path` is a valid NUL-terminated string; the result is checked.
    let descriptor = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::PermissionDenied {
            return Ok(Listing {
                entries: Vec::new(),
                denied: true,
            });
        }
        return Err(error);
    }
    let directory = Directory(descriptor);
    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS
            | ATTR_CMN_ERROR
            | libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_DEVID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_MODTIME
            | libc::ATTR_CMN_CHGTIME
            | libc::ATTR_CMN_FILEID,
        volattr: 0,
        dirattr: 0,
        fileattr: libc::ATTR_FILE_DATALENGTH,
        forkattr: 0,
    };
    let mut entries = Vec::new();
    BUFFER.with_borrow_mut(|buffer| -> io::Result<()> {
        buffer.resize(BUFFER_BYTES / 8, 0);
        loop {
            // SAFETY: `request` and `buffer` are valid for the sizes given; the kernel writes at
            // most `BUFFER_BYTES` bytes into the 8-byte aligned buffer.
            let count = unsafe {
                libc::getattrlistbulk(
                    directory.0,
                    (&raw mut request).cast(),
                    buffer.as_mut_ptr().cast(),
                    BUFFER_BYTES,
                    0,
                )
            };
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            if count == 0 {
                return Ok(());
            }
            let bytes: &[u8] = bytemuck::cast_slice(&buffer[..]);
            parse(bytes, count as usize, &mut entries)
                .ok_or_else(|| io::Error::other("malformed bulk attributes"))?;
        }
    })?;
    Ok(Listing {
        entries,
        denied: false,
    })
}

/// Reads `count` packed records; `None` on an entry error or a record it cannot prove complete.
fn parse(bytes: &[u8], count: usize, entries: &mut Vec<Entry>) -> Option<()> {
    let mut start = 0;
    for _ in 0..count {
        let length = u32_at(bytes, start)? as usize;
        let record = bytes.get(start..start.checked_add(length)?)?;
        entries.push(parse_record(record)?);
        start += length;
    }
    Some(())
}

fn parse_record(record: &[u8]) -> Option<Entry> {
    let mut offset = 4;
    let common = u32_at(record, offset)?;
    let file = u32_at(record, offset + 12)?;
    offset += size_of::<libc::attribute_set_t>();
    if common & ATTR_CMN_ERROR != 0 {
        if u32_at(record, offset)? != 0 {
            return None;
        }
        offset += 4;
    }
    if common & libc::ATTR_CMN_NAME == 0 {
        return None;
    }
    let name_at = offset;
    let name_offset = i32::from_ne_bytes(array_at(record, offset)?);
    let name_length = u32_at(record, offset + 4)? as usize;
    offset += 8;
    let name_start = name_at.checked_add_signed(name_offset as isize)?;
    let name = record.get(name_start..name_start.checked_add(name_length)?)?;
    let name = name.strip_suffix(&[0]).unwrap_or(name).to_vec();
    let mut device = None;
    if common & libc::ATTR_CMN_DEVID != 0 {
        device = Some(i32::from_ne_bytes(array_at(record, offset)?));
        offset += 4;
    }
    if common & libc::ATTR_CMN_OBJTYPE == 0 {
        return None;
    }
    let kind = u32_at(record, offset)?;
    offset += 4;
    let mut time_at = |present: bool| -> Option<Option<(i64, i64)>> {
        if !present {
            return Some(None);
        }
        let seconds = i64::from_ne_bytes(array_at(record, offset)?);
        let nanos = i64::from_ne_bytes(array_at(record, offset + 8)?);
        offset += 16;
        Some(Some((seconds, nanos)))
    };
    let modified = time_at(common & libc::ATTR_CMN_MODTIME != 0)?;
    let changed = time_at(common & libc::ATTR_CMN_CHGTIME != 0)?;
    let mut inode = None;
    if common & libc::ATTR_CMN_FILEID != 0 {
        inode = Some(u64::from_ne_bytes(array_at(record, offset)?));
        offset += 8;
    }
    let mut size = 0;
    if file & libc::ATTR_FILE_DATALENGTH != 0 {
        size = u64::from_ne_bytes(array_at(record, offset)?);
    } else if kind == VREG {
        return None;
    }
    let (seconds, nanos) = match (kind, modified) {
        (VREG, Some(time)) => time,
        (VREG, None) => return None,
        _ => (0, 0),
    };
    let identity = (|| {
        let (seconds, nanos) = changed?;
        let change_time_ns = seconds.checked_mul(1_000_000_000)?.checked_add(nanos)?;
        let inode = inode.filter(|inode| *inode != 0)?;
        (change_time_ns != 0 && (0..1_000_000_000).contains(&nanos)).then_some(FileIdentity {
            device: device? as u64,
            inode,
            change_time_ns,
        })
    })();
    Some(Entry {
        name,
        kind,
        size,
        mtime_ns: seconds.saturating_mul(1_000_000_000).saturating_add(nanos),
        identity,
    })
}

fn array_at<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    bytes.get(offset..offset.checked_add(N)?)?.try_into().ok()
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    array_at(bytes, offset).map(u32::from_ne_bytes)
}
