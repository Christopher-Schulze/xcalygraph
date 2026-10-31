//! SQL facts (`docs/spec.md` sections 5 and 6): tables, views, functions, procedures and
//! triggers from DDL statements (columns as field lists), and `queries` references from SQL
//! statements, both inside `.sql` files and inside string literals of code. Statement scanners
//! instead of a grammar: robust across dialects and without query compilation.

use crate::domain::facts::{self, Facts, Fields, NO_ARITY, Ref, RefKind, SymKind, Symbol};

/// Statement keywords that make a string literal SQL.
const STATEMENT_KEYWORDS: [&str; 6] = ["select", "insert", "update", "delete", "merge", "with"];

/// Words after which a table or view name follows in DML.
const TABLE_KEYWORDS: [&str; 5] = ["from", "join", "into", "update", "table"];

/// DDL objects of a `.sql` file and the tables its views, functions and statements query.
pub fn objects(text: &str, facts: &mut Facts) {
    // `--` comments blanked (offsets and lines kept).
    let text = &blank_comments(text);
    let lower = text.to_ascii_lowercase();
    let line_of = |offset: usize| text[..offset].matches('\n').count() as u32 + 1;
    let mut statement_starts: Vec<(usize, usize)> = Vec::new();
    let mut offset = 0;
    for statement in split_statements(&lower) {
        statement_starts.push((offset, offset + statement.len()));
        offset += statement.len() + 1;
    }
    for &(raw_start, end) in &statement_starts {
        // Leading blank and `--` comment lines belong to no statement.
        let mut start = raw_start;
        loop {
            let rest = &lower[start..end.min(lower.len())];
            let trimmed = rest.trim_start();
            start += rest.len() - trimmed.len();
            if trimmed.starts_with("--") {
                start += trimmed
                    .find('\n')
                    .map_or(trimmed.len(), |newline| newline + 1);
            } else {
                break;
            }
        }
        let statement = &lower[start..end.min(lower.len())];
        let original = &text[start..end.min(text.len())];
        let leading = statement.len() - statement.trim_start().len();
        let line = line_of(start + leading);
        let end_line = line_of(end.min(text.len()).saturating_sub(1).max(start));
        let words: Vec<&str> = statement.split_whitespace().take(12).collect();
        let scope = if words.first() == Some(&"create") {
            let mut index = 1;
            while words.get(index).is_some_and(|word| {
                matches!(
                    *word,
                    "or" | "replace"
                        | "temp"
                        | "temporary"
                        | "unlogged"
                        | "materialized"
                        | "global"
                        | "local"
                        | "definer"
                        | "recursive"
                )
            }) {
                index += 1;
            }
            let kind = match words.get(index).copied() {
                Some("table") => Some(SymKind::Table),
                Some("view") => Some(SymKind::View),
                Some("function" | "procedure" | "trigger") => Some(SymKind::Fn),
                _ => None,
            };
            let mut name_index = index + 1;
            while words
                .get(name_index)
                .is_some_and(|word| matches!(*word, "if" | "not" | "exists"))
            {
                name_index += 1;
            }
            match (kind, words.get(name_index)) {
                (Some(kind), Some(_)) => {
                    let name = object_name(original_word(original, statement, name_index));
                    let signature = facts::clip(
                        &facts::normalize_space(
                            original.trim().split('(').next().unwrap_or_default(),
                        ),
                        160,
                    );
                    facts.symbols.push(Symbol {
                        kind: Some(kind),
                        name: name.clone(),
                        line,
                        end_line,
                        signature,
                        exported: true,
                        ..Symbol::default()
                    });
                    let symbol = (facts.symbols.len() - 1) as u32;
                    if kind == SymKind::Table {
                        let (names, types) = columns(original);
                        if !names.is_empty() {
                            facts.fields.push(Fields {
                                owner: symbol,
                                names,
                                types,
                            });
                        }
                        // Foreign keys: `REFERENCES other (id)`.
                        let mut from = 0;
                        while let Some(found) = statement[from..].find(" references ") {
                            let position = from + found + " references ".len();
                            let word: String = statement[position..]
                                .trim_start()
                                .chars()
                                .take_while(|ch| {
                                    ch.is_ascii_alphanumeric()
                                        || matches!(ch, '_' | '.' | '"' | '`')
                                })
                                .collect();
                            let skipped = statement[position..].len()
                                - statement[position..].trim_start().len();
                            let target = object_name(
                                &original[position + skipped..position + skipped + word.len()],
                            );
                            push_query(facts, Some(symbol), &target, line_of(start + position));
                            from = position;
                        }
                        // A table's own name after `TABLE` is no reference.
                        None
                    } else {
                        Some(symbol)
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        let skip_own = facts.symbols.last().map(|symbol| symbol.name.clone());
        for (name, relative) in table_names(statement) {
            let name = object_name(&original[relative..relative + name.len()]);
            // `CREATE TABLE t`: the table's own name is no query.
            if words.first() == Some(&"create")
                && Some(&name) == skip_own.as_ref()
                && scope.is_none()
            {
                continue;
            }
            push_query(facts, scope, &name, line_of(start + relative));
        }
    }
}

/// `queries` references from SQL statements inside string literals of a code file; the scope is
/// the innermost symbol around the literal.
pub fn embedded_queries(text: &str, facts: &mut Facts) {
    for (offset, full) in string_literals(text) {
        // Leading `-- name: X :many` lines (sqlc and similar generators) precede the statement.
        let skipped: usize = full
            .split_inclusive('\n')
            .take_while(|line| line.trim_start().starts_with("--") || line.trim().is_empty())
            .map(str::len)
            .sum();
        let literal = &full[skipped.min(full.len())..];
        let offset = offset + skipped;
        let lower = literal.to_ascii_lowercase();
        let first = lower.split_whitespace().next().unwrap_or_default();
        if !STATEMENT_KEYWORDS.contains(&first)
            || !TABLE_KEYWORDS
                .iter()
                .any(|keyword| lower.contains(&format!(" {keyword} ")))
        {
            continue;
        }
        // Prose ("select a seat from the menu") is no SQL: an upper-case statement keyword or a
        // further SQL token is required.
        let upper_keyword = literal
            .split_whitespace()
            .next()
            .is_some_and(|word| word.chars().all(|ch| ch.is_ascii_uppercase()));
        let sql_token = [
            " where ",
            " values",
            " set ",
            " join ",
            " order by ",
            " group by ",
            " limit ",
            " returning ",
            "*",
            "=",
            "?",
            "$1",
            " on ",
        ]
        .iter()
        .any(|token| lower.contains(token));
        if !upper_keyword && !sql_token {
            continue;
        }
        let line = text[..offset].matches('\n').count() as u32 + 1;
        let scope = facts
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| symbol.line <= line && line <= symbol.end_line)
            .max_by_key(|(_, symbol)| symbol.line)
            .map(|(index, _)| index as u32);
        for (name, relative) in table_names(&lower) {
            let name = object_name(&literal[relative..relative + name.len()]);
            push_query(
                facts,
                scope,
                &name,
                line + literal[..relative].matches('\n').count() as u32,
            );
        }
    }
}

fn push_query(facts: &mut Facts, scope: Option<u32>, name: &str, line: u32) {
    if name.is_empty() || !name.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_') {
        return;
    }
    let duplicate = facts.refs.iter().any(|reference| {
        reference.kind == RefKind::Query && reference.scope == scope && reference.name == name
    });
    if !duplicate {
        facts.refs.push(Ref {
            kind: RefKind::Query,
            name: name.to_owned(),
            qualifier: None,
            scope,
            line,
            arity: NO_ARITY,
        });
    }
}

/// Last segment of a possibly schema-qualified, quoted name: `public."Users"` -> `Users`.
fn object_name(word: &str) -> String {
    let word = word
        .split('(')
        .next()
        .unwrap_or(word)
        .trim_end_matches([';', ',']);
    word.rsplit('.')
        .next()
        .unwrap_or(word)
        .trim_matches(|ch| matches!(ch, '"' | '`' | '[' | ']'))
        .to_owned()
}

/// The word at `index` of the lowercase statement, in its original spelling.
fn original_word<'a>(original: &'a str, lower: &'a str, index: usize) -> &'a str {
    let mut position = 0;
    for (count, word) in lower.split_whitespace().enumerate() {
        let found = lower[position..]
            .find(word)
            .map_or(position, |relative| position + relative);
        if count == index {
            return original.get(found..found + word.len()).unwrap_or(word);
        }
        position = found + word.len();
    }
    ""
}

/// Names following `FROM`, `JOIN`, `INTO`, `UPDATE` with their byte offset in `lower`.
fn table_names(lower: &str) -> Vec<(String, usize)> {
    let mut names = Vec::new();
    let bytes = lower.as_bytes();
    for keyword in TABLE_KEYWORDS {
        let mut from = 0;
        while let Some(found) = lower[from..].find(keyword) {
            let start = from + found;
            from = start + keyword.len();
            let before_ok = start == 0
                || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
            let after_ok = bytes.get(from).is_some_and(u8::is_ascii_whitespace);
            if !before_ok || !after_ok {
                continue;
            }
            let mut rest = &lower[from..];
            // `TABLE IF [NOT] EXISTS name`.
            for guard in ["if not exists", "if exists"] {
                if let Some(after) = rest.trim_start().strip_prefix(guard) {
                    rest = after;
                }
            }
            let consumed = lower.len() - from - rest.len();
            let skipped = consumed + rest.len() - rest.trim_start().len();
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|ch| {
                    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '"' | '`' | '[' | ']')
                })
                .collect();
            let bare = name.trim_matches(|ch| matches!(ch, '"' | '`' | '[' | ']'));
            if !bare.is_empty()
                && !matches!(
                    bare,
                    "select" | "set" | "values" | "only" | "lateral" | "unnest"
                )
            {
                names.push((name, from + skipped));
            }
        }
    }
    names.sort_by_key(|(_, offset)| *offset);
    names
}

