//! Tolerant JSON reader for manifests and config files: accepts JSONC comments and trailing
//! commas, records the line of every object key. Values are only kept where callers need them.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<Value>),
    /// Keys in source order with their line.
    Object(Vec<(String, u32, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Object(entries) => entries
                .iter()
                .find(|(name, _, _)| name == key)
                .map(|(_, _, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(text) => Some(text),
            _ => None,
        }
    }

    /// Strings of an array, or the single string.
    pub fn strings(&self) -> Vec<String> {
        match self {
            Self::Str(text) => vec![text.clone()],
            Self::Array(items) => items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn entries(&self) -> &[(String, u32, Value)] {
        match self {
            Self::Object(entries) => entries,
            _ => &[],
        }
    }
}

pub fn parse(text: &str) -> Option<Value> {
    parse_with_mode(text, true)
}

/// Parses one complete RFC 8259 JSON value without comments or trailing commas.
pub fn parse_strict(text: &str) -> Option<Value> {
    parse_with_mode(text, false)
}

fn parse_with_mode(text: &str, allow_jsonc: bool) -> Option<Value> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        position: 0,
        line: 1,
        depth: 0,
        allow_jsonc,
    };
    let value = parser.value()?;
    parser.skip();
    if !allow_jsonc && parser.position != parser.bytes.len() {
        return None;
    }
    Some(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
    line: u32,
    depth: usize,
    allow_jsonc: bool,
}

const MAX_DEPTH: usize = 256;

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        if byte == b'\n' {
            self.line += 1;
        }
        Some(byte)
    }

    /// Skips whitespace and `//` and `/* */` comments.
    fn skip(&mut self) {
        loop {
            match (self.peek(), self.bytes.get(self.position + 1).copied()) {
                (Some(b' ' | b'\t' | b'\r' | b'\n'), _) => {
                    self.bump();
                }
                (Some(b'/'), Some(b'/')) if self.allow_jsonc => {
                    while self.peek().is_some_and(|byte| byte != b'\n') {
                        self.bump();
                    }
                }
                (Some(b'/'), Some(b'*')) if self.allow_jsonc => {
                    self.bump();
                    self.bump();
                    while self.peek().is_some()
                        && !(self.peek() == Some(b'*')
                            && self.bytes.get(self.position + 1) == Some(&b'/'))
                    {
                        self.bump();
                    }
                    self.bump();
                    self.bump();
                }
                _ => return,
            }
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.skip();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Value::Str),
            b't' => self.word("true").map(|()| Value::Bool(true)),
            b'f' => self.word("false").map(|()| Value::Bool(false)),
            b'n' => self.word("null").map(|()| Value::Null),
            _ => self.number(),
        }
    }

    fn word(&mut self, word: &str) -> Option<()> {
        if self.bytes[self.position..].starts_with(word.as_bytes()) {
            self.position += word.len();
            Some(())
        } else {
            None
        }
    }

    fn number(&mut self) -> Option<Value> {
        if !self.allow_jsonc {
            return self.strict_number();
        }
        let start = self.position;
        while self.peek().is_some_and(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E')
        }) {
            self.position += 1;
        }
        (self.position > start).then(|| {
            Value::Number(String::from_utf8_lossy(&self.bytes[start..self.position]).into_owned())
        })
    }

    fn strict_number(&mut self) -> Option<Value> {
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        match self.peek()? {
            b'0' => {
                self.position += 1;
                if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    return None;
                }
            }
            b'1'..=b'9' => {
                self.position += 1;
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.position += 1;
                }
            }
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.position += 1;
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return None;
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.position += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return None;
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.position += 1;
            }
        }
        let number = std::str::from_utf8(&self.bytes[start..self.position]).ok()?;
        Some(Value::Number(number.to_owned()))
    }

    fn string(&mut self) -> Option<String> {
        self.bump();
        let mut out = Vec::new();
        loop {
            match self.bump()? {
                b'"' => break,
                b'\\' => match self.bump()? {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'/' => out.push(b'/'),
                    b'b' if !self.allow_jsonc => out.push(0x08),
                    b'f' if !self.allow_jsonc => out.push(0x0c),
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'u' if !self.allow_jsonc => self.push_json_unicode(&mut out)?,
                    b'u' => {
                        let hex = self.bytes.get(self.position..self.position + 4)?;
                        self.position += 4;
                        let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                        let ch = char::from_u32(code).unwrap_or('\u{fffd}');
                        let mut buffer = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
                    }
                    other if self.allow_jsonc => out.push(other),
                    _ => return None,
                },
                byte if !self.allow_jsonc && byte < 0x20 => return None,
                byte => out.push(byte),
            }
        }
        String::from_utf8(out).ok()
    }

    fn push_json_unicode(&mut self, output: &mut Vec<u8>) -> Option<()> {
        let first = self.hex_quad()?;
        let codepoint = match first {
            0xd800..=0xdbff => {
                if self.bytes.get(self.position..self.position + 2)? != b"\\u" {
                    return None;
                }
                self.position += 2;
                let second = self.hex_quad()?;
                if !(0xdc00..=0xdfff).contains(&second) {
                    return None;
                }
                0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
            }
            0xdc00..=0xdfff => return None,
            _ => u32::from(first),
        };
        let character = char::from_u32(codepoint)?;
        let mut buffer = [0u8; 4];
        output.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
        Some(())
    }

    fn hex_quad(&mut self) -> Option<u16> {
        let hex = self.bytes.get(self.position..self.position + 4)?;
        if !hex.iter().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        self.position += 4;
        u16::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    fn array(&mut self) -> Option<Value> {
        self.enter()?;
        self.bump();
        let mut items = Vec::new();
        if self.allow_jsonc {
            loop {
                self.skip();
                match self.peek()? {
                    b']' => {
                        self.bump();
                        break;
                    }
                    b',' => {
                        self.bump();
                    }
                    _ => items.push(self.value()?),
                }
            }
        } else {
            self.strict_array(&mut items)?;
        }
        self.depth -= 1;
        Some(Value::Array(items))
    }

    fn strict_array(&mut self, items: &mut Vec<Value>) -> Option<()> {
        self.skip();
        if self.peek() == Some(b']') {
            self.bump();
            return Some(());
        }
        loop {
            items.push(self.value()?);
            self.skip();
            match self.bump()? {
                b']' => return Some(()),
                b',' => {}
                _ => return None,
            }
        }
    }

    fn object(&mut self) -> Option<Value> {
        self.enter()?;
        self.bump();
        let mut entries = Vec::new();
        if self.allow_jsonc {
            loop {
                self.skip();
                match self.peek()? {
                    b'}' => {
                        self.bump();
                        break;
                    }
                    b',' => {
                        self.bump();
                    }
                    b'"' => {
                        let line = self.line;
                        let key = self.string()?;
                        self.skip();
                        if self.bump()? != b':' {
                            return None;
                        }
                        let value = self.value()?;
                        entries.push((key, line, value));
                    }
                    _ => return None,
                }
            }
        } else {
            self.strict_object(&mut entries)?;
        }
        self.depth -= 1;
        Some(Value::Object(entries))
    }

    fn strict_object(&mut self, entries: &mut Vec<(String, u32, Value)>) -> Option<()> {
        self.skip();
        if self.peek() == Some(b'}') {
            self.bump();
            return Some(());
        }
        loop {
            self.skip();
            if self.peek() != Some(b'"') {
                return None;
            }
            let line = self.line;
            let key = self.string()?;
            self.skip();
            if self.bump()? != b':' {
                return None;
            }
            let value = self.value()?;
            entries.push((key, line, value));
            self.skip();
            match self.bump()? {
                b'}' => return Some(()),
                b',' => {}
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_with_comments_trailing_commas_and_lines() -> Result<(), &'static str> {
        let text = "{\n  // comment\n  \"name\": \"app\",\n  \"paths\": { \"@/*\": [\"src/*\",], },\n  /* block */ \"n\": -1.5e3,\n  \"ok\": true, \"none\": null, \"esc\": \"a\\\"b\\u00e9\"\n}";
        let value = parse(text).ok_or("valid JSONC failed to parse")?;
        assert_eq!(value.get("name").and_then(Value::as_str), Some("app"));
        assert_eq!(value.entries()[0].1, 3);
        assert_eq!(
            value
                .get("paths")
                .and_then(|paths| paths.get("@/*"))
                .map(Value::strings),
            Some(vec!["src/*".to_owned()])
        );
        assert_eq!(value.get("n"), Some(&Value::Number("-1.5e3".to_owned())));
        assert_eq!(value.get("esc").and_then(Value::as_str), Some("a\"bé"));
        assert!(parse("{\"a\": }").is_none());
        assert!(parse(&"[".repeat(1000)).is_none());
        Ok(())
    }

    #[test]
    fn strict_json_decodes_escaped_keys_values_and_surrogate_pairs() -> Result<(), &'static str> {
        let text = r#"{"tag\u005fname":"v\u0039\u0039.0.0","pair":"\ud83d\ude80","url":"file:///tmp/release\nassets/SHA256SUMS","controls":"\b\f\/"}"#;
        let value = parse_strict(text).ok_or("valid strict JSON failed to parse")?;
        assert!(parse_strict(r#"{"a":1, "b":2}"#).is_some());
        assert_eq!(
            value.get("tag_name").and_then(Value::as_str),
            Some("v99.0.0")
        );
        assert_eq!(value.get("pair").and_then(Value::as_str), Some("🚀"));
        assert_eq!(
            value.get("url").and_then(Value::as_str),
            Some("file:///tmp/release\nassets/SHA256SUMS")
        );
        assert_eq!(
            value.get("controls").and_then(Value::as_str),
            Some("\u{8}\u{c}/")
        );
        Ok(())
    }

    #[test]
    fn strict_json_rejects_incomplete_or_non_json_syntax() {
        for text in [
            r#"{"tag_name":"v1","assets":[]"#,
            r#"{"a":1} trailing"#,
            r#"{/* comment */"a":1}"#,
            r#"{"a":1,}"#,
            r#"[1,]"#,
            r#"{"a":01}"#,
            r#"{"a":+1}"#,
            r#"{"a":1.}"#,
            r#"{"a":1e}"#,
            r#"{"a":"\x"}"#,
            r#""\ud800""#,
            r#""\udc00""#,
            r#""\ud800\u0041""#,
        ] {
            assert!(parse_strict(text).is_none(), "accepted {text:?}");
        }
        assert!(parse_strict("\"line\nbreak\"").is_none());
    }
}
