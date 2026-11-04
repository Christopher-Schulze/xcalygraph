//! Budgeted output in the text and JSON Lines formats (`docs/spec.md` section 8).

use std::borrow::Cow;
use std::fmt::Write as _;

/// Bytes per estimated token (`docs/spec.md` section 2, `--budget`).
pub const BYTES_PER_TOKEN: usize = 3;

/// Bytes kept free for the truncation line (`+999999 more` / `{"t":"more","n":999999}`), so
/// output never exceeds the budget, truncation line included.
const TEXT_FOOTER: usize = 13;
const JSON_FOOTER: usize = 26;

/// Bytes kept free for the truncation line in this format.
pub fn footer_len(json: bool) -> usize {
    if json { JSON_FOOTER } else { TEXT_FOOTER }
}

/// One JSON Lines field value.
pub enum Value<'a> {
    Str(&'a str),
    Int(u64),
    Float(f32),
    Strings(&'a [String]),
}

/// A semantic query fact with text and JSON Lines views derived from the same fields.
pub(super) trait OutputFact {
    fn text(&self) -> String;
    fn json_line(&self) -> String;
}

pub struct Writer {
    json: bool,
    capacity: usize,
    limit: usize,
    out: String,
    /// Items that did not fit the budget.
    cut: usize,
    full: bool,
    /// Path of the file whose line was printed last (text grouping).
    current_file: Option<String>,
    /// Path of the location line printed last: the next one in that file prints `:line` only.
    last_location: Option<String>,
}

impl Writer {
    /// The output stays within `budget_tokens * BYTES_PER_TOKEN` bytes, including truncation.
    pub fn new(json: bool, budget_tokens: usize) -> Self {
        let footer = if json { JSON_FOOTER } else { TEXT_FOOTER };
        let capacity = budget_tokens.saturating_mul(BYTES_PER_TOKEN);
        Self {
            json,
            capacity,
            limit: capacity.saturating_sub(footer),
            out: String::new(),
            cut: 0,
            full: false,
            current_file: None,
            last_location: None,
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
        self.cut = self.cut.saturating_add(items);
    }

    /// Writes `{prefix} {path}:{line}{suffix}`, or `{prefix} :{line}{suffix}` right after a line
    /// of the same file.
    pub fn location_line(
        &mut self,
        prefix: &str,
        path: &str,
        line: u32,
        suffix: &str,
        record: &[(&str, Value)],
    ) -> bool {
        let same_file = self.last_location.as_deref() == Some(path);
        let text = if same_file {
            format!("{prefix} :{line}{suffix}")
        } else {
            format!("{prefix} {path}:{line}{suffix}")
        };
        let written = self.line(&text, record);
        if written {
            self.last_location = Some(path.to_owned());
        }
        written
    }

    /// Writes one item: in text form under its file line (when `file` is given), in JSON form
    /// as one object. Returns false and counts the item as cut when it does not fit.
    pub fn item(&mut self, file: Option<&str>, text: &str, record: &[(&str, Value)]) -> bool {
        if self.full {
            self.cut = self.cut.saturating_add(1);
            return false;
        }
        let mut chunk = String::new();
        if self.json {
            chunk = json_object(record);
        } else {
            if let Some(path) = file
                && self.current_file.as_deref() != Some(path)
            {
                chunk.push_str(escape_text(path).as_ref());
                chunk.push('\n');
            }
            if file.is_some() {
                chunk.push_str("  ");
            }
            chunk.push_str(escape_text(text).as_ref());
            chunk.push('\n');
        }
        if chunk.len() > self.remaining() {
            self.full = true;
            self.cut = self.cut.saturating_add(1);
            return false;
        }
        self.out.push_str(&chunk);
        self.last_location = None;
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

    /// Sets `bytes` aside for content written later (`impact` run commands); returns what was
    /// actually reserved.
    pub fn reserve(&mut self, bytes: usize) -> usize {
        let free = self.limit.saturating_sub(self.out.len());
        let reserved = bytes.min(free);
        self.limit -= reserved;
        reserved
    }

    /// Returns reserved bytes and reopens the writer for items that fit them.
    pub fn release(&mut self, bytes: usize) {
        self.limit += bytes;
        self.full = false;
    }

    /// Bytes still available before the truncation line.
    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.out.len())
    }

    /// A pre-rendered JSON Lines record.
    pub fn raw(&mut self, record: &str) -> bool {
        if self.full || record.len() > self.remaining() {
            self.full = true;
            self.cut = self.cut.saturating_add(1);
            return false;
        }
        self.out.push_str(record);
        true
    }

    /// A line that is not grouped under a file (section keywords, notices).
    pub fn line(&mut self, text: &str, record: &[(&str, Value)]) -> bool {
        let written = self.item(None, text, record);
        if written {
            self.current_file = None;
        }
        written
    }

    pub(super) fn fact(&mut self, fact: &impl OutputFact) -> bool {
        if self.json {
            self.raw(&fact.json_line())
        } else {
            self.line(&fact.text(), &[])
        }
    }

    pub(super) fn rendered_fact(&mut self, text: &str, json: &str) -> bool {
        if self.json {
            self.raw(json)
        } else {
            self.line(text, &[])
        }
    }

    pub fn finish(mut self) -> String {
        if self.cut > 0 {
            let available = self.capacity.saturating_sub(self.out.len());
            let full = if self.json {
                json_object(&[
                    ("t", Value::Str("more")),
                    ("n", Value::Int(self.cut as u64)),
                ])
            } else {
                format!("+{} more\n", self.cut)
            };
            let compact = if self.json {
                "{\"t\":\"more\"}\n".to_owned()
            } else {
                format!("+{}\n", self.cut)
            };
            if full.len() <= available {
                self.out.push_str(&full);
            } else if compact.len() <= available {
                self.out.push_str(&compact);
            } else if self.json && available >= 3 {
                self.out.push_str("{}\n");
            } else if !self.json && available >= 2 {
                self.out.push_str("+\n");
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
            Value::Strings(values) => {
                let mut rendered = String::from("[");
                for (position, value) in values.iter().enumerate() {
                    if position > 0 {
                        rendered.push(',');
                    }
                    rendered.push_str(&json_string(value));
                }
                rendered.push(']');
                rendered
            }
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

/// Text form for output fields: escape backslashes and C0/C1 controls, leaving ordinary text
/// unchanged so the records stay single-line and the representation remains reversible.
fn escape_text(text: &str) -> Cow<'_, str> {
    let mut escaped = None::<String>;
    let mut start = 0;
    for (position, character) in text.char_indices() {
        let replacement = match character {
            '\\' => "\\\\",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            control if control.is_control() => {
                let output = escaped.get_or_insert_with(|| String::with_capacity(text.len()));
                output.push_str(&text[start..position]);
                let _ = write!(output, "\\u{:04x}", control as u32);
                start = position + character.len_utf8();
                continue;
            }
            _ => continue,
        };
        let output = escaped.get_or_insert_with(|| String::with_capacity(text.len()));
        output.push_str(&text[start..position]);
        output.push_str(replacement);
        start = position + character.len_utf8();
    }
    match escaped {
        Some(mut output) => {
            output.push_str(&text[start..]);
            Cow::Owned(output)
        }
        None => Cow::Borrowed(text),
    }
}

/// UTF-8 byte length after `escape_text`, for query renderers that preflight text budgets.
pub(super) fn escaped_text_len(text: &str) -> usize {
    text.chars().fold(text.len(), |length, character| {
        let extra = match character {
            '\\' | '\n' | '\r' | '\t' => 1,
            control if control.is_control() => 6 - control.len_utf8(),
            _ => 0,
        };
        length.saturating_add(extra)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_groups_items_under_files_and_reports_the_cut() {
        let mut writer = Writer::new(false, 25);
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
    fn tiny_budgets_use_only_complete_truthful_truncation_indicators() {
        for (json, budget, expected) in [
            (false, 1, "+\n"),
            (false, 2, "+20\n"),
            (false, 3, "+20 more\n"),
            (true, 1, "{}\n"),
            (true, 5, "{\"t\":\"more\"}\n"),
            (true, 7, "{\"t\":\"more\",\"n\":20}\n"),
        ] {
            let mut writer = Writer::new(json, budget);
            writer.skip(20);
            assert_eq!(writer.finish(), expected, "budget {budget}, json {json}");
        }
        let mut writer = Writer::new(true, 12);
        writer.line("", &[("t", Value::Str("a"))]);
        writer.skip(usize::MAX);
        writer.skip(1);
        let output = writer.finish();
        assert_eq!(output, "{\"t\":\"a\"}\n{\"t\":\"more\"}\n");
        assert!(output.len() <= 36);
    }

    #[test]
    fn no_budget_is_exceeded_and_a_tiny_one_yields_the_truncation_line() {
        for json in [false, true] {
            for budget in 0..60 {
                let mut writer = Writer::new(json, budget);
                for index in 0..20 {
                    let name = format!("ünïcödé_{index}_{}", "x".repeat(index));
                    writer.item(
                        Some("dir/a_very_long_path/file.rs"),
                        &format!("{index} fn {name}"),
                        &[("t", Value::Str("sym")), ("name", Value::Str(&name))],
                    );
                }
                let out = writer.finish();
                assert!(
                    out.len() <= budget * BYTES_PER_TOKEN,
                    "{budget} {json}\n{out}"
                );
                if out.is_empty() {
                    assert_eq!(budget, 0);
                    continue;
                }
                if json {
                    assert!(
                        out.lines()
                            .all(|line| line.starts_with('{') && line.ends_with('}'))
                    );
                }
                let shown = out
                    .lines()
                    .filter(|line| line.contains("fn ") || line.contains("sym"))
                    .count();
                if out == "+\n" || out == "{}\n" || out == "{\"t\":\"more\"}\n" {
                    assert_eq!(shown, 0);
                    continue;
                }
                let cut: usize = out
                    .lines()
                    .last()
                    .and_then(|line| {
                        line.trim_start_matches('+')
                            .split([' ', ':', '}'])
                            .filter_map(|part| part.parse().ok())
                            .next_back()
                    })
                    .unwrap_or(0);
                assert_eq!(shown + cut, 20, "{budget} {json}\n{out}");
            }
        }
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
        let values = vec!["run \"all\"".to_owned(), "line\nnext".to_owned()];
        assert_eq!(
            json_object(&[("runs", Value::Strings(&values))]),
            "{\"runs\":[\"run \\\"all\\\"\",\"line\\nnext\"]}\n"
        );
        let mut writer = Writer::new(true, 13);
        writer.item(Some("a"), "", &[("t", Value::Str("sym"))]);
        writer.item(Some("a"), "", &[("t", Value::Str("sym"))]);
        assert_eq!(
            writer.finish(),
            "{\"t\":\"sym\"}\n{\"t\":\"more\",\"n\":1}\n"
        );
    }

    #[test]
    fn text_escapes_backslashes_and_control_characters_reversibly() {
        let mut writer = Writer::new(false, 100);
        assert!(writer.item(Some("a\\b\tc\r\n\u{0001}\u{0085}"), "line\\next\n", &[],));
        assert_eq!(
            writer.finish(),
            "a\\\\b\\tc\\r\\n\\u0001\\u0085\n  line\\\\next\\n\n"
        );
    }

    #[test]
    fn text_budget_counts_escaped_path_bytes() {
        let mut writer = Writer::new(false, 10);
        assert!(!writer.item(Some("a\t"), "12345678901", &[]));
        assert_eq!(writer.finish(), "+1 more\n");
    }

    #[test]
    fn escaped_text_length_matches_rendered_utf8_bytes() {
        let text = "ordinary/路径\\name\t\u{0001}\u{0085}";
        assert_eq!(escaped_text_len(text), escape_text(text).len());
        assert_eq!(escape_text("ordinary/路径").as_ref(), "ordinary/路径");
    }
}