/// Column names and types of `CREATE TABLE name ( col type, ..., CONSTRAINT ... )`.
fn columns(statement: &str) -> (Vec<String>, Vec<String>) {
    let Some(open) = statement.find('(') else {
        return (Vec::new(), Vec::new());
    };
    let mut depth = 0;
    let mut current = String::new();
    let mut parts = Vec::new();
    for ch in statement[open + 1..].chars() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => break,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    parts.push(current);
    let mut names = Vec::new();
    let mut types = Vec::new();
    for part in parts {
        let mut words = part.split_whitespace();
        let (Some(name), Some(kind)) = (words.next(), words.next()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "constraint" | "primary" | "foreign" | "unique" | "check" | "key" | "index" | "exclude"
        ) {
            continue;
        }
        names.push(object_name(name));
        types.push(kind.trim_end_matches(',').to_ascii_lowercase());
    }
    (names, types)
}

/// `--` line comments replaced by spaces outside quotes.
fn blank_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let mut quote: Option<char> = None;
        let mut cut = None;
        let mut previous = ' ';
        for (index, ch) in line.char_indices() {
            match quote {
                Some(open) if ch == open => quote = None,
                Some(_) => {}
                None if ch == '\'' || ch == '"' => quote = Some(ch),
                None if ch == '-' && previous == '-' => {
                    cut = Some(index - 1);
                    break;
                }
                None => {}
            }
            previous = ch;
        }
        match cut {
            Some(cut) => {
                out.push_str(&line[..cut]);
                out.extend(
                    line[cut..]
                        .chars()
                        .map(|ch| if ch == '\n' { '\n' } else { ' ' }),
                );
            }
            None => out.push_str(line),
        }
    }
    out
}

