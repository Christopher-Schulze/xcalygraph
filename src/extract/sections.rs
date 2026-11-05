//! Full text search of documents: the words of each section become the search terms of its
//! heading, so a hit names the section and its line. A long section or text without headings is
//! split into further sections named by their first line or first word within a long line.

use std::collections::HashSet;

use xxhash_rust::xxh3::xxh3_64;

use crate::domain::facts::{self, Facts, SymKind, Symbol};
use crate::domain::search;

/// Distinct search terms per section; a section beyond it continues in a new one.
const SECTION_TERMS: usize = 256;
/// Characters of a section's first line used as its name.
const NAME_CHARS: usize = 60;

/// Indexes a text file line by line.
pub fn index_lines(facts: &mut Facts, text: &str) {
    index_text(
        facts,
        text.lines()
            .enumerate()
            .map(|(index, line)| (index as u32 + 1, line)),
    );
}

/// Indexes `(line, text)` pairs in line order under the headings already in `facts`.
pub fn index_text<'a>(facts: &mut Facts, lines: impl Iterator<Item = (u32, &'a str)>) {
    let mut headings: Vec<(u32, u32)> = facts
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.kind == Some(SymKind::Heading))
        .map(|(index, symbol)| (symbol.line, index as u32))
        .collect();
    headings.sort_unstable();
    let last_line = facts.lines;
    let mut next_heading = 0;
    let mut owner: Option<u32> = None;
    let mut open = Section::default();
    let mut buffer = String::new();
    for (line, text) in lines {
        let text = text.trim();
        let mut moved = false;
        while headings
            .get(next_heading)
            .is_some_and(|&(start, _)| start <= line)
        {
            owner = Some(headings[next_heading].1);
            next_heading += 1;
            moved = true;
        }
        if moved {
            open.flush(facts, line.saturating_sub(1));
            open.heading = owner;
        }
        // Comment rulers (`##`, `---`) neither name a section nor hold a word.
        if !text.chars().any(char::is_alphanumeric) {
            continue;
        }
        if open.first_line == 0 {
            open.start(line, text, owner);
        }
        search::for_each_token(text, &mut buffer, |token| {
            if token.len() >= 3 && open.seen.insert(xxh3_64(token.as_bytes())) {
                if open.seen.len() > SECTION_TERMS {
                    open.last_line = line;
                    open.flush(facts, line);
                    open.start(line, token, owner);
                    open.seen.insert(xxh3_64(token.as_bytes()));
                }
                if !open.terms.is_empty() {
                    open.terms.push(' ');
                }
                open.terms.push_str(token);
            }
        });
        open.last_line = line;
        if open.seen.len() >= SECTION_TERMS {
            open.flush(facts, line);
        }
    }
    open.flush(facts, last_line);
}

#[derive(Default)]
struct Section {
    /// The heading whose terms are still free; `None` once used or without a heading.
    heading: Option<u32>,
    parent: Option<u32>,
    first_line: u32,
    last_line: u32,
    name: String,
    terms: String,
    /// Hashes of the section's terms: most words repeat, and a copy per word costs millions.
    seen: HashSet<u64>,
}

impl Section {
    fn start(&mut self, line: u32, text: &str, owner: Option<u32>) {
        self.first_line = line;
        self.last_line = line;
        self.parent = owner;
        self.name = facts::clip(&facts::normalize_space(text), NAME_CHARS);
    }

    /// Hands the collected terms to the heading, or to a new section when it has its terms.
    fn flush(&mut self, facts: &mut Facts, end_line: u32) {
        if !self.terms.is_empty() {
            let terms = std::mem::take(&mut self.terms);
            match self.heading.take() {
                Some(heading) => facts.symbols[heading as usize].terms = terms,
                None => facts.symbols.push(Symbol {
                    kind: Some(SymKind::Heading),
                    name: std::mem::take(&mut self.name),
                    parent: self.parent,
                    line: self.first_line,
                    end_line: end_line.max(self.last_line),
                    terms,
                    exported: true,
                    ..Symbol::default()
                }),
            }
        }
        self.first_line = 0;
        self.seen.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rulers_do_not_name_sections() {
        let mut facts = Facts {
            lines: 3,
            ..Facts::default()
        };
        index_lines(&mut facts, "##\n# User Database\n##\n");
        assert_eq!(facts.symbols[0].name, "# User Database");
        assert_eq!(facts.symbols[0].line, 2);
    }

    #[test]
    fn long_lines_retain_every_term_in_bounded_sections() {
        let words: Vec<String> = (0..600u32)
            .map(|index| {
                format!(
                    "word{}{}{}",
                    char::from(b'a' + (index % 26) as u8),
                    char::from(b'a' + ((index / 26) % 26) as u8),
                    char::from(b'a' + ((index / 676) % 26) as u8),
                )
            })
            .collect();
        for heading in [false, true] {
            let mut facts = Facts {
                lines: if heading { 2 } else { 1 },
                ..Facts::default()
            };
            if heading {
                facts.symbols.push(Symbol {
                    kind: Some(SymKind::Heading),
                    name: "Long line".to_owned(),
                    line: 1,
                    end_line: 2,
                    ..Symbol::default()
                });
            }
            let prefix = if heading { "# Long line\n" } else { "" };
            let text = format!("{prefix}{} quokkatailneedle\n", words.join(" "));
            index_lines(&mut facts, &text);
            let terms: HashSet<&str> = facts
                .symbols
                .iter()
                .flat_map(|symbol| symbol.terms.split_whitespace())
                .collect();
            let required: HashSet<String> = search::tokenize(&text)
                .into_iter()
                .filter(|token| token.len() >= 3)
                .collect();
            assert!(
                required.iter().all(|word| terms.contains(word.as_str())),
                "heading={heading}: missing {} of {} normalized terms",
                required
                    .iter()
                    .filter(|word| !terms.contains(word.as_str()))
                    .count(),
                required.len(),
            );
            assert_eq!(facts.symbols.len(), 3);
            for symbol in &facts.symbols {
                assert!(symbol.terms.split_whitespace().count() <= SECTION_TERMS);
            }
            for symbol in &facts.symbols[1..] {
                assert_eq!(symbol.line, facts.lines);
                assert_eq!(symbol.end_line, facts.lines);
                assert_eq!(symbol.parent, heading.then_some(0));
            }
        }
    }
}
