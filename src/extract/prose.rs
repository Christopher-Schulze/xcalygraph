//! Prose documents other than Markdown (`docs/spec.md` sections 5 and 6): plain text,
//! reStructuredText, Org, AsciiDoc, LaTeX and HTML. Headings become symbols, relative links and
//! includes path references, inline code identifiers and paths mentions. Line scanners: no
//! grammar, tolerant of malformed input. `sections.rs` turns all their text into search terms;
//! binary documents (`documents.rs`) share the outline.

use super::sections;
use crate::domain::facts::{self, Facts, NO_ARITY, Ref, RefKind, SymKind, Symbol};
use crate::domain::filetype::FileType;

pub fn extract(file_type: FileType, text: &str) -> Facts {
    match file_type {
        FileType::Html => html(text),
        FileType::Rst => rst(text),
        _ => {
            let mut outline = Outline::new(text);
            for (index, line) in text.lines().enumerate() {
                let number = index as u32 + 1;
                match file_type {
                    FileType::Org => org_line(&mut outline, line, number),
                    FileType::AsciiDoc => asciidoc_line(&mut outline, line, number),
                    FileType::Tex => tex_line(&mut outline, line, number),
                    _ => outline.prose(line, number),
                }
            }
            outline.finish()
        }
    }
}

/// Headings, references and text of one document.
pub(super) struct Outline {
    facts: Facts,
    /// Open headings by level, as symbol indices.
    open: Vec<(usize, u32)>,
    line_count: u32,
    paragraph: Vec<String>,
    paragraph_done: bool,
    /// Text with its locator, for formats that are not read line by line afterwards.
    keep_text: bool,
    body: Vec<(u32, String)>,
    locator: u32,
}

impl Outline {
    pub(super) fn new(text: &str) -> Self {
        Self {
            keep_text: false,
            ..Self::with_text(text.lines().count() as u32)
        }
    }

    /// An outline over `line_count` locators (pages, paragraphs or sheets) keeping section text.
    pub(super) fn with_text(line_count: u32) -> Self {
        Self {
            facts: Facts::default(),
            open: Vec::new(),
            line_count,
            paragraph: Vec::new(),
            paragraph_done: false,
            keep_text: true,
            body: Vec::new(),
            locator: 1,
        }
    }

    pub(super) fn heading(&mut self, level: usize, title: &str, line: u32) {
        let title = facts::clip_space(title, 160);
        if title.is_empty() {
            return;
        }
        while self
            .open
            .last()
            .is_some_and(|&(open_level, _)| open_level >= level)
        {
            if let Some((_, closed)) = self.open.pop() {
                self.facts.symbols[closed as usize].end_line = line.saturating_sub(1).max(1);
            }
        }
        let parent = self.open.last().map(|&(_, symbol)| symbol);
        let id = self.facts.symbols.len() as u32;
        self.facts.symbols.push(Symbol {
            kind: Some(SymKind::Heading),
            name: title.clone().into(),
            parent,
            line,
            end_line: self.line_count.max(line),
            exported: true,
            ..Symbol::default()
        });
        self.open.push((level, id));
        self.paragraph_done |= !self.paragraph.is_empty();
        self.locator = line;
    }

    fn scope(&self) -> Option<u32> {
        self.open.last().map(|&(_, symbol)| symbol)
    }

    /// A relative link or include; URLs, anchors and mail links are skipped.
    pub(super) fn path(&mut self, target: &str, line: u32) {
        let target = target
            .trim()
            .trim_matches(['<', '>', '"', '\''])
            .split(['#', '?'])
            .next()
            .unwrap_or_default();
        let external = target.contains("://")
            || target.starts_with("mailto:")
            || target.starts_with("tel:")
            || target.starts_with("data:")
            || target.starts_with("javascript:");
        if target.is_empty() || external || target.contains(char::is_whitespace) {
            return;
        }
        let scope = self.scope();
        self.facts.refs.push(Ref {
            kind: RefKind::Path,
            name: target.into(),
            qualifier: None,
            scope,
            line,
            arity: NO_ARITY,
            byte: crate::domain::facts::NO_BYTE,
        });
    }

