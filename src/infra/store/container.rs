//! Index file container: 64-byte header with magic, format version, kind and XXH3-64 checksum,
//! a section table and 64-byte aligned sections (`docs/spec.md` section 9).

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::ops::Range;
use std::path::Path;

use memmap2::Mmap;
use xxhash_rust::xxh3::xxh3_64;

pub const MAGIC: [u8; 8] = *b"XCALYGR\x00";
/// Bumped on every incompatible change of any index file layout.
pub const FORMAT_VERSION: u32 = 5;
const HEADER_LEN: usize = 64;
const ALIGN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
    Meta = 1,
    Files = 2,
    Facts = 3,
    Derived = 4,
    Git = 5,
}

#[derive(Debug)]
pub enum OpenError {
    Missing,
    /// Wrong magic, version, kind, length or checksum: the file must be rebuilt.
    Invalid(&'static str),
    Io(io::Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("index file missing"),
            Self::Invalid(reason) => write!(formatter, "index file invalid: {reason}"),
            Self::Io(error) => write!(formatter, "index file unreadable: {error}"),
        }
    }
}

/// Serializes sections into one container image.
pub fn build(kind: Kind, sections: &[&[u8]]) -> Vec<u8> {
    let table_len = align(sections.len() * 16);
    let mut offset = HEADER_LEN + table_len;
    let mut table = Vec::with_capacity(table_len);
    for section in sections {
        table.extend_from_slice(&(offset as u64).to_le_bytes());
        table.extend_from_slice(&(section.len() as u64).to_le_bytes());
        offset = align(offset + section.len());
    }
    table.resize(table_len, 0);
    let mut image = vec![0u8; HEADER_LEN];
    image.extend_from_slice(&table);
    for section in sections {
        image.extend_from_slice(section);
        image.resize(align(image.len()), 0);
    }
    let checksum = xxh3_64(&image[HEADER_LEN..]);
    let payload_len = (image.len() - HEADER_LEN) as u64;
    image[0..8].copy_from_slice(&MAGIC);
    image[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    image[12..16].copy_from_slice(&(kind as u32).to_le_bytes());
    image[16..20].copy_from_slice(&(sections.len() as u32).to_le_bytes());
    image[24..32].copy_from_slice(&payload_len.to_le_bytes());
    image[32..40].copy_from_slice(&checksum.to_le_bytes());
    image
}

/// Writes the image atomically (temp file in the same directory, then rename), without fsync.
/// An existing file with the same checksum and length is left untouched. Returns bytes written.
pub fn write(path: &Path, image: &[u8]) -> io::Result<u64> {
    if same_header(path, image) {
        return Ok(0);
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("index file path without name"))?;
    let temp = path.with_file_name(format!(".{name}.tmp.{}", std::process::id()));
    let result = File::create(&temp)
        .and_then(|mut file| file.write_all(image))
        .and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map(|()| image.len() as u64)
}

fn same_header(path: &Path, image: &[u8]) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut header = [0u8; HEADER_LEN];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    header[..40] == image[..40]
        && file.metadata().map(|meta| meta.len()).ok() == Some(image.len() as u64)
}

/// True when the file starts with a header of this format version and kind whose recorded
/// length matches the file size.
pub fn header_matches(path: &Path, kind: Kind) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut header = [0u8; HEADER_LEN];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    let mut length = [0u8; 8];
    length.copy_from_slice(&header[24..32]);
    header[0..8] == MAGIC
        && header[8..12] == FORMAT_VERSION.to_le_bytes()
        && header[12..16] == (kind as u32).to_le_bytes()
        && file
            .metadata()
            .is_ok_and(|meta| meta.len() == u64::from_le_bytes(length) + HEADER_LEN as u64)
}

/// A validated, memory-mapped container.
pub struct Container {
    map: Mmap,
    sections: Vec<Range<usize>>,
}

impl Container {
    pub fn open(path: &Path, kind: Kind) -> Result<Self, OpenError> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(OpenError::Missing);
            }
            Err(error) => return Err(OpenError::Io(error)),
        };
        // SAFETY: index files are never modified in place; writers replace them by rename, so the
        // mapped inode stays immutable for the lifetime of this mapping.
        let map = unsafe { Mmap::map(&file) }.map_err(OpenError::Io)?;
        let sections = validate(&map, kind)?;
        Ok(Self { map, sections })
    }

    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Bytes of section `index`; empty when absent.
    pub fn section(&self, index: usize) -> &[u8] {
        self.sections
            .get(index)
            .and_then(|range| self.map.get(range.clone()))
            .unwrap_or_default()
    }
}

