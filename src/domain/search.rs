//! Tokenization and BM25F fields of the search index (`docs/spec.md` sections 6 and 7.2).

/// Indexed fields of a node, in posting order.
/// Ordered by posting frequency: term frequencies are packed 4 bits per field into a varint, so
/// the most common fields take the low bits and most postings encode in one byte.
pub const FIELDS: usize = 7;
pub const BODY: usize = 0;
pub const NAME: usize = 1;
pub const SIGNATURE: usize = 2;
pub const PARENT: usize = 3;
pub const DOC: usize = 4;
pub const PATH: usize = 5;
/// Names of the symbols calling or using this one: a component is described by its users.
pub const USED_BY: usize = 6;
pub const FIELD_WEIGHTS: [f32; FIELDS] = [1.8, 3.0, 0.5, 1.5, 1.0, 1.0, 0.5];
pub const K1: f32 = 1.2;
pub const B: f32 = 0.75;

/// Splits Unicode alphanumeric runs on separators, camelCase/PascalCase and letter-digit
/// boundaries; lowercases. `HTTPServer2` -> `http`, `server`, `2`.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buffer = String::new();
    for_each_token(text, &mut buffer, |token| tokens.push(token.to_owned()));
    tokens
}

/// Calls `emit` with every lowercase token of `text`, reusing `buffer`; ASCII words take a
/// byte-level fast path without per-character allocation. Two-word phrases with a one-word
/// equivalent (`sign in`, `signIn`) also emit that word (`login`).
pub fn for_each_token(text: &str, buffer: &mut String, mut emit: impl FnMut(&str)) {
    let mut previous = "";
    each_token(text, buffer, |token| {
        emit(token);
        if let Some(word) = phrase(previous, token) {
            emit(word);
        }
        previous = match token {
            "sign" => "sign",
            "log" => "log",
            _ => "",
        };
    });
}

/// Calls `emit` with Unicode alphanumeric runs, retaining underscores for source-word deduplication.
/// ASCII bytes are scanned directly; query tokenization splits underscores after this shared pass.
pub fn for_each_word(text: &str, mut emit: impl FnMut(&str)) {
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut in_word = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let character = if byte.is_ascii() {
            None
        } else {
            text[index..].chars().next()
        };
        let is_word = character.map_or_else(
            || byte.is_ascii_alphanumeric() || byte == b'_',
            |ch| ch.is_alphanumeric() || ch == '_',
        );
        if is_word {
            if !in_word {
                start = index;
                in_word = true;
            }
        } else if in_word {
            emit(&text[start..index]);
            in_word = false;
        }
        index += character.map_or(1, char::len_utf8);
    }
    if in_word {
        emit(&text[start..]);
    }
}

/// One-word form of an auth phrase: `sign in`, `log in` -> `login`.
fn phrase(previous: &str, token: &str) -> Option<&'static str> {
    match (previous, token) {
        ("sign" | "log", "in") => Some("login"),
        ("sign" | "log", "out") => Some("logout"),
        ("sign", "up") => Some("register"),
        _ => None,
    }
}

fn each_token(text: &str, buffer: &mut String, mut emit: impl FnMut(&str)) {
    for_each_word(text, |word| {
        for part in word.split('_').filter(|part| !part.is_empty()) {
            each_token_word(part, buffer, &mut emit);
        }
    });
}

fn each_token_word(word: &str, buffer: &mut String, emit: &mut impl FnMut(&str)) {
    if word.is_ascii() {
        let bytes = word.as_bytes();
        let mut start = 0;
        for index in 1..bytes.len() {
            let (previous, current) = (bytes[index - 1], bytes[index]);
            let next_lower = bytes.get(index + 1).is_some_and(u8::is_ascii_lowercase);
            let boundary = (previous.is_ascii_lowercase() && current.is_ascii_uppercase())
                || (previous.is_ascii_uppercase() && current.is_ascii_uppercase() && next_lower)
                || (previous.is_ascii_alphabetic() && current.is_ascii_digit())
                || (previous.is_ascii_digit() && current.is_ascii_alphabetic());
            if boundary {
                emit_lower(&word[start..index], buffer, emit);
                start = index;
            }
        }
        emit_lower(&word[start..], buffer, emit);
        return;
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

/// Maps common code abbreviations, spelled-out words and synonyms to one form, so `dir` matches
/// "directory", "config" matches `cfg` and "remove" matches `delete`; applied to indexed text
/// and queries alike, after [`stem`], so keys are stemmed forms (`delet`, `databas`).
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
        "attr" | "attribut" => "attr",
        "init" | "initializ" | "initialization" => "init",
        "auth" | "authentication" | "authorization" => "auth",
        "db" | "databas" => "db",
        "msg" | "messag" => "message",
        "tmp" | "temporary" => "temp",
        "doc" | "document" | "documentation" => "doc",
        "info" | "information" => "info",
        "spec" | "specification" => "spec",
        "var" | "variabl" => "var",
        "fs" | "filesystem" => "fs",
        "remov" | "delet" | "destroy" => "delete",
        "signin" | "logon" => "login",
        "signout" | "logoff" => "logout",
        "signup" => "register",
        "reload" | "refetch" | "invalidat" => "refresh",
        "navigat" => "redirect",
        // `paging` stems to `pag`.
        "pagination" | "paginat" | "pager" | "pag" => "page",
        _ => return,
    };
    word.clear();
    word.push_str(canonical);
}

/// Light English suffix stripping, applied identically to indexed text and queries:
/// `trees` -> `tree`, `expanded` -> `expand`, `imports` -> `import`, `entries` -> `entry`;
/// a final `e` of words with at least 5 letters goes too, so `delete`, `deleted`, `deletes` and
/// `deleting` share `delet`. Stems keep at least 3 letters; `ss`, `us` and `is` endings stay.
pub fn stem(word: &mut String) {
    if word.len() <= 4 || !word.is_ascii() {
        return;
    }
    strip_suffix(word);
    if word.len() >= 5 && word.ends_with('e') && !word.ends_with("ee") {
        word.pop();
    }
}

fn strip_suffix(word: &mut String) {
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

/// English stopwords dropped from query terms only; stemmed forms (`where` -> `wher`).
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
            | "wher"
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
        let cases: [(&str, &[&str]); 13] = [
            ("NewStore", &["new", "stor"]),
            ("HTTPServer2", &["http", "server", "2"]),
            ("parse_json_files", &["pars", "json", "file"]),
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
            ("Pagination paginated pager", &["page", "page", "page"]),
            (
                "signIn removeUser",
                &["sign", "in", "login", "delete", "user"],
            ),
            (
                "created creates create file files",
                &["creat", "creat", "creat", "file", "file"],
            ),
        ];
        for (input, want) in cases {
            assert_eq!(tokenize(input), want, "{input}");
        }
    }

    #[test]
    fn unicode_word_runs_feed_the_same_query_tokens() {
        let mut words = Vec::new();
        for_each_word("parse_json ÖffneÜberCache 東京", |word| {
            words.push(word.to_owned())
        });
        assert_eq!(words, ["parse_json", "ÖffneÜberCache", "東京"]);
        assert_eq!(
            tokenize("parse_json ÖffneÜberCache 東京"),
            ["pars", "json", "öffne", "über", "cach", "東京"]
        );
    }

    #[test]
    fn query_terms_drop_stopwords_unless_nothing_remains() {
        assert_eq!(
            query_terms("where is the config loaded"),
            ["config", "load"]
        );
        assert_eq!(query_terms("the"), ["the"]);
        assert_eq!(query_terms("store Store"), ["stor"]);
    }
}