    /// Inline code naming an identifier (`Store`, `store.Get`, `crate::x`) or a relative path.
    pub(super) fn mention(&mut self, inside: &str, line: u32) {
        let inside = inside.trim().trim_end_matches("()");
        let identifier_like = !inside.is_empty()
            && inside.len() <= 120
            && inside
                .chars()
                .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.' | ':' | '/' | '-'))
            && inside.chars().any(char::is_alphabetic)
            && !inside.starts_with('-');
        let scope = self.scope();
        let duplicate = self.facts.refs.iter().any(|reference| {
            reference.kind == RefKind::Mention
                && reference.scope == scope
                && reference.name == inside
        });
        if identifier_like && !duplicate {
            self.facts.refs.push(Ref {
                kind: RefKind::Mention,
                name: inside.into(),
                qualifier: None,
                scope,
                line,
                arity: NO_ARITY,
                byte: crate::domain::facts::NO_BYTE,
            });
        }
    }

    /// Body text: the first paragraph becomes the file doc, all text feeds search segments.
    pub(super) fn text(&mut self, text: &str) {
        let text = text.trim();
        if !self.paragraph_done {
            if text.is_empty() {
                self.paragraph_done = !self.paragraph.is_empty();
            } else {
                self.paragraph.push(text.to_owned());
            }
        }
        if self.keep_text && !text.is_empty() {
            self.body.push((self.locator, text.to_owned()));
        }
    }

    /// The page, paragraph or line the following text belongs to.
    pub(super) fn at(&mut self, locator: u32) {
        self.locator = locator;
    }

    /// Plain line: backticked mentions and text.
    pub(super) fn prose(&mut self, line: &str, number: u32) {
        for inside in delimited(line, "`", "`") {
            self.mention(inside, number);
        }
        self.locator = number;
        self.text(line);
    }

    pub(super) fn finish(mut self) -> Facts {
        self.facts.doc = facts::clip_space(&self.paragraph.join(" "), 400);
        self.facts.lines = self.line_count;
        let body = std::mem::take(&mut self.body);
        sections::index_text(
            &mut self.facts,
            body.iter().map(|(locator, text)| (*locator, text.as_str())),
        );
        self.facts
    }
}

/// Contents between `open` and `close` markers on one line (`` `x` ``, `~x~`).
pub(super) fn delimited<'a>(line: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(close) else { break };
        found.push(&after[..end]);
        rest = &after[end + close.len()..];
    }
    found
}

/// reStructuredText: titles underlined (and optionally overlined) with one punctuation
/// character, levels in order of first use; `` `text <target>`_ `` links, `image`, `figure`,
/// `include` and `literalinclude` directives; ``code`` and `:role:`name`` mentions.
fn rst(text: &str) -> Facts {
    let lines: Vec<&str> = text.lines().collect();
    let mut outline = Outline::new(text);
    let mut styles: Vec<(char, bool)> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let number = index as u32 + 1;
        let next = lines.get(index + 1).copied().unwrap_or_default();
        let title = line.trim();
        if !title.is_empty()
            && !is_adornment(line)
            && let Some(mark) = adornment(next)
            && next.trim_end().chars().count() >= title.chars().count()
        {
            let overlined = index > 0 && adornment(lines[index - 1]) == Some(mark);
            let level = match styles.iter().position(|&style| style == (mark, overlined)) {
                Some(position) => position + 1,
                None => {
                    styles.push((mark, overlined));
                    styles.len()
                }
            };
            outline.heading(level, title, number);
            index += 2;
            continue;
        }
        if is_adornment(line) {
            index += 1;
            continue;
        }
        let trimmed = line.trim_start();
        for directive in ["image::", "figure::", "include::", "literalinclude::"] {
            if let Some(target) = trimmed
                .strip_prefix(".. ")
                .and_then(|rest| rest.strip_prefix(directive))
            {
                outline.path(target, number);
            }
        }
        for link in delimited(line, "`", "`_") {
            if let Some((_, target)) = link.rsplit_once('<') {
                outline.path(target.trim_end_matches('>'), number);
            }
        }
        for code in delimited(line, "``", "``") {
            outline.mention(code, number);
        }
        for role in [
            ":func:`", ":class:`", ":meth:`", ":mod:`", ":attr:`", ":obj:`", ":file:`",
        ] {
            for name in delimited(line, role, "`") {
                outline.mention(name.trim_start_matches(['~', '.']), number);
            }
        }
        if !trimmed.starts_with("..") {
            outline.text(line);
        }
        index += 1;
    }
    outline.finish()
}

