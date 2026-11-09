//! Word-level structural index for TASK 225 (slice 1).

pub fn is_identifier_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}
pub fn is_whitespace_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifier_byte() {
        assert!(is_identifier_byte(b'a'));
        assert!(!is_identifier_byte(b' '));
    }
}
