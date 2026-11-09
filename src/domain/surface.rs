//! Surface type for TASK 217 incremental tiers.
//!
//! The surface behind one query session: fingerprint of its definition,
//! build age, dirty condition, warm read set and elapsed time; seven measured
//! refresh costs stay in the caller and never enter the hash.

use xxhash_rust::xxh3::xxh3_64;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surface {
    /// Fingerprint of the query definition.
    pub fingerprint: u64,
    /// Age of the build the query ran under.
    pub build_age: u32,
    /// Dirty condition flags.
    pub dirty_path: u32,
    /// Warm read set identifiers.
    pub files_changed: u32,
    /// Warm read set second component.
    pub functions_changed: u32,
    /// Warm read set third component.
    pub symbols_added: u32,
    /// Warm read set fourth component.
    pub symbols_removed: u32,
}

impl Surface {
    /// Repeatable surface hash over the definition and warm set only;
    /// elapsed time and refresh costs are excluded by design.
    #[allow(dead_code)]
    pub fn hash(&self) -> u64 {
        let mut h = xxh3_64(&self.fingerprint.to_le_bytes());
        h ^= xxh3_64(&self.build_age.to_le_bytes());
        h = h.wrapping_mul(0x9e3779b97f4a7c15);
        h ^= xxh3_64(&self.dirty_path.to_le_bytes());
        h = h.wrapping_mul(0x9e3779b97f4a7c15);
        h ^= xxh3_64(&self.files_changed.to_le_bytes());
        h = h.wrapping_mul(0x9e3779b97f4a7c15);
        h ^= xxh3_64(&self.functions_changed.to_le_bytes());
        h = h.wrapping_mul(0x9e3779b97f4a7c15);
        h ^= xxh3_64(&self.symbols_added.to_le_bytes());
        h = h.wrapping_mul(0x9e3779b97f4a7c15);
        h ^= xxh3_64(&self.symbols_removed.to_le_bytes());
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_is_stable_and_ignores_time() {
        let s = Surface {
            fingerprint: 42,
            build_age: 1,
            dirty_path: 7,
            files_changed: 3,
            functions_changed: 2,
            symbols_added: 1,
            symbols_removed: 1,
        };
        assert_eq!(s.hash(), s.hash());
        // Different warm set yields different hash
        let t = Surface {
            files_changed: 4,
            ..s.clone()
        };
        assert_ne!(s.hash(), t.hash());
    }
}
