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
    let mut parser = Parser {
        bytes: text.as_bytes(),
        position: 0,
        line: 1,
        depth: 0,
    };
    let value = parser.value()?;
    parser.skip();
    Some(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
    line: u32,
    depth: usize,
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
                (Some(b'/'), Some(b'/')) => {
                    while self.peek().is_some_and(|byte| byte != b'\n') {
                        self.bump();
                    }
                }
                (Some(b'/'), Some(b'*')) => {
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

    fn string(&mut self) -> Option<String> {
        self.bump();
        let mut out = Vec::new();
        loop {
            match self.bump()? {
                b'"' => break,
                b'\\' => match self.bump()? {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'u' => {
                        let hex = self.bytes.get(self.position..self.position + 4)?;
                        self.position += 4;
                        let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                        let ch = char::from_u32(code).unwrap_or('\u{fffd}');
                        let mut buffer = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
                    }
                    other => out.push(other),
                },
                byte => out.push(byte),
            }
        }
        String::from_utf8(out).ok()
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    fn array(&mut self) -> Option<Value> {
        self.enter()?;
        self.bump();
        let mut items = Vec::new();
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
        self.depth -= 1;
        Some(Value::Array(items))
    }

    fn object(&mut self) -> Option<Value> {
        self.enter()?;
        self.bump();
        let mut entries = Vec::new();
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
        self.depth -= 1;
        Some(Value::Object(entries))
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
}