fn validate(bytes: &[u8], kind: Kind) -> Result<Vec<Range<usize>>, OpenError> {
    let read_u32 =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let read_u64 = |at: usize| {
        let mut buffer = [0u8; 8];
        buffer.copy_from_slice(&bytes[at..at + 8]);
        u64::from_le_bytes(buffer)
    };
    if bytes.len() < HEADER_LEN {
        return Err(OpenError::Invalid("truncated header"));
    }
    if bytes[0..8] != MAGIC {
        return Err(OpenError::Invalid("magic"));
    }
    if read_u32(8) != FORMAT_VERSION {
        return Err(OpenError::Invalid("format version"));
    }
    if read_u32(12) != kind as u32 {
        return Err(OpenError::Invalid("kind"));
    }
    let count = read_u32(16) as usize;
    if read_u64(24) != (bytes.len() - HEADER_LEN) as u64 {
        return Err(OpenError::Invalid("length"));
    }
    if read_u64(32) != xxh3_64(&bytes[HEADER_LEN..]) {
        return Err(OpenError::Invalid("checksum"));
    }
    if HEADER_LEN + count * 16 > bytes.len() {
        return Err(OpenError::Invalid("section table"));
    }
    (0..count)
        .map(|index| {
            let entry = HEADER_LEN + index * 16;
            let start =
                usize::try_from(read_u64(entry)).map_err(|_| OpenError::Invalid("offset"))?;
            let len =
                usize::try_from(read_u64(entry + 8)).map_err(|_| OpenError::Invalid("length"))?;
            let end = start
                .checked_add(len)
                .ok_or(OpenError::Invalid("section bounds"))?;
            if end > bytes.len() || start % ALIGN != 0 {
                return Err(OpenError::Invalid("section bounds"));
            }
            Ok(start..end)
        })
        .collect()
}

const fn align(value: usize) -> usize {
    value.div_ceil(ALIGN) * ALIGN
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::testdir::TestDir;

    #[test]
    fn sections_round_trip_aligned() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("container")?;
        let path = dir.path().join("files");
        let image = build(Kind::Files, &[b"abc", b"", &[7u8; 100]]);
        assert_eq!(write(&path, &image)?, image.len() as u64);
        let container = Container::open(&path, Kind::Files).map_err(|e| e.to_string())?;
        assert_eq!(container.section_count(), 3);
        assert_eq!(container.section(0), b"abc");
        assert_eq!(container.section(1), b"");
        assert_eq!(container.section(2), &[7u8; 100]);
        assert_eq!(container.section(3), b"");
        Ok(())
    }

    #[test]
    fn identical_image_is_not_rewritten() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("container-same")?;
        let path = dir.path().join("meta");
        let image = build(Kind::Meta, &[b"v1"]);
        write(&path, &image)?;
        let before = fs::metadata(&path)?.modified()?;
        assert_eq!(write(&path, &image)?, 0);
        assert_eq!(fs::metadata(&path)?.modified()?, before);
        assert!(write(&path, &build(Kind::Meta, &[b"v2"]))? > 0);
        Ok(())
    }

    #[test]
    fn open_reader_keeps_its_snapshot_while_a_writer_replaces_the_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("container-snapshot")?;
        let path = dir.path().join("derived");
        write(&path, &build(Kind::Derived, &[b"old snapshot"]))?;
        let reader = Container::open(&path, Kind::Derived).map_err(|e| e.to_string())?;
        write(&path, &build(Kind::Derived, &[b"new snapshot, longer"]))?;
        assert_eq!(reader.section(0), b"old snapshot");
        let fresh = Container::open(&path, Kind::Derived).map_err(|e| e.to_string())?;
        assert_eq!(fresh.section(0), b"new snapshot, longer");
        Ok(())
    }

    #[test]
    fn corruption_truncation_and_wrong_kind_are_rejected() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = TestDir::new("container-bad")?;
        let path = dir.path().join("derived");
        let image = build(Kind::Derived, &[b"payload"]);
        let invalid = |bytes: &[u8], kind| -> io::Result<bool> {
            fs::write(&path, bytes)?;
            Ok(matches!(
                Container::open(&path, kind),
                Err(OpenError::Invalid(_))
            ))
        };
        let mut flipped = image.clone();
        let last = flipped.len() - 60;
        flipped[last] ^= 1;
        assert!(invalid(&flipped, Kind::Derived)?);
        assert!(invalid(&image[..image.len() - 1], Kind::Derived)?);
        assert!(invalid(&image[..10], Kind::Derived)?);
        assert!(invalid(&image, Kind::Facts)?);
        assert!(matches!(
            Container::open(&dir.path().join("none"), Kind::Meta),
            Err(OpenError::Missing)
        ));
        Ok(())
    }
}
