//! Immutable fact strings that live until the process ends. A [`Name`] is one thin pointer to a
//! length-prefixed copy in a leaked per-thread arena, so facts hold 8 bytes per string and cost no
//! heap allocation per string. A direct-mapped per-thread cache shares repeated identifiers; it
//! may miss, so equal strings can have different pointers and equality always falls back to bytes.

use std::borrow::Borrow;
use std::cell::RefCell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::ptr::NonNull;

const HEADER: usize = size_of::<u32>();
const CHUNK: usize = 64 * 1024;
const CACHE_SLOTS: usize = 1 << 15;
/// Empty string: a zero length header without text.
static EMPTY: [u8; HEADER] = [0; HEADER];

#[derive(Clone, Copy)]
pub struct Name(NonNull<u8>);

// SAFETY: a `Name` points to immutable bytes that are leaked, so they are valid on every thread
// for the rest of the process.
unsafe impl Send for Name {}
// SAFETY: as for `Send`; shared access only reads immutable bytes.
unsafe impl Sync for Name {}

struct Arena {
    cache: Vec<Option<Name>>,
    chunk: &'static mut [u8],
}

thread_local! {
    static ARENA: RefCell<Arena> = const {
        RefCell::new(Arena { cache: Vec::new(), chunk: &mut [] })
    };
}

impl Name {
    /// Longer texts can not occur: source files are limited far below 4 GiB.
    pub fn new(text: &str) -> Self {
        if text.is_empty() {
            return Self::default();
        }
        ARENA.with_borrow_mut(|arena| arena.intern(text))
    }

    pub fn as_str(&self) -> &'static str {
        let pointer = self.0.as_ptr();
        // SAFETY: `Name` is only built from a header written by `Arena::store` or from `EMPTY`:
        // `length` initialized UTF-8 bytes follow the header in leaked memory.
        unsafe {
            let length = pointer.cast::<u32>().read_unaligned() as usize;
            let text = std::slice::from_raw_parts(pointer.add(HEADER), length);
            std::str::from_utf8_unchecked(text)
        }
    }
}

impl Arena {
    fn intern(&mut self, text: &str) -> Name {
        if self.cache.is_empty() {
            self.cache = vec![None; CACHE_SLOTS];
        }
        let slot = cache_slot(text.as_bytes());
        if let Some(name) = self.cache[slot]
            && name.as_str() == text
        {
            return name;
        }
        let name = self.store(text);
        self.cache[slot] = Some(name);
        name
    }

    fn store(&mut self, text: &str) -> Name {
        let needed = HEADER + text.len();
        if needed > self.chunk.len() {
            // A text above a quarter chunk gets its own block and leaves the current chunk open.
            if needed > CHUNK / 4 {
                return write_name(Vec::leak(vec![0; needed]), text);
            }
            self.chunk = Vec::leak(vec![0; CHUNK]);
        }
        let (target, rest) = std::mem::take(&mut self.chunk).split_at_mut(needed);
        self.chunk = rest;
        write_name(target, text)
    }
}

fn write_name(target: &'static mut [u8], text: &str) -> Name {
    let (header, bytes) = target.split_at_mut(HEADER);
    header.copy_from_slice(&(text.len() as u32).to_ne_bytes());
    bytes.copy_from_slice(text.as_bytes());
    Name(NonNull::from(target).cast())
}

/// Length and both 8-byte ends: enough to separate identifiers, and a collision only costs a copy.
fn cache_slot(text: &[u8]) -> usize {
    let (head, tail) = if text.len() >= 8 {
        (load(&text[..8]), load(&text[text.len() - 8..]))
    } else {
        (load(text), 0)
    };
    let mixed =
        (head ^ tail.rotate_left(23) ^ text.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (mixed >> (u64::BITS - CACHE_SLOTS.trailing_zeros())) as usize
}

fn load(bytes: &[u8]) -> u64 {
    let mut word = [0; 8];
    word[..bytes.len()].copy_from_slice(bytes);
    u64::from_ne_bytes(word)
}

impl Default for Name {
    fn default() -> Self {
        Self(NonNull::from(&EMPTY).cast())
    }
}

impl Deref for Name {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for Name {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<&String> for Name {
    fn from(text: &String) -> Self {
        Self::new(text)
    }
}

impl From<String> for Name {
    fn from(text: String) -> Self {
        Self::new(&text)
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0 || self.as_str() == other.as_str()
    }
}

impl Eq for Name {}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for Name {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<Name> for str {
    fn eq(&self, other: &Name) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Name> for &str {
    fn eq(&self, other: &Name) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<Name> for String {
    fn eq(&self, other: &Name) -> bool {
        self == other.as_str()
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Name {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), formatter)
    }
}

impl fmt::Display for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::Name;

    #[test]
    fn names_keep_their_text_across_chunks_and_threads() -> Result<(), String> {
        let texts: Vec<String> = (0..40_000)
            .map(|number| "x".repeat(number % 90) + &number.to_string())
            .collect();
        let names: Vec<Name> = texts.iter().map(Name::from).collect();
        let long = Name::new(&"y".repeat(100_000));
        let moved = std::thread::spawn(move || (names, long));
        let (names, long) = moved.join().map_err(|_| "the thread panicked")?;
        for (name, text) in names.iter().zip(&texts) {
            assert_eq!(name.as_str(), text);
        }
        assert_eq!(long.len(), 100_000);
        assert_eq!(Name::new(""), Name::default());
        assert_eq!(Name::default().as_str(), "");
        Ok(())
    }

    #[test]
    fn equal_text_is_equal_without_a_shared_pointer() -> Result<(), String> {
        let here = Name::new("resolve_file");
        let there = std::thread::spawn(|| Name::new("resolve_file"))
            .join()
            .map_err(|_| "the thread panicked")?;
        assert_ne!(here.0, there.0);
        assert_eq!(here, there);
        assert_eq!(here, Name::new("resolve_file"));
        assert_eq!(here.0, Name::new("resolve_file").0);
        assert!(Name::new("a") < Name::new("b"));
        Ok(())
    }
}