fn adornment(line: &str) -> Option<char> {
    is_adornment(line)
        .then(|| line.trim_end().chars().next())
        .flatten()
}

fn is_adornment(line: &str) -> bool {
    let line = line.trim_end();
    let Some(first) = line.chars().next() else {
        return false;
    };
    line.chars().count() >= 3
        && "=-~^\"'*+#:.`_".contains(first)
        && line.chars().all(|ch| ch == first)
}

/// Org: `*` headings; `[[target][text]]` and `[[target]]` links (`file:` prefix stripped);
/// `~code~` and `=verbatim=` mentions.
fn org_line(outline: &mut Outline, line: &str, number: u32) {
    let stars = line.bytes().take_while(|&byte| byte == b'*').count();
    if stars > 0 && line.as_bytes().get(stars) == Some(&b' ') {
        outline.heading(stars, &line[stars..], number);
        return;
    }
    if line.trim_start().starts_with('#') {
        return;
    }
    for link in delimited(line, "[[", "]]") {
        let target = link.split("][").next().unwrap_or(link);
        outline.path(target.strip_prefix("file:").unwrap_or(target), number);
    }
    for code in delimited(line, "~", "~")
        .into_iter()
        .chain(delimited(line, "=", "="))
    {
        outline.mention(code, number);
    }
    outline.text(line);
}

/// AsciiDoc: `=` headings (`=` is the document title); `link:`, `xref:`, `include::`,
/// `image::` and `image:` targets; backticked mentions.
fn asciidoc_line(outline: &mut Outline, line: &str, number: u32) {
    let marks = line.bytes().take_while(|&byte| byte == b'=').count();
    if (1..=6).contains(&marks) && line.as_bytes().get(marks) == Some(&b' ') {
        outline.heading(marks, &line[marks..], number);
        return;
    }
    if line.starts_with("//") {
        return;
    }
    for macro_name in ["include::", "image::", "link:", "xref:", "image:"] {
        let mut rest = line;
        while let Some(start) = rest.find(macro_name) {
            let after = &rest[start + macro_name.len()..];
            let end = after.find('[').unwrap_or(after.len());
            let target = &after[..end];
            if !target.starts_with(':') && !target.contains(char::is_whitespace) {
                outline.path(target, number);
            }
            rest = &after[end..];
        }
    }
    outline.prose(line, number);
}

/// LaTeX: sectioning commands as headings; `\input`, `\include` (`.tex` implied),
/// `\includegraphics` and `\href` targets; `\texttt` and `\verb|x|` mentions; `%` comments.
fn tex_line(outline: &mut Outline, line: &str, number: u32) {
    let code = strip_tex_comment(line);
    for (level, command) in [
        "\\part",
        "\\chapter",
        "\\section",
        "\\subsection",
        "\\subsubsection",
        "\\paragraph",
    ]
    .iter()
    .enumerate()
    {
        for starred in ["{", "*{"] {
            for title in delimited(code, &format!("{command}{starred}"), "}") {
                outline.heading(level + 1, title, number);
            }
        }
    }
    for command in ["\\input{", "\\include{"] {
        for target in delimited(code, command, "}") {
            let has_extension = target
                .rsplit('/')
                .next()
                .is_some_and(|name| name.contains('.'));
            if has_extension {
                outline.path(target, number);
            } else {
                outline.path(&format!("{target}.tex"), number);
            }
        }
    }
    for target in delimited(code, "\\includegraphics", "}") {
        outline.path(target.rsplit('{').next().unwrap_or(target), number);
    }
    for target in delimited(code, "\\href{", "}") {
        outline.path(target, number);
    }
    for name in delimited(code, "\\texttt{", "}")
        .into_iter()
        .chain(delimited(code, "\\verb|", "|"))
    {
        outline.mention(&name.replace("\\_", "_"), number);
    }
    if !code.trim_start().starts_with('\\') {
        outline.text(code);
    }
}

