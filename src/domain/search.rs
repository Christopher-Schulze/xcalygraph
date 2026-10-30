//! Tokenization and BM25F fields of the search index (`docs/spec.md` sections 6 and 7.2).

/// Indexed fields of a node, in posting order.
pub const FIELDS: usize = 5;
pub const NAME: usize = 0;
pub const PARENT: usize = 1;
pub const PATH: usize = 2;
pub const SIGNATURE: usize = 3;
pub const DOC: usize = 4;
pub const FIELD_WEIGHTS: [f32; FIELDS] = [3.0, 1.5, 1.0, 0.5, 1.0];
pub const K1: f32 = 1.2;
pub const B: f32 = 0.75;

/// Splits on non-alphanumerics, camelCase and PascalCase boundaries and letter-digit
/// boundaries; lowercases. `HTTPServer2` -> `http`, `server`, `2`.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buffer = String::new();
    for_each_token(text, &mut buffer, |token| tokens.push(token.to_owned()));
    tokens
}

/// Calls `emit` with every lowercase token of `text`, reusing `buffer`; ASCII words take a
/// byte-level fast path without per-character allocation.
pub fn for_each_token(text: &str, buffer: &mut String, mut emit: impl FnMut(&str)) {
    for word in text.split(|ch: char| !ch.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        if word.is_ascii() {
            let bytes = word.as_bytes();
            let mut start = 0;
            for index in 1..bytes.len() {
                let (previous, current) = (bytes[index - 1], bytes[index]);
                let next_lower = bytes.get(index + 1).is_some_and(u8::is_ascii_lowercase);
                let boundary = (previous.is_ascii_lowercase() && current.is_ascii_uppercase())
                    || (previous.is_ascii_uppercase()
                        && current.is_ascii_uppercase()
                        && next_lower)
                    || (previous.is_ascii_alphabetic() && current.is_ascii_digit())
                    || (previous.is_ascii_digit() && current.is_ascii_alphabetic());
                if boundary {
                    emit_lower(&word[start..index], buffer, &mut emit);
                    start = index;
                }
            }
            emit_lower(&word[start..], buffer, &mut emit);
            continue;
        }
        let chars: Vec<(usize, char)> = word.char_indices().collect();
        let mut start = 0;
        for index in 1..chars.len() {
            let (previous, current) = (chars[index - 1].1, chars[index].1);
            let next_lower = chars
                .get(index + 1)
                .is_some_and(|(_, ch)| ch.is_lowercase());
            let boundary = (previous.is_lowercase() && current.is_uppercase())
                || (previous.is_uppercase() && current.is_uppercase() && next_lower)
                || (previous.is_alphabetic() && current.is_ascii_digit())
                || (previous.is_ascii_digit() && current.is_alphabetic());
            if boundary {
                buffer.clear();
                buffer.push_str(&word[chars[start].0..chars[index].0].to_lowercase());
                stem(buffer);
                canonicalize(buffer);
                emit(buffer);
                start = index;
            }
        }
        buffer.clear();
        buffer.push_str(&word[chars[start].0..].to_lowercase());
        stem(buffer);
        canonicalize(buffer);
        emit(buffer);
    }
}

fn emit_lower(part: &str, buffer: &mut String, emit: &mut impl FnMut(&str)) {
    buffer.clear();
    buffer.extend(
        part.bytes()
            .map(|byte| char::from(byte.to_ascii_lowercase())),
    );
    stem(buffer);
    canonicalize(buffer);
    emit(buffer);
}