/// Statements split at `;` outside quotes, comments and dollar-quoted bodies.
fn split_statements(lower: &str) -> Vec<&str> {
    let mut statements = Vec::new();
    let bytes = lower.as_bytes();
    let mut start = 0;
    let mut index = 0;
    let mut quote: Option<u8> = None;
    let mut dollar = false;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(open) => {
                if byte == open {
                    quote = None;
                }
            }
            None if lower[index..].starts_with("$$") => {
                dollar = !dollar;
                index += 1;
            }
            None if dollar => {}
            None if lower[index..].starts_with("--") => {
                index = lower[index..]
                    .find('\n')
                    .map_or(bytes.len(), |end| index + end);
                continue;
            }
            None if byte == b'\'' || byte == b'"' => quote = Some(byte),
            None if byte == b';' => {
                statements.push(&lower[start..index]);
                start = index + 1;
            }
            None => {}
        }
        index += 1;
    }
    if start < lower.len() {
        statements.push(&lower[start..]);
    }
    statements
}

/// String literals (`"..."`, `'...'`, `` `...` ``, triple quotes) with the byte offset of their
/// content; escapes honored, lines kept.
fn string_literals(text: &str) -> Vec<(usize, &str)> {
    let mut literals = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !matches!(byte, b'"' | b'\'' | b'`') {
            index += 1;
            continue;
        }
        let triple =
            text[index..].starts_with(&String::from_utf8_lossy(&[byte, byte, byte]).into_owned());
        let delimiter = if triple { 3 } else { 1 };
        let start = index + delimiter;
        let mut end = start;
        let mut found = None;
        while end < bytes.len() {
            if bytes[end] == b'\\' {
                end += 2;
                continue;
            }
            if bytes[end] == byte
                && (!triple
                    || text[end..]
                        .starts_with(&String::from_utf8_lossy(&[byte, byte, byte]).into_owned()))
            {
                found = Some(end);
                break;
            }
            if bytes[end] == b'\n' && byte == b'\'' && !triple {
                break;
            }
            end += 1;
        }
        match found {
            Some(close) => {
                if let Some(content) = text.get(start..close) {
                    literals.push((start, content));
                }
                index = close + delimiter;
            }
            None => index = start,
        }
    }
    literals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(facts: &Facts) -> (Vec<String>, Vec<String>) {
        let symbols = facts
            .symbols
            .iter()
            .map(|symbol| {
                format!(
                    "{}-{} {:?} {}",
                    symbol.line,
                    symbol.end_line,
                    symbol.kind(),
                    symbol.name
                )
            })
            .collect();
        let refs = facts
            .refs
            .iter()
            .map(|reference| {
                let scope = reference.scope.map_or("-".to_owned(), |scope| {
                    facts.symbols[scope as usize].name.clone()
                });
                format!("{} {scope} -> {}", reference.line, reference.name)
            })
            .collect();
        (symbols, refs)
    }

    #[test]
    fn ddl_objects_columns_and_view_queries() {
        let mut facts = Facts::default();
        objects(
            "-- schema; with a comment\nCREATE TABLE IF NOT EXISTS public.\"Users\" (\n  id serial PRIMARY KEY,\n  email text NOT NULL,\n  CONSTRAINT uq UNIQUE (email)\n);\n\nCREATE OR REPLACE VIEW active_users AS\n  SELECT u.id FROM users u JOIN orders o ON o.user_id = u.id;\n\nCREATE FUNCTION touch() RETURNS trigger AS $$\nBEGIN UPDATE audit SET n = n + 1; RETURN NEW; END;\n$$ LANGUAGE plpgsql;\n",
            &mut facts,
        );
        let (symbols, refs) = summary(&facts);
        assert_eq!(
            symbols,
            ["2-6 Table Users", "8-9 View active_users", "11-13 Fn touch"]
        );
        assert_eq!(facts.fields[0].names, ["id", "email"]);
        assert_eq!(facts.fields[0].types, ["serial", "text"]);
        assert_eq!(
            refs,
            [
                "9 active_users -> users",
                "9 active_users -> orders",
                "12 touch -> audit"
            ]
        );
    }

    #[test]
    fn embedded_sql_in_string_literals_of_code() {
        let mut facts = Facts::default();
        facts.symbols.push(Symbol {
            kind: Some(SymKind::Fn),
            name: "load".to_owned(),
            line: 1,
            end_line: 6,
            ..Symbol::default()
        });
        embedded_queries(
            "func load() {\n\tdb.Query(\"SELECT id FROM users WHERE id = $1\")\n\tdb.Exec(`-- name: Audit :exec\n\t\tINSERT INTO audit (id) VALUES (1)`)\n\tlog(\"select a seat from the menu\")\n}\n",
            &mut facts,
        );
        let (_, refs) = summary(&facts);
        assert_eq!(refs, ["2 load -> users", "4 load -> audit"]);
    }
}
