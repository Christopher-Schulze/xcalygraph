//! Budgeted output in the text and JSON Lines formats (`docs/spec.md` section 8).

use std::fmt::Write as _;

/// Bytes per estimated token (`docs/spec.md` section 2, `--budget`).
pub const BYTES_PER_TOKEN: usize = 3;

/// One JSON Lines field value.
pub enum Value<'a> {
    Str(&'a str),
    Int(u64),
    Float(f32),
}

pub struct Writer {
    json: bool,
    limit: usize,
    out: String,
    /// Items that did not fit the budget.
    cut: usize,
    full: bool,
    /// Path of the file whose line was printed last (text grouping).
    current_file: Option<String>,
}

impl Writer {
    pub fn new(json: bool, budget_tokens: usize) -> Self {
        Self {
            json,
            limit: budget_tokens.saturating_mul(BYTES_PER_TOKEN),
            out: String::new(),
            cut: 0,
            full: false,
            current_file: None,
        }
    }

    pub fn is_json(&self) -> bool {
        self.json
    }

    pub fn is_full(&self) -> bool {
        self.full
    }

    /// Counts an item that was not written because the budget is exhausted.
    pub fn skip(&mut self, items: usize) {
        self.cut += items;
    }

    /// Writes one item: in text form under its file line (when `file` is given), in JSON form
    /// as one object. Returns false and counts the item as cut when it does not fit.
    pub fn item(&mut self, file: Option<&str>, text: &str, record: &[(&str, Value)]) -> bool {
        if self.full {
            self.cut += 1;
            return false;
        }
        let mut chunk = String::new();
        if self.json {
            chunk = json_object(record);
        } else {
            if let Some(path) = file
                && self.current_file.as_deref() != Some(path)
            {
                chunk.push_str(path);
                chunk.push('\n');
            }
            if file.is_some() {
                chunk.push_str("  ");
            }
            chunk.push_str(text);
            chunk.push('\n');
        }
        if self.out.len() + chunk.len() > self.limit && !self.out.is_empty() {
            self.full = true;
            self.cut += 1;
            return false;
        }
        self.out.push_str(&chunk);
        if !self.json {
            self.current_file = file.map(str::to_owned);
        }
        true
    }

    /// A file line of its own; following items of the same file group under it.
    pub fn file(&mut self, path: &str, record: &[(&str, Value)]) -> bool {
        if !self.json && self.current_file.as_deref() == Some(path) {
            return true;
        }
        let written = self.item(None, path, record);
        if written && !self.json {
            self.current_file = Some(path.to_owned());
        }
        written
    }

    /// A line written regardless of the budget (run commands of `impact`).
    pub fn force_line(&mut self, text: &str, record: &[(&str, Value)]) {
        if self.json {
            self.out.push_str(&json_object(record));
        } else {
            self.out.push_str(text);
            self.out.push('\n');
        }
        self.current_file = None;
    }

    /// A pre-rendered JSON Lines record.
    pub fn raw(&mut self, record: &str) {
        self.out.push_str(record);
    }

    /// A line that is not grouped under a file (section keywords, notices).
    pub fn line(&mut self, text: &str, record: &[(&str, Value)]) -> bool {
        let written = self.item(None, text, record);
        if written {
            self.current_file = None;
        }
        written
    }

    pub fn finish(mut self) -> String {
        if self.cut > 0 {
            if self.json {
                self.out.push_str(&json_object(&[
                    ("t", Value::Str("more")),
                    ("n", Value::Int(self.cut as u64)),
                ]));
            } else {
                let _ = writeln!(self.out, "+{} more", self.cut);
            }
        }
        self.out
    }
}

pub fn json_object(fields: &[(&str, Value)]) -> String {
    let mut out = String::from("{");
    let mut first = true;
    for (key, value) in fields {
        let rendered = match value {
            Value::Str("") => continue,
            Value::Str(text) => json_string(text),
            Value::Int(number) => number.to_string(),
            Value::Float(number) => format!("{:.3}", number),
        };
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&json_string(key));
        out.push(':');
        out.push_str(&rendered);
    }
    out.push_str("}\n");
    out
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_groups_items_under_files_and_reports_the_cut() {
        let mut writer = Writer::new(false, 20);
        assert!(writer.item(Some("a.go"), "3 fn A", &[]));
        assert!(writer.item(Some("a.go"), "9 fn B", &[]));
        assert!(writer.item(Some("b.go"), "1 fn C", &[]));
        assert!(!writer.item(
            Some("b.go"),
            "2 fn a_rather_long_name_that_does_not_fit",
            &[]
        ));
        assert!(!writer.item(Some("b.go"), "3 fn D", &[]));
        assert_eq!(
            writer.finish(),
            "a.go\n  3 fn A\n  9 fn B\nb.go\n  1 fn C\n+2 more\n"
        );
    }

    #[test]
    fn json_omits_empty_fields_and_escapes() {
        let line = json_object(&[
            ("t", Value::Str("sym")),
            ("doc", Value::Str("")),
            ("sig", Value::Str("f(\"x\")\n")),
            ("line", Value::Int(3)),
            ("score", Value::Float(1.5)),
        ]);
        assert_eq!(
            line,
            "{\"t\":\"sym\",\"sig\":\"f(\\\"x\\\")\\n\",\"line\":3,\"score\":1.500}\n"
        );
        let mut writer = Writer::new(true, 1);
        writer.item(Some("a"), "", &[("t", Value::Str("sym"))]);
        writer.item(Some("a"), "", &[("t", Value::Str("sym"))]);
        assert_eq!(
            writer.finish(),
            "{\"t\":\"sym\"}\n{\"t\":\"more\",\"n\":1}\n"
        );
    }
}
