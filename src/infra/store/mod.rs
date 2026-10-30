//! Index store: location, meta, file table and fact shards (`docs/spec.md` section 9).

pub mod codec;
pub mod container;
pub mod derived;
pub mod facts_codec;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use xxhash_rust::xxh3::xxh3_64;

use codec::{DecodeError, DecodeResult, Decoder, Encoder};
use container::{Container, Kind, OpenError};

use crate::domain::corpus::FileRecord;
use crate::domain::facts::Facts;
use crate::domain::filetype::FileType;
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
        let container = Container::open(&self.path("meta"), Kind::Meta)?;
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

    /// Facts of every file in the shard, sorted by path.
    pub fn read_shard(&self, shard: usize) -> Result<Vec<(String, Facts)>, OpenError> {
        let container = Container::open(&self.shard_path(shard), Kind::Facts)?;
        let mut decoder = Decoder::new(container.section(0));
        let decode = |decoder: &mut Decoder| -> DecodeResult<Vec<(String, Facts)>> {
            let count = decoder.len()?;
            (0..count)
                .map(|_| Ok((decoder.string()?, facts_codec::decode(decoder)?)))
                .collect()
        };
        decode(&mut decoder).map_err(|_| OpenError::Invalid("facts content"))
    }

    pub fn write_shard(&self, shard: usize, files: &[(&str, &Facts)]) -> io::Result<u64> {
        let mut encoder = Encoder::default();
        encoder.var_usize(files.len());
        for (path, facts) in files {
            encoder.str(path);
            facts_codec::encode(&mut encoder, facts);
        }
        container::write(
            &self.shard_path(shard),
            &container::build(Kind::Facts, &[&encoder.bytes]),
        )
    }

    pub fn read_container(&self, name: &str, kind: Kind) -> Result<Container, OpenError> {
        Container::open(&self.path(name), kind)
    }

    pub fn write_container(&self, name: &str, image: &[u8]) -> io::Result<u64> {
        container::write(&self.path(name), image)
    }

    /// Deletes the whole index directory.
    pub fn drop_all(&self) -> io::Result<bool> {
        match fs::remove_dir_all(&self.dir) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
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
        let records = vec![
            record("src/a.go", Role::Src, Some(FileType::Go)),
            record("src/ab.go", Role::Test, Some(FileType::Go)),
            record("src/é/x", Role::Other, None),
            record("x", Role::Sensitive, None),
        ];
        store.write_files(&records)?;
        assert_eq!(store.read_files().map_err(|e| e.to_string())?, records);
        let facts = Facts {
            lines: 3,
            ..Facts::default()
        };
        store.write_shard(7, &[("src/a.go", &facts)])?;
        assert_eq!(
            store.read_shard(7).map_err(|e| e.to_string())?,
            [("src/a.go".to_owned(), facts)]
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
}
