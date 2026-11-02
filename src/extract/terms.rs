//! Body words of symbols for search (`docs/spec.md` section 6), stored as normalized search
//! tokens so the derived index only splits them: the tokens of the distinct identifier-like
//! words of each symbol's lines outside nested symbols (identifiers, string contents, comments,
//! markup text), so a question in behavior words ("store the session cookie") finds the handler
//! whose body calls `cookies.set('jwt', ...)`. Each line counts for its innermost symbol only;
//! a symbol's lines start at the doc comment above it; lines outside every symbol (top-level
//! statements) are the file's words. URLs are skipped:
//! their host words (`api.realworld.show`) say nothing about behavior.

use std::collections::HashSet;

use crate::domain::facts::{BODY_TERMS, Facts, SymKind};
use crate::domain::search;

/// Keywords and literals common to most languages, and markup filler (`className`, `div`,
/// `span`): in nearly every body, never distinguishing.
const KEYWORDS: [&str; 51] = [
    "and",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "classname",
    "const",
    "continue",
    "def",
    "default",
    "div",
    "elif",
    "else",
    "end",
    "enum",
    "export",
    "extends",
    "false",
    "final",
    "for",
    "from",
    "func",
    "function",
    "impl",
    "import",
    "let",
    "mut",
    "new",
    "nil",
    "none",
    "not",
    "null",
    "pub",
    "return",
    "self",
    "span",
    "static",
    "struct",
    "super",
    "switch",
    "the",
    "then",
    "this",
    "throw",
    "true",
    "try",
    "undefined",
    "var",
    "while",
];

/// Fills [`crate::domain::facts::Symbol::terms`] for code symbols and
/// [`Facts::terms`] for top-level code from the file content. `spans` holds the syntax-tree byte
/// range of the leading symbols (the engine's); later symbols span their whole lines. Ownership
/// is by bytes, so one-line and minified symbols keep their own words and never take a
/// neighbor's.
pub fn add(content: &[u8], facts: &mut Facts, spans: &[(usize, usize)]) {
    let starts = line_starts(content);
    let line_start = |line: u32| starts.get(line.saturating_sub(1) as usize).copied();
    let line_end = |line: u32| {
        starts
            .get(line as usize)
            .map_or(content.len(), |next| next - 1)
    };
    let owned: Vec<(usize, usize)> = facts
        .symbols
        .iter()
        .enumerate()
        .map(|(index, symbol)| {
            let (start, end) = spans.get(index).copied().unwrap_or_else(|| {
                (
                    line_start(symbol.line).unwrap_or(0),
                    line_end(symbol.end_line.max(symbol.line)),
                )
            });
            // The doc comment directly above belongs to the symbol.
            let first = leading_comment_start(content, &starts, symbol.line);
            let start = if first < symbol.line {
                line_start(first).map_or(start, |comment| comment.min(start))
            } else {
                start
            };
            (start.min(content.len()), end.min(content.len()).max(start))
        })
        .collect();
    let ownership = if owned.len() >= MIN_OWNERSHIP_PLAN_SYMBOLS {
        OwnershipPlan::build(&owned, (0, content.len()))
    } else {
        None
    };
    let mut direct_children = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    facts.terms = match &ownership {
        Some(plan) => {
            plan.file_children(&mut direct_children);
            collect(content, (0, content.len()), &direct_children, &mut seen)
        }
        None => collect(content, (0, content.len()), &owned, &mut seen),
    };
    for (index, symbol) in facts.symbols.iter_mut().enumerate() {
        if matches!(
            symbol.kind(),
            SymKind::Fn
                | SymKind::Method
                | SymKind::Class
                | SymKind::Struct
                | SymKind::Enum
                | SymKind::Iface
                | SymKind::Trait
                | SymKind::Const
                | SymKind::Var
                | SymKind::Macro
                | SymKind::Table
                | SymKind::View
        ) {
            symbol.terms = match &ownership {
                Some(plan) => {
                    plan.children(index, &mut direct_children);
                    collect(content, owned[index], &direct_children, &mut seen)
                }
                None => collect(content, owned[index], &owned, &mut seen),
            };
        }
    }
}