fn strip_tex_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && (index == 0 || bytes[index - 1] != b'\\') {
            return &line[..index];
        }
        index += 1;
    }
    line
}

/// HTML: `<h1>`..`<h6>` headings, `href` and `src` attributes, `<code>` mentions, visible text
/// (script and style contents skipped, `<title>` as the file doc fallback).
fn html(text: &str) -> Facts {
    let mut outline = Outline::new(text);
    outline.keep_text = true;
    let mut line = 1u32;
    let mut position = 0;
    let mut visible = String::new();
    // Open capture: heading level or code, with its start line and text.
    let mut capture: Option<(Capture, u32, String)> = None;
    let mut title = String::new();
    let mut in_title = false;
    let skip_to = |from: usize, marker: &str, lowercase: bool| -> usize {
        let found = if lowercase {
            text[from..].to_ascii_lowercase().find(marker)
        } else {
            text[from..].find(marker)
        };
        found.map_or(text.len(), |offset| from + offset)
    };
    while position < text.len() {
        let rest = &text[position..];
        if !rest.starts_with('<') {
            let end = skip_to(position, "<", false);
            let chunk = &text[position..end];
            let decoded = decode_entities(chunk);
            match &mut capture {
                Some((kind, _, captured)) => {
                    if let Capture::Code(code) = kind {
                        code.non_variable_text |=
                            code.variable_depth == 0 && !decoded.trim().is_empty();
                    }
                    captured.push_str(&decoded);
                }
                None if in_title => title.push_str(&decoded),
                None => visible.push_str(&decoded),
            }
            line += chunk.matches('\n').count() as u32;
            position = end;
            continue;
        }
        if rest.starts_with("<!--") {
            let end = (skip_to(position, "-->", false) + 3).min(text.len());
            line += text[position..end].matches('\n').count() as u32;
            position = end;
            continue;
        }
        let end = (skip_to(position, ">", false) + 1).min(text.len());
        let tag = &text[position..end];
        let tag_line = line;
        line += tag.matches('\n').count() as u32;
        position = end;
        let name = tag_name(tag);
        let closing = tag.starts_with("</");
        if matches!(name.as_str(), "i" | "var")
            && let Some((Capture::Code(code), _, _)) = &mut capture
        {
            if closing {
                code.variable_depth = code.variable_depth.saturating_sub(1);
            } else {
                code.variable = true;
                code.variable_depth = code.variable_depth.saturating_add(1);
            }
        }
        if !closing {
            for attribute in ["href", "src"] {
                if let Some(value) = attribute_value(tag, attribute) {
                    outline.path(value, tag_line);
                }
            }
        }
        if matches!(name.as_str(), "script" | "style") && !closing {
            let skipped_end = skip_to(position, &format!("</{name}"), true);
            line += text[position..skipped_end].matches('\n').count() as u32;
            position = skipped_end;
            continue;
        }
        if name == "title" {
            in_title = !closing;
            continue;
        }
        let level = match name.as_bytes() {
            [b'h', digit] if (b'1'..=b'6').contains(digit) => Some((digit - b'0') as usize),
            _ => None,
        };
        match (level, name.as_str(), closing) {
            (Some(level), _, false) => {
                capture = Some((Capture::Heading(level), tag_line, String::new()));
            }
            (_, "code", false) if capture.is_none() => {
                capture = Some((
                    Capture::Code(CodeCapture::default()),
                    tag_line,
                    String::new(),
                ));
            }
            (Some(_), _, true) | (_, "code", true) => match capture.take() {
                Some((Capture::Heading(level), start, captured)) => {
                    flush_visible(&mut outline, &mut visible, line);
                    outline.heading(level, &captured, start);
                }
                Some((Capture::Code(code), start, captured)) => {
                    if !code.variable
                        || code.non_variable_text
                        || captured.chars().any(|ch| matches!(ch, '.' | ':' | '/'))
                    {
                        outline.mention(&captured, start);
                    }
                    visible.push_str(&captured);
                }
                None => {}
            },
            _ if is_block(&name) => flush_visible(&mut outline, &mut visible, line),
            _ => {}
        }
    }
    flush_visible(&mut outline, &mut visible, line);
    let mut facts = outline.finish();
    if facts.doc.is_empty() {
        facts.doc = facts::clip_space(&title, 400);
    }
    facts
}

