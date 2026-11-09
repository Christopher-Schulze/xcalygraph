//! Word-level structural index for TASK 225 (slice 1).
//! Portable scalar masks; NEON/AVX2 fast path plugs in here.

#[allow(dead_code)]
pub fn is_identifier_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}
#[allow(dead_code)]
pub fn is_whitespace_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// One 64-byte block of structural masks (portable scalar version).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Masks {
    pub identifier: u64,
    pub whitespace: u64,
    pub quote: u64,
    pub upper: u64,
    pub high_bit: u64,
}

/// SIMD variant dispatch (differential: NEON/AVX2 must match portable).
#[allow(dead_code)]
pub fn masks_for_block_simd(bytes: &[u8]) -> Masks {
    // Portable path today; NEON/AVX2 kernels plug in here.
    masks_for_block(bytes)
}
/// Explicit portable implementation (reference for differential test).
#[allow(dead_code)]
pub fn masks_for_block_portable(bytes: &[u8]) -> Masks {
    masks_for_block(bytes)
}

/// Builds masks for the first `min(len,64)` bytes (portable scalar).
#[allow(dead_code)]
pub fn masks_for_block(bytes: &[u8]) -> Masks {
    let mut m = Masks::default();
    for (i, &b) in bytes.iter().take(64).enumerate() {
        let bit = 1u64 << i;
        if is_identifier_byte(b) {
            m.identifier |= bit;
        }
        if is_whitespace_byte(b) {
            m.whitespace |= bit;
        }
        if b == b'"' || b == b'\'' {
            m.quote |= bit;
        }
        if b.is_ascii_uppercase() {
            m.upper |= bit;
        }
        if b >= 0x80 {
            m.high_bit |= bit;
        }
    }
    m
}

/// Finds word runs from identifier mask bits (portable).
/// Yields `(start, len)` in the 64-byte block.
#[allow(dead_code)]
pub fn word_runs(mask: u64) -> impl Iterator<Item = (u8, u8)> {
    let mut rem = mask;
    let mut pos = 0u8;
    std::iter::from_fn(move || {
        if rem == 0 {
            return None;
        }
        let trailing = rem.trailing_zeros() as u8;
        let run = (rem >> trailing).trailing_ones() as u8;
        let start = pos + trailing;
        pos = start + run;
        rem >>= trailing + run;
        Some((start, run))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifier_byte() {
        assert!(is_identifier_byte(b'a'));
        assert!(!is_identifier_byte(b' '));
    }
    #[test]
    fn masks_and_runs_match_scalar() {
        let bytes = b"ab C\"d\x80";
        let m = masks_for_block(bytes);
        assert_eq!(m.identifier, 0b00101011); // a,b at 0,1; C at 3; d at 5
        let runs: Vec<(u8, u8)> = word_runs(m.identifier).collect();
        assert_eq!(runs, [(0, 2), (3, 1), (5, 1)]);
        assert!(m.high_bit >> 6 & 1 == 1);
    }
    #[test]
    fn masks_match_direct_scan_on_randomish() {
        let data: Vec<u8> = (0..128).map(|i| (i * 37 % 256) as u8).collect();
        let m1 = masks_for_block(&data);
        // Direct recompute
        let mut expected_id = 0u64;
        let mut expected_ws = 0u64;
        for (i, &b) in data.iter().take(64).enumerate() {
            if is_identifier_byte(b) {
                expected_id |= 1 << i;
            }
            if is_whitespace_byte(b) {
                expected_ws |= 1 << i;
            }
        }
        assert_eq!(m1.identifier, expected_id);
        assert_eq!(m1.whitespace, expected_ws);
    }
    #[test]
    fn simd_and_portable_agree_on_all_bytes() {
        for len in [0usize, 1, 63, 64, 65, 128] {
            let data: Vec<u8> = (0..len).map(|i| ((i * 37 + 11) % 256) as u8).collect();
            assert_eq!(masks_for_block(&data), masks_for_block_simd(&data));
            assert_eq!(masks_for_block(&data), masks_for_block_portable(&data));
        }
        // All byte values differential
        let all: Vec<u8> = (0u32..256).map(|v| v as u8).collect();
        for chunk in all.chunks(64) {
            assert_eq!(masks_for_block(chunk), masks_for_block_simd(chunk));
        }
    }
}