/// A containment forest over owned byte ranges. Direct children cover every nested descendant,
/// so body extraction need not rescan them. Partial overlaps reject the plan and keep the legacy
/// all-ranges scan unchanged.
struct OwnershipPlan {
    intervals: Vec<OwnedInterval>,
    symbol_intervals: Vec<usize>,
    first_root: Option<usize>,
    file_root: Option<usize>,
}

const MIN_OWNERSHIP_PLAN_SYMBOLS: usize = 2048;

struct OwnedInterval {
    range: (usize, usize),
    first_child: Option<usize>,
    last_child: Option<usize>,
    next_sibling: Option<usize>,
}

impl OwnershipPlan {
    fn build(ranges: &[(usize, usize)], file_range: (usize, usize)) -> Option<Self> {
        let mut sorted: Vec<(usize, usize, usize)> = ranges
            .iter()
            .copied()
            .enumerate()
            .map(|(symbol, (first, last))| (first, last, symbol))
            .collect();
        sorted.sort_unstable_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1))
        });

        let mut plan = Self {
            intervals: Vec::with_capacity(ranges.len()),
            symbol_intervals: vec![0; ranges.len()],
            first_root: None,
            file_root: None,
        };
        let mut stack: Vec<usize> = Vec::new();
        let mut last_root: Option<usize> = None;
        let mut cursor = 0;
        while cursor < sorted.len() {
            let range = (sorted[cursor].0, sorted[cursor].1);
            let interval = plan.intervals.len();
            plan.intervals.push(OwnedInterval {
                range,
                first_child: None,
                last_child: None,
                next_sibling: None,
            });

            let mut next = cursor;
            while next < sorted.len() && (sorted[next].0, sorted[next].1) == range {
                plan.symbol_intervals[sorted[next].2] = interval;
                next += 1;
            }
            if range == file_range {
                plan.file_root = Some(interval);
            }

            while let Some(&previous) = stack.last() {
                if plan.intervals[previous].range.1 > range.0 {
                    break;
                }
                stack.pop();
            }
            if let Some(&parent) = stack.last() {
                if range.1 > plan.intervals[parent].range.1 {
                    return None;
                }
                if let Some(previous) = plan.intervals[parent].last_child {
                    plan.intervals[previous].next_sibling = Some(interval);
                } else {
                    plan.intervals[parent].first_child = Some(interval);
                }
                plan.intervals[parent].last_child = Some(interval);
            } else {
                if let Some(previous) = last_root {
                    plan.intervals[previous].next_sibling = Some(interval);
                } else {
                    plan.first_root = Some(interval);
                }
                last_root = Some(interval);
            }
            stack.push(interval);
            cursor = next;
        }
        Some(plan)
    }

    fn children(&self, symbol: usize, ranges: &mut Vec<(usize, usize)>) {
        let interval = self.symbol_intervals[symbol];
        self.ranges_from(self.intervals[interval].first_child, ranges);
    }

    fn file_children(&self, ranges: &mut Vec<(usize, usize)>) {
        let first = match self.file_root {
            Some(root) => self.intervals[root].first_child,
            None => self.first_root,
        };
        self.ranges_from(first, ranges);
    }

    fn ranges_from(&self, mut interval: Option<usize>, ranges: &mut Vec<(usize, usize)>) {
        ranges.clear();
        while let Some(index) = interval {
            let node = &self.intervals[index];
            ranges.push(node.range);
            interval = node.next_sibling;
        }
    }
}

