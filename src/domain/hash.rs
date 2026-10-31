//! Fast non-cryptographic hashing for in-memory maps of the hot path (FxHash scheme: rotate,
//! xor, multiply per word). Keys are trusted index data, so flooding resistance is not needed.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Debug, Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word) ^ (rest.len() as u64) << 56);
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub type FastSet<K> = HashSet<K, BuildHasherDefault<FxHasher>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    fn hash_of(value: impl Hash) -> u64 {
        let mut hasher = FxHasher::default();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn distinct_keys_hash_differently_and_maps_work() {
        assert_ne!(hash_of("store"), hash_of("stores"));
        assert_ne!(hash_of("abcdefgh"), hash_of("abcdefgh\\0"));
        assert_ne!(hash_of((1u32, "a")), hash_of((2u32, "a")));
        let mut map: FastMap<String, u32> = FastMap::default();
        map.insert("x".to_owned(), 1);
        assert_eq!(map.get("x"), Some(&1));
    }
}
