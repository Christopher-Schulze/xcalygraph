//! Index store: location, meta, file table and fact shards (`docs/spec.md` section 9).

pub mod codec;
pub mod container;
pub mod derived;
pub mod facts_codec;

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use xxhash_rust::xxh3::xxh3_64;

use codec::{DecodeError, DecodeResult, Decoder, Encoder};
use container::{Container, Kind, OpenError};

use crate::domain::corpus::{FileIdentity, FileRecord};
use crate::domain::facts::Facts;
use crate::domain::filetype::FileType;
use crate::domain::history::History;
use crate::domain::role::Role;

pub const SHARDS: usize = 64;

/// Index directory of `root` below `base`: `<root basename>-<XXH3-64 hex of the canonical root>`.
pub fn index_dir(base: &Path, canonical_root: &Path) -> PathBuf {
    let name = canonical_root.file_name().map_or_else(
        || "root".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let hash = xxh3_64(canonical_root.as_os_str().as_encoded_bytes());
    base.join(format!("{name}-{hash:016x}"))
}

pub fn shard_of(path: &str) -> usize {
    (xxh3_64(path.as_bytes()) % SHARDS as u64) as usize
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub extraction_version: u32,
    pub binary_version: String,
    pub root: String,
    pub write_time_ns: i64,
}

/// Paths of the files of one index directory.
#[derive(Debug, Clone)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn create_dirs(&self) -> io::Result<()> {
        fs::create_dir_all(self.dir.join("facts"))
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn shard_path(&self, shard: usize) -> PathBuf {
        self.dir.join("facts").join(format!("{shard:02}"))
    }

    pub fn read_meta(&self) -> Result<Meta, OpenError> {
        self.read_meta_version(container::FORMAT_VERSION)
    }

    /// Reads metadata from a known compatible container version.
    pub fn read_meta_version(&self, version: u32) -> Result<Meta, OpenError> {
        let container = Container::open_version(&self.path("meta"), Kind::Meta, version)?;
        decode_meta(container.section(0)).map_err(|_| OpenError::Invalid("meta content"))
    }

    pub fn write_meta(&self, meta: &Meta) -> io::Result<u64> {
        let mut encoder = Encoder::default();
        encoder.var_u32(meta.extraction_version);
        encoder.str(&meta.binary_version);
        encoder.str(&meta.root);
        encoder.signed(meta.write_time_ns);
        container::write(
            &self.path("meta"),
            &container::build(Kind::Meta, &[&encoder.bytes]),
        )
    }

    pub fn read_files(&self) -> Result<Vec<FileRecord>, OpenError> {
        let container = Container::open(&self.path("files"), Kind::Files)?;
        decode_files(container.section(0)).map_err(|_| OpenError::Invalid("files content"))
    }

    pub fn write_files(&self, records: &[FileRecord]) -> io::Result<u64> {
        let mut encoder = Encoder::default();
        encoder.var_usize(records.len());
        let mut previous = "";
        for record in records {
            // Front coding: shared prefix length with the previous path, then the suffix.
            let shared = common_prefix(previous, &record.path);
            encoder.var_usize(shared);
            encoder.str(&record.path[shared..]);
            encoder.var(record.size);
            encoder.signed(record.mtime_ns);
            if let Some(identity) = record.identity {
                encoder.u8(1);
                encoder.var(identity.device);
                encoder.var(identity.inode);
                encoder.signed(identity.change_time_ns);
            } else {
                encoder.u8(0);
            }
            encoder.u128(record.hash);
            encoder.u8(record.role as u8);
            encoder.u8(record.file_type.map_or(0, |file_type| file_type as u8));
            previous = &record.path;
        }
        container::write(
            &self.path("files"),
            &container::build(Kind::Files, &[&encoder.bytes]),
        )
    }

    /// Source hash and facts of every file in the shard, sorted by path.
    pub fn read_shard(&self, shard: usize) -> Result<Vec<(String, u128, Facts)>, OpenError> {
        let container = Container::open(&self.shard_path(shard), Kind::Facts)?;
        let (Some(table), Some(bytes)) =
            (inflate(container.section(0)), inflate(container.section(1)))
        else {
            return Err(OpenError::Invalid("facts compression"));
        };
        let decode = || -> DecodeResult<Vec<(String, u128, Facts)>> {
            let mut decoder = Decoder::with_table(&bytes, &table)?;
            let count = decoder.len()?;
            (0..count)
                .map(|_| {
                    Ok((
                        decoder.string()?,
                        decoder.u128()?,
                        facts_codec::decode(&mut decoder)?,
                    ))
                })
                .collect()
        };
        decode().map_err(|_| OpenError::Invalid("facts content"))
    }

    pub fn write_shard(&self, shard: usize, files: &[(&str, u128, &Facts)]) -> io::Result<u64> {
        // Section 0: shard-local string table; section 1: per-file facts referencing it. Both are
        // deflated: shards are read whole and in parallel, so inflating costs little.
        let mut encoder = Encoder::with_table();
        encoder.var_usize(files.len());
        for (path, content_hash, facts) in files {
            encoder.str(path);
            encoder.u128(*content_hash);
            facts_codec::encode(&mut encoder, facts);
        }
        let table = deflate(&encoder.table_bytes())?;
        let bytes = deflate(&encoder.bytes)?;
        container::write(
            &self.shard_path(shard),
            &container::build(Kind::Facts, &[&table, &bytes]),
        )
    }

    /// Cached git history analyses if they were computed at `head`.
    pub fn read_history(&self, head: &str) -> Option<History> {
        let container = Container::open(&self.path("git"), Kind::Git).ok()?;
        let decode = || -> DecodeResult<Option<History>> {
            let mut decoder = Decoder::with_table(container.section(1), container.section(0))?;
            if decoder.str()? != head {
                return Ok(None);
            }
            let mut history = History::default();
            for _ in 0..decoder.len()? {
                let path = decoder.string()?;
                history.churn.insert(path, decoder.var_u32()?);
            }
            for _ in 0..decoder.len()? {
                history
                    .co_change
                    .push((decoder.string()?, decoder.string()?, decoder.var_u32()?));
            }
            Ok(Some(history))
        };
        decode().ok().flatten()
    }

    pub fn write_history(&self, head: &str, history: &History) -> io::Result<u64> {
        let mut encoder = Encoder::with_table();
        encoder.str(head);
        encoder.var_usize(history.churn.len());
        for (path, count) in &history.churn {
            encoder.str(path);
            encoder.var_u32(*count);
        }
        encoder.var_usize(history.co_change.len());
        for (left, right, count) in &history.co_change {
            encoder.str(left);
            encoder.str(right);
            encoder.var_u32(*count);
        }
        let table = encoder.table_bytes();
        container::write(
            &self.path("git"),
            &container::build(Kind::Git, &[&table, &encoder.bytes]),
        )
    }

    pub fn read_container(&self, name: &str, kind: Kind) -> Result<Container, OpenError> {
        Container::open(&self.path(name), kind)
    }

    /// Header check without reading the payload: magic, format version, kind and length.
    pub fn has_valid_header(&self, name: &str, kind: Kind) -> bool {
        container::header_matches(&self.path(name), kind)
    }

    pub fn write_container(&self, name: &str, image: &[u8]) -> io::Result<u64> {
        container::write(&self.path(name), image)
    }

    /// Stamps the last use (for `ls`), at most once an hour so queries stay write-free.
    pub fn mark_used(&self) {
        let stamp = self.path("used");
        match fs::symlink_metadata(&stamp) {
            Ok(metadata) if valid_stamp(&metadata) => {
                let recent = metadata.modified().is_ok_and(|modified| {
                    modified
                        .elapsed()
                        .is_ok_and(|age| age < std::time::Duration::from_secs(3600))
                });
                if !recent {
                    #[cfg(unix)]
                    let _ = touch_stamp(&stamp, &metadata);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let _ = fs::File::create_new(&stamp);
            }
            _ => {}
        }
    }

    /// Last use: the newer of the use stamp and the last index write.
    pub fn last_used(&self) -> Option<std::time::SystemTime> {
        let used = fs::symlink_metadata(self.path("used"))
            .ok()
            .filter(valid_stamp)
            .and_then(|metadata| metadata.modified().ok());
        let written = fs::metadata(self.path("meta"))
            .and_then(|metadata| metadata.modified())
            .ok();
        used.into_iter().chain(written).max()
    }

    /// Removes temporary files of killed writers. Every store write holds the writer lock, so
    /// the caller, holding it too, sees no temporary file in use. Best effort: a leftover that
    /// cannot be removed costs disk only, never correctness.
    pub fn remove_temp_leftovers(&self) {
        for dir in [self.dir.clone(), self.dir.join("facts")] {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with('.') && name.contains(".tmp.") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    pub fn drop_all(&self) -> io::Result<bool> {
        match fs::remove_dir_all(&self.dir) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}

fn valid_stamp(metadata: &fs::Metadata) -> bool {
    metadata.len() == 0 && super::file_access::is_unshared_regular(metadata)
}

#[cfg(unix)]
fn touch_stamp(path: &Path, previous: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    if previous.nlink() != 1 {
        return Ok(());
    }
    let file = super::file_access::open_existing_regular(path, false)?;
    let current = file.metadata()?;
    if valid_stamp(&current) && (current.dev(), current.ino()) == (previous.dev(), previous.ino()) {
        file.set_modified(std::time::SystemTime::now())?;
    }
    Ok(())
}

fn deflate(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = flate2::write::DeflateEncoder::new(
        Vec::with_capacity(bytes.len() / 2),
        flate2::Compression::fast(),
    );
    encoder.write_all(bytes)?;
    encoder.finish()
}

fn inflate(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes.len() * 3);
    flate2::read::DeflateDecoder::new(bytes)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

fn common_prefix(left: &str, right: &str) -> usize {
    let mut shared = left
        .bytes()
        .zip(right.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !right.is_char_boundary(shared) {
        shared -= 1;
    }
    shared
}

fn decode_meta(bytes: &[u8]) -> DecodeResult<Meta> {
    let mut decoder = Decoder::new(bytes);
    Ok(Meta {
        extraction_version: decoder.var_u32()?,
        binary_version: decoder.string()?,
        root: decoder.string()?,
        write_time_ns: decoder.signed()?,
    })
}

fn decode_files(bytes: &[u8]) -> DecodeResult<Vec<FileRecord>> {
    let mut decoder = Decoder::new(bytes);
    let count = decoder.len()?;
    let mut records: Vec<FileRecord> = Vec::with_capacity(count);
    for _ in 0..count {
        let shared = decoder.var_usize()?;
        let suffix = decoder.str()?;
        let previous = records.last().map_or("", |record| record.path.as_str());
        let prefix = previous.get(..shared).ok_or(DecodeError("path prefix"))?;
        let path = format!("{prefix}{suffix}");
        let size = decoder.var()?;
        let mtime_ns = decoder.signed()?;
        let identity = match decoder.u8()? {
            0 => None,
            1 => Some(FileIdentity {
                device: decoder.var()?,
                inode: decoder.var()?,
                change_time_ns: decoder.signed()?,
            }),
            _ => return Err(DecodeError("file identity tag")),
        };
        let hash = decoder.u128()?;
        let role = Role::from_u8(decoder.u8()?).ok_or(DecodeError("role"))?;
        let file_type = match decoder.u8()? {
            0 => None,
            value => Some(FileType::from_u8(value).ok_or(DecodeError("file type"))?),
        };
        records.push(FileRecord {
            path,
            size,
            mtime_ns,
            identity,
            hash,
            role,
            file_type,
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::testdir::TestDir;

    fn record(path: &str, role: Role, file_type: Option<FileType>) -> FileRecord {
        FileRecord {
            path: path.to_owned(),
            size: 12,
            mtime_ns: -3,
            identity: Some(FileIdentity {
                device: 42,
                inode: 123,
                change_time_ns: -5,
            }),
            hash: 0xdead_beef,
            role,
            file_type,
        }
    }

    #[test]
    fn meta_files_and_shards_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("store")?;
        let store = Store::new(dir.path().join("index"));
        store.create_dirs()?;
        let meta = Meta {
            extraction_version: 3,
            binary_version: "0.1.0".to_owned(),
            root: "/r".to_owned(),
            write_time_ns: 42,
        };
        store.write_meta(&meta)?;
        assert_eq!(store.read_meta().map_err(|e| e.to_string())?, meta);
        let mut without_identity = record("x", Role::Sensitive, None);
        without_identity.identity = None;
        let records = vec![
            record("src/a.go", Role::Src, Some(FileType::Go)),
            record("src/ab.go", Role::Test, Some(FileType::Go)),
            record("src/é/x", Role::Other, None),
            without_identity,
        ];
        store.write_files(&records)?;
        assert_eq!(store.read_files().map_err(|e| e.to_string())?, records);
        let facts = Facts {
            lines: 3,
            ..Facts::default()
        };
        let content_hash = 0x1122_3344_5566_7788_99aa_bbcc_ddee_ff00;
        store.write_shard(7, &[("src/a.go", content_hash, &facts)])?;
        assert_eq!(
            store.read_shard(7).map_err(|e| e.to_string())?,
            [("src/a.go".to_owned(), content_hash, facts)]
        );
        assert!(matches!(store.read_shard(8), Err(OpenError::Missing)));
        assert!(store.drop_all()?);
        assert!(!store.drop_all()?);
        Ok(())
    }

    #[test]
    fn index_dir_names_root_and_hash() {
        let dir = index_dir(Path::new("/cache"), Path::new("/work/app"));
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert!(
            name.starts_with("app-") && name.len() == "app-".len() + 16,
            "{name}"
        );
        assert_ne!(dir, index_dir(Path::new("/cache"), Path::new("/other/app")));
        assert!(shard_of("a/b.go") < SHARDS);
    }

    #[test]
    fn common_prefix_respects_char_boundaries() {
        assert_eq!(common_prefix("src/é", "src/è"), 4);
        assert_eq!(common_prefix("abc", "abd"), 2);
    }

    #[cfg(unix)]
    #[test]
    fn last_used_ignores_foreign_symlink_and_hard_link() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("store-last-used-links")?;
        let store = Store::new(dir.path().join("index"));
        store.create_dirs()?;
        let meta = Meta {
            extraction_version: 1,
            binary_version: "test".to_owned(),
            root: "/root".to_owned(),
            write_time_ns: 0,
        };
        store.write_meta(&meta)?;
        let expected = fs::metadata(store.dir.join("meta"))?.modified()?;
        let target = dir.path().join("foreign");
        fs::File::create_new(&target)?
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(7200))?;
        let used = store.dir.join("used");
        symlink(&target, &used)?;
        assert_eq!(store.last_used(), Some(expected));
        fs::remove_file(&used)?;
        fs::hard_link(&target, &used)?;
        assert_eq!(store.last_used(), Some(expected));
        Ok(())
    }
}