/// Tokens of the distinct words of the bytes of `range` outside the ranges of `others` strictly
/// inside it (the range itself and identical ranges are not inside), line by line. A containment
/// plan supplies direct children because their bytes also cover every nested descendant.
fn collect(
    content: &[u8],
    (first, last): (usize, usize),
    others: &[(usize, usize)],
    seen: &mut HashSet<String>,
) -> String {
    let mut text = content[first..last].to_vec();
    for &(start, end) in others {
        if start >= first && end <= last && (start, end) != (first, last) {
            for byte in &mut text[start - first..end - first] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
        }
    }
    without_class_lists(&mut text);
    seen.clear();
    let mut terms = String::new();
    let mut count = 0;
    let mut line_words = String::new();
    let mut buffer = String::new();
    for line in text.split(|byte| *byte == b'\n') {
        if count >= BODY_TERMS {
            break;
        }
        // Search tokens of the line's new words (phrases such as `sign in` stay adjacent); a
        // token shared by several words (`delete` of `deleteUser`, `deleteComment`) repeats, so
        // its frequency still counts.
        line_words.clear();
        for_each_body_word(line, |word| {
            let lower = word.to_lowercase();
            if !KEYWORDS.contains(&lower.as_str()) && seen.insert(lower) {
                line_words.push_str(word);
                line_words.push(' ');
            }
        });
        search::for_each_token(&line_words, &mut buffer, |token| {
            if count >= BODY_TERMS || token.len() < 3 {
                return;
            }
            if !terms.is_empty() {
                terms.push(' ');
            }
            terms.push_str(token);
            count += 1;
        });
    }
    terms
}

/// Functions whose string arguments are CSS class names (`cn("flex", cond && "px-4")`).
const CLASS_CALLS: [&[u8]; 9] = [
    b"cn",
    b"clsx",
    b"classnames",
    b"classNames",
    b"cx",
    b"twMerge",
    b"twJoin",
    b"cva",
    b"tv",
];

/// Blanks CSS literals in exact `class`/`className` attributes, bound class expressions, Svelte
/// `class:<name>` directives and recognized class-composition calls ([`CLASS_CALLS`]). Dynamic
/// identifiers, comments and strings in nested ordinary calls stay searchable. Byte length and
/// line breaks are preserved.
fn without_class_lists(text: &mut [u8]) {
    let mut index = 0;
    while index < text.len() {
        if let Some(end) = comment_end(text, index, text.len()) {
            index = end;
            continue;
        }
        if matches!(text[index], b'\'' | b'"' | b'`') {
            index = skip_string(text, index, text.len());
            continue;
        }
        if text[index] == b'<'
            && let Some(end) = markup_tag(text, index)
        {
            index = end;
            continue;
        }
        if text[index] == b'('
            && is_class_call_before(text, index)
            && let Some(close) = matching_delimiter(text, index, b')', text.len())
        {
            class_expression(text, index + 1, close, true);
            index = close + 1;
            continue;
        }
        index += 1;
    }
}