enum Capture {
    Heading(usize),
    Code(CodeCapture),
}

#[derive(Default)]
struct CodeCapture {
    variable: bool,
    variable_depth: usize,
    non_variable_text: bool,
}

fn flush_visible(outline: &mut Outline, visible: &mut String, line: u32) {
    let text = facts::normalize_space(visible);
    visible.clear();
    if !text.is_empty() {
        outline.at(line);
        outline.text(&text);
        // A block ends a paragraph for the file doc.
        outline.text("");
    }
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "li"
            | "ul"
            | "ol"
            | "table"
            | "tr"
            | "section"
            | "article"
            | "br"
            | "pre"
            | "blockquote"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "dd"
            | "dt"
    )
}

fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('<')
        .trim_start_matches('/')
        .split(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn attribute_value<'a>(tag: &'a str, attribute: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find(attribute) {
        let start = search + found;
        search = start + attribute.len();
        let before_ok = start > 0 && lower.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = tag[search..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let (quote, body) = match value.chars().next()? {
            quote @ ('"' | '\'') => (quote, &value[1..]),
            _ => {
                let end = value
                    .find(|ch: char| ch.is_whitespace() || ch == '>')
                    .unwrap_or(value.len());
                return Some(&value[..end]);
            }
        };
        return body.find(quote).map(|end| &body[..end]);
    }
    None
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(facts: &Facts) -> (Vec<String>, Vec<String>) {
        let symbols = facts
            .symbols
            .iter()
            .map(|symbol| {
                let parent = symbol.parent.map_or("-".to_owned(), |p| {
                    facts.symbols[p as usize].name.to_string()
                });
                format!(
                    "{}-{} {} <{parent}",
                    symbol.line, symbol.end_line, symbol.name
                )
            })
            .collect();
        let refs = facts
            .refs
            .iter()
            .map(|reference| format!("{} {:?} {}", reference.line, reference.kind, reference.name))
            .collect();
        (symbols, refs)
    }

    #[test]
    fn rst_titles_links_directives_and_roles() {
        let text = "=====\nGuide\n=====\n\nIntro with ``Store`` and :func:`store.open`.\n\nUsage\n-----\n\nSee `api <api.rst>`_ and `site <https://x.io>`_.\n\n.. image:: img/logo.png\n.. include:: parts/intro.rst\n\nDeep\n~~~~\n\nText.\n";
        let facts = extract(FileType::Rst, text);
        let (symbols, refs) = summary(&facts);
        assert_eq!(
            symbols,
            ["2-18 Guide <-", "7-18 Usage <Guide", "15-18 Deep <Usage"]
        );
        assert_eq!(
            refs,
            [
                "5 Mention Store",
                "5 Mention store.open",
                "10 Path api.rst",
                "12 Path img/logo.png",
                "13 Path parts/intro.rst"
            ]
        );
        assert_eq!(facts.doc, "Intro with ``Store`` and :func:`store.open`.");
    }

    #[test]
    fn org_asciidoc_and_tex_headings_and_references() {
        let org = extract(
            FileType::Org,
            "#+TITLE: x\n* Top\nUse ~Store~ in [[file:src/a.go][a]].\n** Sub\n[[docs/b.org]]\n",
        );
        let (symbols, refs) = summary(&org);
        assert_eq!(symbols, ["2-5 Top <-", "4-5 Sub <Top"]);
        assert_eq!(
            refs,
            ["3 Path src/a.go", "3 Mention Store", "5 Path docs/b.org"]
        );

        let adoc = extract(
            FileType::AsciiDoc,
            "= Title\n\nUse `Store`.\n\n== Part\ninclude::chapters/one.adoc[]\nimage::img/a.png[Alt]\nsee link:https://x.io[x] and xref:other.adoc[other]\n",
        );
        let (symbols, refs) = summary(&adoc);
        assert_eq!(symbols, ["1-8 Title <-", "5-8 Part <Title"]);
        assert_eq!(
            refs,
            [
                "3 Mention Store",
                "6 Path chapters/one.adoc",
                "7 Path img/a.png",
                "8 Path other.adoc"
            ]
        );

        let tex = extract(
            FileType::Tex,
            "\\section{Intro}\nThe \\texttt{store\\_open} call. % \\input{hidden}\n\\subsection*{Detail}\n\\input{chapters/one}\n\\includegraphics[width=3cm]{fig/plot.pdf}\n",
        );
        let (symbols, refs) = summary(&tex);
        assert_eq!(symbols, ["1-5 Intro <-", "3-5 Detail <Intro"]);
        assert_eq!(
            refs,
            [
                "2 Mention store_open",
                "4 Path chapters/one.tex",
                "5 Path fig/plot.pdf"
            ]
        );
    }

    #[test]
    fn html_headings_links_code_and_text() {
        let text = "<!DOCTYPE html>\n<html><head><title>Store docs</title>\n<link href=\"css/site.css\" rel=\"stylesheet\">\n<script src=\"js/app.js\">var x = '<h1>no</h1>';</script>\n</head><body>\n<h1>Guide</h1>\n<p>Call <code>store.Get</code> &amp; see <a href=\"api.html#x\">api</a>.</p>\n<!-- <h2>hidden</h2> -->\n<h2 id=\"u\">Usage</h2>\n<img src='img/logo.png'>\n<a href=\"https://x.io\">x</a>\n</body></html>\n";
        let facts = extract(FileType::Html, text);
        let (symbols, refs) = summary(&facts);
        assert_eq!(symbols, ["6-12 Guide <-", "9-12 Usage <Guide"]);
        assert_eq!(
            refs,
            [
                "3 Path css/site.css",
                "4 Path js/app.js",
                "7 Mention store.Get",
                "7 Path api.html",
                "10 Path img/logo.png"
            ]
        );
        assert_eq!(facts.doc, "Call store.Get & see api.");
    }

    #[test]
    fn html_variable_spans_are_text_not_global_symbol_names() {
        let facts = html(concat!(
            "<h1>API</h1>\n",
            "<p>Call <code><a href=\"api.html\">fetch</a></code> with <code><i>request</i></code>.</p>\n",
            "<p><code><var>context</var></code> and <code><i><var>session</var></i></code> are parameters.</p>\n",
            "<p><code><i>Store.read</i></code> and <code><i>render</i>()</code> are named APIs.</p>\n",
        ));
        let (_, refs) = summary(&facts);
        assert_eq!(
            refs,
            [
                "2 Path api.html",
                "2 Mention fetch",
                "4 Mention Store.read",
                "4 Mention render"
            ]
        );
        let terms = &facts.symbols[0].terms;
        for name in ["request", "context", "session"] {
            assert!(
                terms.split_whitespace().any(|term| term == name),
                "{name}: {terms}"
            );
        }
    }

    #[test]
    fn outline_text_becomes_section_terms() {
        let mut outline = Outline::with_text(3);
        outline.heading(1, "Intro", 1);
        outline.text("Opening words");
        outline.heading(1, "Next", 2);
        outline.text("Short closing");
        let facts = outline.finish();
        let terms: Vec<(&str, &str)> = facts
            .symbols
            .iter()
            .map(|symbol| (symbol.name.as_ref(), symbol.terms.as_ref()))
            .collect();
        assert_eq!(terms, [("Intro", "open word"), ("Next", "short clos")]);
        let plain = extract(FileType::Text, "First line.\n\nMore text.\n");
        assert_eq!(plain.doc, "First line.");
    }
}
