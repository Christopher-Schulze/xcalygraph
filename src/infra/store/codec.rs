//! Compact byte encoding: LEB128 varints, zigzag integers and length-prefixed strings.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub &'static str);

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "corrupt index data: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

pub type DecodeResult<T> = Result<T, DecodeError>;

#[derive(Debug, Default)]
pub struct Encoder {
    pub bytes: Vec<u8>,
}

impl Encoder {
    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn var(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.bytes.push((value as u8) | 0x80);
            value >>= 7;
        }
        self.bytes.push(value as u8);
    }

    pub fn var_usize(&mut self, value: usize) {
        self.var(value as u64);
    }

    pub fn var_u32(&mut self, value: u32) {
        self.var(u64::from(value));
    }

    pub fn signed(&mut self, value: i64) {
        self.var(((value << 1) ^ (value >> 63)) as u64);
    }

    pub fn u128(&mut self, value: u128) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn str(&mut self, value: &str) {
        self.var_usize(value.len());
        self.bytes.extend_from_slice(value.as_bytes());
    }

    pub fn opt_str(&mut self, value: Option<&str>) {
        match value {
            Some(text) => {
                self.u8(1);
                self.str(text);
            }
            None => self.u8(0),
        }
    }

    pub fn strs(&mut self, values: &[String]) {
        self.var_usize(values.len());
        for value in values {
            self.str(value);
        }
    }
}

pub struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.position >= self.bytes.len()
    }

    pub fn u8(&mut self) -> DecodeResult<u8> {
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or(DecodeError("truncated byte"))?;
        self.position += 1;
        Ok(byte)
    }

    pub fn var(&mut self) -> DecodeResult<u64> {
        let mut value = 0u64;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            if shift >= 64 {
                return Err(DecodeError("varint overflow"));
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    }

    pub fn var_usize(&mut self) -> DecodeResult<usize> {
        usize::try_from(self.var()?).map_err(|_| DecodeError("length overflow"))
    }

    pub fn var_u32(&mut self) -> DecodeResult<u32> {
        u32::try_from(self.var()?).map_err(|_| DecodeError("u32 overflow"))
    }

    pub fn signed(&mut self) -> DecodeResult<i64> {
        let raw = self.var()?;
        Ok(((raw >> 1) as i64) ^ -((raw & 1) as i64))
    }

    pub fn u128(&mut self) -> DecodeResult<u128> {
        let end = self.position + 16;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or(DecodeError("truncated u128"))?;
        let mut buffer = [0u8; 16];
        buffer.copy_from_slice(slice);
        self.position = end;
        Ok(u128::from_le_bytes(buffer))
    }

    pub fn str(&mut self) -> DecodeResult<&'a str> {
        let len = self.var_usize()?;
        let end = self
            .position
            .checked_add(len)
            .ok_or(DecodeError("string overflow"))?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or(DecodeError("truncated string"))?;
        self.position = end;
        std::str::from_utf8(slice).map_err(|_| DecodeError("invalid utf-8"))
    }

    pub fn string(&mut self) -> DecodeResult<String> {
        self.str().map(str::to_owned)
    }

    pub fn opt_string(&mut self) -> DecodeResult<Option<String>> {
        match self.u8()? {
            0 => Ok(None),
            1 => self.string().map(Some),
            _ => Err(DecodeError("invalid option tag")),
        }
    }

    pub fn strings(&mut self) -> DecodeResult<Vec<String>> {
        let len = self.var_usize()?;
        if len > self.bytes.len() {
            return Err(DecodeError("list length exceeds data"));
        }
        (0..len).map(|_| self.string()).collect()
    }

    /// Length prefix of a list, bounded by the remaining bytes to reject corrupt counts early.
    pub fn len(&mut self) -> DecodeResult<usize> {
        let len = self.var_usize()?;
        if len > self.bytes.len().saturating_sub(self.position) {
            return Err(DecodeError("list length exceeds data"));
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() -> DecodeResult<()> {
        let mut encoder = Encoder::default();
        encoder.var(0);
        encoder.var(300);
        encoder.var(u64::MAX);
        encoder.signed(-5);
        encoder.signed(i64::MIN);
        encoder.u128(u128::MAX - 7);
        encoder.str("héllo");
        encoder.opt_str(None);
        encoder.opt_str(Some("x"));
        encoder.strs(&["a".to_owned(), String::new()]);
        let mut decoder = Decoder::new(&encoder.bytes);
        assert_eq!(decoder.var()?, 0);
        assert_eq!(decoder.var()?, 300);
        assert_eq!(decoder.var()?, u64::MAX);
        assert_eq!(decoder.signed()?, -5);
        assert_eq!(decoder.signed()?, i64::MIN);
        assert_eq!(decoder.u128()?, u128::MAX - 7);
        assert_eq!(decoder.str()?, "héllo");
        assert_eq!(decoder.opt_string()?, None);
        assert_eq!(decoder.opt_string()?, Some("x".to_owned()));
        assert_eq!(decoder.strings()?, ["a", ""]);
        assert!(decoder.is_done());
        Ok(())
    }

    #[test]
    fn truncated_and_corrupt_input_is_an_error() {
        assert!(Decoder::new(&[0x80]).var().is_err());
        assert!(Decoder::new(&[5, b'a']).str().is_err());
        assert!(Decoder::new(&[0xff; 11]).var().is_err());
        assert!(Decoder::new(&[2, 0xff, 0xfe]).str().is_err());
        assert!(Decoder::new(&[9]).opt_string().is_err());
    }
}