/// Maps common code abbreviations and their spelled-out words to one form, so `dir` matches
/// "directory" and "config" matches `cfg`; applied to indexed text and queries alike.
pub fn canonicalize(word: &mut String) {
    let canonical = match word.as_str() {
        "directory" | "directory's" => "dir",
        "cfg" | "configuration" | "conf" => "config",
        "repository" => "repo",
        "environment" => "env",
        "err" => "error",
        "ctx" => "context",
        "idx" => "index",
        "buf" => "buffer",
        "str" => "string",
        "num" => "number",
        "len" => "length",
        "cnt" => "count",
        "pkg" => "package",
        "impl" | "implementation" => "implement",
        "param" | "parameter" => "param",
        "arg" | "argument" => "arg",
        "attr" | "attribute" => "attr",
        "init" | "initialize" | "initialization" => "init",
        "auth" | "authentication" | "authorization" => "auth",
        "db" | "database" => "db",
        "msg" | "message" => "message",
        "tmp" | "temporary" => "temp",
        "doc" | "document" | "documentation" => "doc",
        "info" | "information" => "info",
        "spec" | "specification" => "spec",
        "var" | "variable" => "var",
        "fs" | "filesystem" => "fs",
        _ => return,
    };
    word.clear();
    word.push_str(canonical);
}

/// Light English suffix stripping, applied identically to indexed text and queries:
/// `trees` -> `tree`, `expanded` -> `expand`, `imports` -> `import`, `entries` -> `entry`.
/// Stems keep at least 3 letters; `ss`, `us` and `is` endings stay.
pub fn stem(word: &mut String) {
    if word.len() <= 4 || !word.is_ascii() {
        return;
    }
    let strip = |word: &mut String, suffix: &str, replacement: &str| -> bool {
        if word.ends_with(suffix) && word.len() - suffix.len() >= 3 {
            word.truncate(word.len() - suffix.len());
            word.push_str(replacement);
            return true;
        }
        false
    };
    if strip(word, "ies", "y") || strip(word, "sses", "ss") {
        return;
    }
    if word.ends_with("ss") || word.ends_with("us") || word.ends_with("is") {
        return;
    }
    for suffix in ["ches", "shes", "xes", "zes"] {
        if word.ends_with(suffix) {
            word.truncate(word.len() - 2);
            return;
        }
    }
    if strip(word, "ing", "") || strip(word, "ed", "") {
        return;
    }
    strip(word, "s", "");
}

/// English stopwords dropped from query terms only.
pub fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "by"
            | "does"
            | "do"
            | "for"
            | "from"
            | "how"
            | "in"
            | "is"
            | "it"
            | "of"
            | "on"
            | "or"
            | "the"
            | "that"
            | "this"
            | "to"
            | "what"
            | "where"
            | "which"
            | "who"
            | "why"
            | "with"
            | "when"
    )
}

/// Query terms: tokens without stopwords, deduplicated in order; all tokens when every token is
/// a stopword.
pub fn query_terms(query: &str) -> Vec<String> {
    let all = tokenize(query);
    let mut terms: Vec<String> = Vec::new();
    for token in all.iter().filter(|token| !is_stopword(token)) {
        if !terms.contains(token) {
            terms.push(token.clone());
        }
    }
    if terms.is_empty() {
        for token in all {
            if !terms.contains(&token) {
                terms.push(token);
            }
        }
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_split_case_digits_and_separators() {
        let cases: [(&str, &[&str]); 10] = [
            ("NewStore", &["new", "store"]),
            ("HTTPServer2", &["http", "server", "2"]),
            ("parse_json_files", &["parse", "json", "file"]),
            ("expanded use trees", &["expand", "use", "tree"]),
            ("entries process status", &["entry", "process", "status"]),
            ("classes matches boxes", &["class", "match", "box"]),
            (
                "index directories cfgDir",
                &["index", "dir", "config", "dir"],
            ),
            ("src/infra/scan.rs", &["src", "infra", "scan", "rs"]),
            ("getV2Config", &["get", "v", "2", "config"]),
            ("ÜberCase", &["über", "case"]),
        ];
        for (input, want) in cases {
            assert_eq!(tokenize(input), want, "{input}");
        }
    }

    #[test]
    fn query_terms_drop_stopwords_unless_nothing_remains() {
        assert_eq!(
            query_terms("where is the config loaded"),
            ["config", "load"]
        );
        assert_eq!(query_terms("the"), ["the"]);
        assert_eq!(query_terms("store Store"), ["store"]);
    }
}