fn markup_tag(text: &mut [u8], start: usize) -> Option<usize> {
    if text.get(start) != Some(&b'<') || !text.get(start + 1)?.is_ascii_alphabetic() {
        return None;
    }
    let mut index = start + 2;
    while text.get(index).is_some_and(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
    }) {
        index += 1;
    }
    if text
        .get(index)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>'))
    {
        return None;
    }

    while index < text.len() {
        if text[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        match text[index] {
            b'>' => return Some(index + 1),
            b'/' if text.get(index + 1) == Some(&b'>') => return Some(index + 2),
            b'{' => {
                index = matching_delimiter(text, index, b'}', text.len())? + 1;
            }
            _ => {
                let name_start = index;
                while text.get(index).is_some_and(|byte| {
                    !byte.is_ascii_whitespace() && !matches!(byte, b'=' | b'/' | b'>')
                }) {
                    index += 1;
                }
                if index == name_start {
                    index += 1;
                    continue;
                }
                let name_end = index;
                if let Some(suffix) = class_directive_suffix(&text[name_start..name_end]) {
                    blank(text, name_start + suffix, name_end);
                }
                let class_kind = class_attribute(&text[name_start..name_end]);
                while text.get(index).is_some_and(u8::is_ascii_whitespace) {
                    index += 1;
                }
                if text.get(index) != Some(&b'=') {
                    continue;
                }
                index += 1;
                while text.get(index).is_some_and(u8::is_ascii_whitespace) {
                    index += 1;
                }
                match text.get(index).copied() {
                    Some(quote @ (b'\'' | b'"')) => {
                        let value_start = index + 1;
                        let value_end = attribute_quote_end(text, index, quote)?;
                        if let Some(kind) = class_kind {
                            if kind == ClassAttribute::Static {
                                class_attribute_text(text, value_start, value_end);
                            } else {
                                class_expression(text, value_start, value_end, true);
                            }
                        }
                        index = value_end + 1;
                    }
                    Some(b'{') => {
                        let close = matching_delimiter(text, index, b'}', text.len())?;
                        if class_kind.is_some() {
                            class_expression(text, index + 1, close, true);
                        }
                        index = close + 1;
                    }
                    Some(_) => {
                        let value_start = index;
                        while text
                            .get(index)
                            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'>')
                        {
                            index += 1;
                        }
                        if class_kind == Some(ClassAttribute::Static) {
                            blank(text, value_start, index);
                        }
                    }
                    None => return None,
                }
            }
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClassAttribute {
    Static,
    Bound,
}

fn class_attribute(name: &[u8]) -> Option<ClassAttribute> {
    if name.eq_ignore_ascii_case(b"class") || name == b"className" {
        return Some(ClassAttribute::Static);
    }
    if matches!(
        name,
        b":class" | b"v-bind:class" | b"class:list" | b"[class]" | b"[className]"
    ) || (name.starts_with(b"[class.") && name.ends_with(b"]"))
    {
        return Some(ClassAttribute::Bound);
    }
    None
}

fn class_directive_suffix(name: &[u8]) -> Option<usize> {
    (name.starts_with(b"class:") && name.len() > b"class:".len()).then_some(b"class:".len())
}

fn class_attribute_text(text: &mut [u8], start: usize, end: usize) {
    let mut segment = start;
    let mut index = start;
    while index < end {
        if text[index] == b'\\' {
            index = (index + 2).min(end);
            continue;
        }
        if text[index] == b'{'
            && let Some(close) = matching_delimiter(text, index, b'}', end)
        {
            blank(text, segment, index);
            class_expression(text, index + 1, close, true);
            index = close + 1;
            segment = index;
            continue;
        }
        index += 1;
    }
    blank(text, segment, end);
}

fn class_expression(text: &mut [u8], start: usize, end: usize, owned: bool) {
    let mut index = start;
    while index < end {
        if let Some(comment_end) = comment_end(text, index, end) {
            index = comment_end;
            continue;
        }
        match text[index] {
            quote @ (b'\'' | b'"') => {
                let close = quoted_end(text, index, quote, end, false);
                if owned && close > index + 1 && text.get(close - 1) == Some(&quote) {
                    blank(text, index + 1, close - 1);
                }
                index = close;
            }
            b'`' => {
                index = class_template(text, index, end, owned);
            }
            byte if identifier_start(byte) => {
                let name_start = index;
                index += 1;
                while index < end && identifier_continue(text[index]) {
                    index += 1;
                }
                let name_end = index;
                let mut open = index;
                while open < end && text[open].is_ascii_whitespace() {
                    open += 1;
                }
                if text.get(open) == Some(&b'(')
                    && let Some(close) = matching_delimiter(text, open, b')', end)
                {
                    let child_owned = is_class_call(&text[name_start..name_end]);
                    class_expression(text, open + 1, close, child_owned);
                    index = close + 1;
                }
            }
            b'(' => {
                if let Some(close) = matching_delimiter(text, index, b')', end) {
                    class_expression(text, index + 1, close, owned);
                    index = close + 1;
                } else {
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
}

fn class_template(text: &mut [u8], start: usize, end: usize, owned: bool) -> usize {
    let mut segment = start + 1;
    let mut index = segment;
    while index < end {
        if text[index] == b'\\' {
            index = (index + 2).min(end);
            continue;
        }
        if text[index] == b'`' {
            if owned {
                blank(text, segment, index);
            }
            return index + 1;
        }
        if text[index] == b'$'
            && text.get(index + 1) == Some(&b'{')
            && let Some(close) = matching_delimiter(text, index + 1, b'}', end)
        {
            if owned {
                blank(text, segment, index);
            }
            class_expression(text, index + 2, close, owned);
            index = close + 1;
            segment = index;
            continue;
        }
        index += 1;
    }
    if owned {
        blank(text, segment, end);
    }
    end
}

fn is_class_call_before(text: &[u8], open: usize) -> bool {
    let mut end = open;
    while end > 0 && text[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && identifier_continue(text[start - 1]) {
        start -= 1;
    }
    start < end && is_class_call(&text[start..end])
}

fn is_class_call(name: &[u8]) -> bool {
    CLASS_CALLS.contains(&name)
}

fn identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

fn identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn comment_end(text: &[u8], start: usize, limit: usize) -> Option<usize> {
    let tail = text.get(start..limit)?;
    if tail.starts_with(b"//") {
        return Some(
            start
                + tail
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .unwrap_or(tail.len()),
        );
    }
    if tail.starts_with(b"/*") {
        return Some(
            start
                + tail[2..]
                    .windows(2)
                    .position(|window| window == b"*/")
                    .map_or(tail.len(), |offset| offset + 4),
        );
    }
    if tail.starts_with(b"<!--") {
        return Some(
            start
                + tail[4..]
                    .windows(3)
                    .position(|window| window == b"-->")
                    .map_or(tail.len(), |offset| offset + 7),
        );
    }
    None
}

fn matching_delimiter(text: &[u8], open: usize, close: u8, limit: usize) -> Option<usize> {
    let opening = *text.get(open)?;
    let mut depth = 0usize;
    let mut index = open;
    while index < limit {
        if let Some(end) = comment_end(text, index, limit) {
            index = end;
            continue;
        }
        match text[index] {
            quote @ (b'\'' | b'"') => index = quoted_end(text, index, quote, limit, false),
            b'`' => index = template_end(text, index, limit),
            byte if byte == opening => {
                depth += 1;
                index += 1;
            }
            byte if byte == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn template_end(text: &[u8], start: usize, limit: usize) -> usize {
    let mut index = start + 1;
    while index < limit {
        if text[index] == b'\\' {
            index = (index + 2).min(limit);
            continue;
        }
        if text[index] == b'`' {
            return index + 1;
        }
        if text[index] == b'$'
            && text.get(index + 1) == Some(&b'{')
            && let Some(close) = matching_delimiter(text, index + 1, b'}', limit)
        {
            index = close + 1;
            continue;
        }
        index += 1;
    }
    limit
}

fn skip_string(text: &[u8], start: usize, limit: usize) -> usize {
    match text[start] {
        b'`' => template_end(text, start, limit),
        quote => quoted_end(text, start, quote, limit, false),
    }
}

fn quoted_end(text: &[u8], start: usize, quote: u8, limit: usize, multiline: bool) -> usize {
    let mut index = start + 1;
    while index < limit {
        match text[index] {
            b'\\' => index = (index + 2).min(limit),
            byte if byte == quote => return index + 1,
            b'\n' if !multiline => return index,
            _ => index += 1,
        }
    }
    limit
}

fn attribute_quote_end(text: &[u8], start: usize, quote: u8) -> Option<usize> {
    let end = quoted_end(text, start, quote, text.len(), true);
    (end > start + 1 && text.get(end - 1) == Some(&quote)).then_some(end - 1)
}

fn blank(text: &mut [u8], start: usize, end: usize) {
    for byte in &mut text[start..end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

/// First line of the comment and annotation block directly above `line` (`///`, `/** */`,
/// `#`, `--`, `@Decorator`), else `line`: the full doc comment counts for the symbol.
fn leading_comment_start(content: &[u8], starts: &[usize], line: u32) -> u32 {
    let mut first = line;
    while first > 1 {
        let Some(text) = line_text(content, starts, first - 1) else {
            break;
        };
        let trimmed = text.trim_ascii_start();
        let comment = [&b"//"[..], b"/*", b"*", b"#", b"--", b"@"]
            .iter()
            .any(|marker| trimmed.starts_with(marker));
        if !comment {
            break;
        }
        first -= 1;
    }
    first
}

/// Byte offset of every line start.
fn line_starts(content: &[u8]) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            content
                .iter()
                .enumerate()
                .filter(|&(_, &byte)| byte == b'\n')
                .map(|(index, _)| index + 1),
        )
        .collect()
}

/// Bytes of the 1-based line, without its newline.
fn line_text<'a>(content: &'a [u8], starts: &[usize], line: u32) -> Option<&'a [u8]> {
    let start = *starts.get(line.checked_sub(1)? as usize)?;
    let end = starts
        .get(line as usize)
        .map_or(content.len(), |next| next - 1);
    content.get(start..end.max(start))
}

/// Unicode alphanumeric source words of at least two UTF-8 bytes outside URLs; digits-only words
/// are excluded. Invalid UTF-8 becomes separators while its valid neighboring words survive.
fn for_each_body_word(text: &[u8], mut emit: impl FnMut(&str)) {
    for chunk in text.split(|byte| {
        byte.is_ascii_whitespace() || matches!(byte, b'\'' | b'"' | b'`' | b'(' | b')')
    }) {
        if chunk.windows(3).any(|window| window == b"://") {
            continue;
        }
        let chunk = String::from_utf8_lossy(chunk);
        search::for_each_word(&chunk, |word| {
            if word.len() >= 2 && word.chars().any(char::is_alphabetic) {
                emit(word);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::facts::Symbol;

    fn function_facts(count: usize) -> Facts {
        Facts {
            symbols: (0..count)
                .map(|index| Symbol {
                    kind: Some(SymKind::Fn),
                    name: format!("function_{index}"),
                    line: 1,
                    end_line: 1,
                    ..Symbol::default()
                })
                .collect(),
            ..Facts::default()
        }
    }

    /// Original all-ranges behavior used as the equivalence oracle for the optimized plan.
    fn legacy_add(content: &[u8], initial: &Facts, ranges: &[(usize, usize)]) -> Facts {
        let mut facts = initial.clone();
        let mut seen = HashSet::new();
        facts.terms = collect(content, (0, content.len()), ranges, &mut seen);
        for (index, symbol) in facts.symbols.iter_mut().enumerate() {
            symbol.terms = collect(content, ranges[index], ranges, &mut seen);
        }
        facts
    }

    #[test]
    fn ownership_plan_preserves_nested_equal_zero_width_and_single_line_facts() -> Result<(), String>
    {
        let content = b"alpha beta gamma delta";
        let mut ranges = vec![(0, 22), (6, 16), (6, 16), (11, 16), (12, 12), (17, 22)];
        ranges.extend(std::iter::repeat_n(
            (6, 16),
            MIN_OWNERSHIP_PLAN_SYMBOLS - ranges.len(),
        ));
        let initial = function_facts(ranges.len());
        let expected = legacy_add(content, &initial, &ranges);
        let mut actual = initial;
        add(content, &mut actual, &ranges);

        assert_eq!(actual, expected);
        assert_eq!(actual.symbols[4].terms, "");

        let plan = OwnershipPlan::build(&ranges, (0, content.len()))
            .ok_or_else(|| "expected nested spans".to_owned())?;
        let mut children = Vec::new();
        plan.children(0, &mut children);
        assert_eq!(children.len(), 2);
        plan.children(1, &mut children);
        assert_eq!(children.len(), 1);
        plan.children(2, &mut children);
        assert_eq!(children.len(), 1);
        plan.children(3, &mut children);
        assert_eq!(children.len(), 1);
        plan.children(4, &mut children);
        assert_eq!(children.len(), 0);
        plan.children(5, &mut children);
        assert_eq!(children.len(), 0);
        plan.file_children(&mut children);
        assert_eq!(children.len(), 2);
        Ok(())
    }

    #[test]
    fn partially_overlapping_spans_keep_the_legacy_facts() {
        let content = b"alpha beta gamma";
        let mut ranges = vec![(0, 10), (6, 16)];
        ranges.extend(std::iter::repeat_n(
            (16, 16),
            MIN_OWNERSHIP_PLAN_SYMBOLS - ranges.len(),
        ));
        let initial = function_facts(ranges.len());
        assert!(OwnershipPlan::build(&ranges, (0, content.len())).is_none());

        let expected = legacy_add(content, &initial, &ranges);
        let mut actual = initial;
        add(content, &mut actual, &ranges);
        assert_eq!(actual, expected);
    }

    #[test]
    fn body_words_skip_nested_symbols_and_keywords() {
        let content =
            b"class Session {\n  ttl = 30\n  save() {\n    cookies.set('jwt', token)\n  }\n}\n";
        let mut facts = Facts {
            symbols: vec![
                Symbol {
                    kind: Some(SymKind::Class),
                    name: "Session".to_owned(),
                    line: 1,
                    end_line: 6,
                    ..Symbol::default()
                },
                Symbol {
                    kind: Some(SymKind::Method),
                    name: "save".to_owned(),
                    parent: Some(0),
                    line: 3,
                    end_line: 5,
                    ..Symbol::default()
                },
            ],
            ..Facts::default()
        };
        add(content, &mut facts, &[]);
        assert_eq!(facts.symbols[0].terms, "session ttl");
        assert_eq!(facts.symbols[1].terms, "save cooky set jwt token");
        assert_eq!(facts.terms, "");
    }

    #[test]
    fn unicode_body_words_match_query_tokenization() {
        let content = "/// Öffne den RückgabeWert für die Übersicht.\nfn öffneÜberCache() {\n    let mietÜbersicht = \"RückgabeWert\";\n    // Prüfe den Zustand 中\n}\n";
        let mut facts = Facts {
            symbols: vec![Symbol {
                kind: Some(SymKind::Fn),
                name: "öffneÜberCache".to_owned(),
                line: 2,
                end_line: 5,
                ..Symbol::default()
            }],
            ..Facts::default()
        };

        add(content.as_bytes(), &mut facts, &[]);

        for token in
            search::tokenize("öffne über cache mietÜbersicht RückgabeWert Prüfe Zustand 中")
        {
            assert!(
                facts.symbols[0]
                    .terms
                    .split_whitespace()
                    .any(|indexed| indexed == token),
                "body index omitted query token {token:?}: {}",
                facts.symbols[0].terms
            );
        }
    }

    #[test]
    fn invalid_utf8_bytes_separate_words_without_losing_neighbors() {
        let mut words = Vec::new();
        for_each_body_word(b"before \xff after", |word| words.push(word.to_owned()));
        assert_eq!(words, ["before", "after"]);
    }

    fn kept_words(text: &str) -> Vec<String> {
        let mut bytes = text.as_bytes().to_vec();
        without_class_lists(&mut bytes);
        assert_eq!(bytes.len(), text.len());
        let mut words = Vec::new();
        for_each_body_word(&bytes, |word| words.push(word.to_owned()));
        words
    }

    #[test]
    fn class_lists_are_dropped_only_in_class_contexts() {
        let cases: [(&str, &[&str]); 9] = [
            (
                "<div className=\"flex items-center gap-2\">Sign out</div>",
                &["div", "className", "Sign", "out", "div"],
            ),
            (
                "<nav class='md:hidden px-4'>Menu</nav>",
                &["nav", "class", "Menu", "nav"],
            ),
            (
                "<p :class=\"{ 'text-red-500': failed }\">x</p>",
                &["class", "failed"],
            ),
            (
                "cn(\n  \"flex items-center\",\n  active && \"bg-blue-500\"\n)",
                &["cn", "active"],
            ),
            (
                "<div className={`grid ${wide ? \"col-span-2\" : \"\"}`}></div>",
                &["div", "className", "wide", "div"],
            ),
            (
                "label(\"small-screen navigation\"); log.warn('read-only mode')",
                &[
                    "label",
                    "small",
                    "screen",
                    "navigation",
                    "log",
                    "warn",
                    "read",
                    "only",
                    "mode",
                ],
            ),
            (
                "// set className=\"flex gap-2\" here",
                &["set", "className", "flex", "gap", "here"],
            ),
            (
                "say(\"use className=\\\"flex gap-2\\\" carefully\")",
                &["say", "use", "className", "flex", "gap", "carefully"],
            ),
            (
                "const styles = \"flex items-center\"; title(\"data-test-id\")",
                &[
                    "const", "styles", "flex", "items", "center", "title", "data", "test", "id",
                ],
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(kept_words(text), expected, "{text}");
        }
    }

    #[test]
    fn class_attribute_names_and_composition_calls_have_exact_ownership() {
        let source = r#"
const asset_class = "asset-description-code";
const classes = cn("flex gap-2", selected && "bg-blue-500", labelFor("ordinary-caption-text"));
<div asset_class="asset-attribute-description" class="rounded border-dashed" aria-label="screen-reader-description" />
"#;
        let words = kept_words(source);
        for expected in [
            "asset_class",
            "description",
            "code",
            "attribute",
            "ordinary",
            "caption",
            "screen",
            "reader",
        ] {
            assert!(
                words.iter().any(|word| word == expected),
                "missing {expected}: {words:?}"
            );
        }
        for excluded in ["flex", "gap", "blue", "rounded", "border", "dashed"] {
            assert!(
                !words.iter().any(|word| word == excluded),
                "class word {excluded} survived: {words:?}"
            );
        }
    }

    #[test]
    fn jsx_conditional_and_template_classes_keep_dynamic_and_ordinary_words() {
        let source = r#"
const view = <div className={active ? "bg-blue-500" : "bg-gray-500"} aria-label="mobile-navigation-copy" />;
const templated = <span className={`grid ${wide ? "col-span-2" : "col-span-1"}`} title="read-only-copy" />;
const ordinary = "className=\"literal-violet-500\"";
<div className="escaped-amber-500\"still-class" aria-label="after-escape-description" />
<!-- <div class="comment-magenta-code"> -->
"#;
        let words = kept_words(source);
        for expected in [
            "active",
            "wide",
            "mobile",
            "navigation",
            "copy",
            "read",
            "only",
            "violet",
            "magenta",
            "after",
            "escape",
            "description",
        ] {
            assert!(
                words.iter().any(|word| word == expected),
                "missing {expected}: {words:?}"
            );
        }
        for excluded in ["blue", "gray", "grid", "col", "amber"] {
            assert!(
                !words.iter().any(|word| word == excluded),
                "class word {excluded} survived: {words:?}"
            );
        }
    }

    #[test]
    fn svelte_conditional_classes_keep_state_terms_without_directive_names() {
        let source = r#"
<button class="rounded-md px-4 {isReady ? 'bg-green-500' : 'bg-gray-500'}" class:highlighted={isSelected} aria-label="save-button-label">Save</button>
"#;
        let words = kept_words(source);
        for expected in ["isReady", "isSelected", "save", "button", "label"] {
            assert!(
                words.iter().any(|word| word == expected),
                "missing {expected}: {words:?}"
            );
        }
        for excluded in ["rounded", "green", "gray", "highlighted"] {
            assert!(
                !words.iter().any(|word| word == excluded),
                "class word {excluded} survived: {words:?}"
            );
        }
    }

    #[test]
    fn vue_bound_class_conditions_keep_state_and_hide_class_literals() {
        let source = r#"
<div :class="isSelected ? 'bg-red-500' : 'bg-blue-500'" class="rounded-md" :aria-label="'accessible-description'"></div>
"#;
        let words = kept_words(source);
        for expected in ["isSelected", "accessible", "description"] {
            assert!(
                words.iter().any(|word| word == expected),
                "missing {expected}: {words:?}"
            );
        }
        for excluded in ["red", "blue", "rounded"] {
            assert!(
                !words.iter().any(|word| word == excluded),
                "class word {excluded} survived: {words:?}"
            );
        }
    }

    #[test]
    fn doc_comments_above_a_symbol_are_its_words() {
        let content = b"setup(app)\n/**\n * Compose middleware into one handler.\n */\nfunction compose(a) {\n  return a\n}\n";
        let mut facts = Facts {
            symbols: vec![Symbol {
                kind: Some(SymKind::Fn),
                name: "compose".to_owned(),
                line: 5,
                end_line: 7,
                ..Symbol::default()
            }],
            ..Facts::default()
        };
        add(content, &mut facts, &[]);
        assert_eq!(facts.symbols[0].terms, "compos middlewar into one handler");
        assert_eq!(facts.terms, "setup app");
    }

    #[test]
    fn top_level_code_belongs_to_the_file_and_urls_are_skipped() {
        let content = b"const base = 'https://api.example.show/v1'\napi.interceptors.use(notify)\n";
        let mut facts = Facts {
            symbols: vec![Symbol {
                kind: Some(SymKind::Const),
                name: "base".to_owned(),
                line: 1,
                end_line: 1,
                ..Symbol::default()
            }],
            ..Facts::default()
        };
        add(content, &mut facts, &[]);
        // A one-line symbol owns its line; URLs stay skipped.
        assert_eq!(facts.symbols[0].terms, "base");
        assert_eq!(facts.terms, "api interceptor use notify");
    }
}
