//! `find`: ranked symbols and files, one line each (`docs/spec.md` section 7.2).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::output::{Value, Writer};
use super::{Filter, Index, Node, path, show};
use crate::domain::filetype::FileType;
use crate::domain::search::{self, B, FIELD_WEIGHTS, FIELDS, K1};
use crate::infra::store::derived::PostingRecord;

const EXACT_NAME_BONUS: f32 = 5.0;
const RANK_WEIGHT: f32 = 2.0;
/// BM25 scales by the share of query terms a node matches (coordination): a node covering
/// three of five terms in its body beats one matching a single term in its name.
const COORD_EXPONENT: f32 = 1.0;
const EXPANSION_WEIGHT: f32 = 0.5;
/// Hits below this share of the top score are noise.
const RELEVANCE_FLOOR: f32 = 0.05;
/// Source lines of the first answer and the width they are clipped to.
const EXCERPT_LINES: usize = 8;
const EXCERPT_WIDTH: usize = 100;
/// The top hit gets its source only when it leads the runner-up by this factor: then it is right
/// in about 78 % of the sealed questions, below it in about 38 %, and the list serves better.
const DOMINANCE: f32 = 1.3;
/// Leading hits searched for connections, and the connections shown.
const CHAIN_HITS: usize = 6;
const MAX_CHAINS: usize = 2;
/// Callers and callees of the first answer, each.
const NEIGHBOURS: usize = 3;
/// Tests of the first answer, with one command to run them.
const TESTS: usize = 2;
const CARD_INDENT: usize = 2;
/// Hits of a text answer with their full signature.
const DETAILED_HITS: usize = 2;
/// Environment star lines per answer.
const MAX_STARS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub node: u32,
    pub score: f32,
}

/// Scores every candidate of the query; sorted by score, then path, then line.
pub fn search(index: &Index, filter: &Filter, query: &str) -> Vec<Hit> {
    let Some(name) = exact_symbol_name(query) else {
        return search_terms(index, filter, query);
    };
    let exact: HashSet<u32> = super::names::lookup(index, filter, name)
        .into_iter()
        .filter(|&node| matches!(index.node(node), Node::Symbol(_)))
        .map(|node| index.derived.interface_canonical(node))
        .collect();
    if exact.is_empty() {
        return Vec::new();
    }
    let mut hits = search_terms(index, filter, name);
    hits.retain(|hit| exact.contains(&index.derived.interface_canonical(hit.node)));
    hits
}

fn exact_symbol_name(query: &str) -> Option<&str> {
    let mut words = query.split_whitespace().peekable();
    while let Some(word) = words.next() {
        if intent_word(word).eq_ignore_ascii_case("exact")
            && words
                .peek()
                .is_some_and(|next| intent_word(next).eq_ignore_ascii_case("symbol"))
        {
            words.next();
            return words
                .next()
                .map(exact_name_token)
                .filter(|name| !name.is_empty());
        }
    }
    None
}

fn intent_word(word: &str) -> &str {
    word.trim_matches(|character: char| !character.is_alphanumeric() && character != '_')
}

fn exact_name_token(word: &str) -> &str {
    word.trim_matches(|character: char| {
        matches!(
            character,
            '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '?' | '!'
        )
    })
    .trim_end_matches(['.', ',', ';', ':', '?', '!'])
}

fn search_terms(index: &Index, filter: &Filter, query: &str) -> Vec<Hit> {
    let terms = search::query_terms(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let derived = &index.derived;
    let symbol_count = index.symbol_count();
    let node_count = derived.node_count() as f32;
    let averages = derived.averages();
    let lengths = derived.lengths();
    let postings: Vec<(Vec<PostingRecord>, Vec<PostingRecord>)> = terms
        .iter()
        .map(|term| {
            let expanded = expanded_term(term)
                .map(|word| derived.postings(&word))
                .unwrap_or_default();
            (derived.postings(term), expanded)
        })
        .collect();
    let mut candidates: Vec<u32> = postings
        .iter()
        .flat_map(|(exact, expanded)| exact.iter().chain(expanded).map(|p| p.node))
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    candidates.retain(|&node| filter.node(index, node));
    let raw = query.trim().to_lowercase().replace([' ', '_', '-'], "");
    let ascii_query = raw.is_ascii();
    let qualified_query =
        (query.contains('.') || query.contains("::")).then(|| query.trim().replace("::", "."));
    let max_log_rank = derived
        .symbols()
        .iter()
        .map(|symbol| (1.0 + symbol.rank * node_count).ln())
        .fold(0.0f32, f32::max)
        .max(f32::EPSILON);
    let mut scores = vec![(0.0f32, 0u32); candidates.len()];
    for (exact, expanded) in &postings {
        let idf = |list: &[PostingRecord]| {
            let df = list.len() as f32;
            (1.0 + (node_count - df + 0.5) / (df + 0.5)).ln()
        };
        let alternatives = [
            (exact, idf(exact), 1.0),
            (expanded, idf(expanded), EXPANSION_WEIGHT),
        ];
        let mut cursors = [0usize; 2];
        let mut cached_files = [None; 2];
        let mut cached_paths = [0u8; 2];
        // Candidates and posting lists are both in node order, so each list is traversed once.
        for (&node, entry) in candidates.iter().zip(&mut scores) {
            let length = lengths
                .get(node as usize)
                .map_or([0; FIELDS], |record| record.lengths);
            let mut best = 0.0f32;
            for (alternative, (list, idf, strength)) in alternatives.iter().copied().enumerate() {
                if list.is_empty() {
                    continue;
                }
                let mut tf = [0u8; FIELDS];
                while cursors[alternative] < list.len() && list[cursors[alternative]].node < node {
                    cursors[alternative] += 1;
                }
                if let Some(found) = list
                    .get(cursors[alternative])
                    .filter(|found| found.node == node)
                {
                    for (field, slot) in tf.iter_mut().enumerate() {
                        *slot = found.tf(field);
                    }
                }
                if node < symbol_count {
                    let file = derived.symbols()[node as usize].file;
                    // Symbols inherit path evidence; adjacent candidates often share a file.
                    if cached_files[alternative] != Some(file) {
                        let file_node = symbol_count + file;
                        cached_files[alternative] = Some(file);
                        cached_paths[alternative] = posting(list, file_node)
                            .map_or(0, |file_posting| file_posting.tf(search::PATH));
                    }
                    tf[search::PATH] = cached_paths[alternative];
                }
                let mut weighted = 0.0;
                for field in 0..FIELDS {
                    if tf[field] == 0 {
                        continue;
                    }
                    let normalization = 1.0 - B + B * f32::from(length[field]) / averages[field];
                    weighted +=
                        FIELD_WEIGHTS[field] * f32::from(tf[field]) / normalization.max(0.01);
                }
                best = best.max(strength * idf * weighted / (K1 + weighted));
            }
            if best > 0.0 {
                entry.0 += best;
                entry.1 += 1;
            }
        }
    }
    let mut hits: Vec<Hit> = candidates
        .into_iter()
        .zip(scores)
        .filter(|(_, score)| score.1 > 0)
        .map(|(node, (bm25, matched))| {
            let coverage = matched as f32 / terms.len() as f32;
            let mut score = bm25 * coverage.powf(COORD_EXPONENT);
            let mut rank = 0.0;
            if let Node::Symbol(symbol) = index.node(node) {
                let name = index.string(symbol.name);
                let exact_name = if ascii_query && name.is_ascii() {
                    name.bytes()
                        .filter(|&byte| byte != b'_')
                        .map(|byte| byte.to_ascii_lowercase())
                        .eq(raw.bytes())
                } else {
                    name.to_lowercase().replace('_', "") == raw
                };
                // Only the whole query naming the symbol earns the bonus; single terms are already
                // weighted by the name field, and a bonus per term lets common words win.
                if exact_name {
                    score += EXACT_NAME_BONUS;
                }
                if let Some(qualified) = &qualified_query
                    && index
                        .qualified(symbol)
                        .replace("::", ".")
                        .ends_with(qualified.as_str())
                {
                    score += EXACT_NAME_BONUS;
                }
                rank = (1.0 + symbol.rank * node_count).ln() / max_log_rank;
            }
            Hit {
                node,
                score: score * index.role_weight(node) + RANK_WEIGHT * rank,
            }
        })
        .collect();
    if !derived.interface_groups().is_empty() {
        let mut positions: HashMap<u32, usize> = HashMap::new();
        let mut distinct: Vec<Hit> = Vec::with_capacity(hits.len());
        for entry in hits {
            let node = entry.node;
            if derived
                .interface_groups()
                .binary_search_by_key(&node, |pair| pair[0])
                .is_err()
            {
                distinct.push(entry);
                continue;
            }
            let canonical = derived.interface_canonical(node);
            if let Some(&position) = positions.get(&canonical) {
                if entry.score > distinct[position].score {
                    distinct[position] = entry;
                }
            } else {
                positions.insert(canonical, distinct.len());
                distinct.push(entry);
            }
        }
        hits = distinct;
    }
    // The final score floor does not depend on order; discard noise before sorting it.
    if let Some(top) = hits.iter().map(|hit| hit.score).max_by(f32::total_cmp) {
        hits.retain(|hit| hit.score >= top * RELEVANCE_FLOOR);
    }
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| index.path_of(a.node).cmp(index.path_of(b.node)))
            .then_with(|| index.line_of(a.node).cmp(&index.line_of(b.node)))
            .then_with(|| a.node.cmp(&b.node))
    });
    // A file hit below one of its own symbols repeats a path already shown.
    let mut shown_files = std::collections::HashSet::new();
    hits.retain(|hit| {
        let file = index.file_index_of(hit.node);
        let is_file = hit.node >= symbol_count;
        shown_files.insert(file) || !is_file
    });
    hits
}

/// Additional query evidence is weaker than the original term and never changes indexed terms.
fn expanded_term(term: &str) -> Option<String> {
    if matches!(term, "retriev" | "retrieval") {
        return Some("get".to_owned());
    }
    if let Some(root) = term.strip_suffix("ization").filter(|root| root.len() >= 3) {
        return Some(format!("{root}iz"));
    }
    if term.ends_with("ation") && term.len() >= 8 {
        return term.strip_suffix("ion").map(str::to_owned);
    }
    (term.len() >= 4 && term.is_ascii()).then(|| format!("{term}iz"))
}

fn posting(list: &[PostingRecord], node: u32) -> Option<&PostingRecord> {
    list.binary_search_by_key(&node, |posting| posting.node)
        .ok()
        .map(|found| &list[found])
}

/// Renders the best hits that fit the budget in score order; adjacent hits of one file share
/// its path line, so a weak sibling never pushes a stronger hit of another file down.
pub fn render(index: &Index, hits: &[Hit], json: bool, budget: usize) -> String {
    let write = |writer: &mut Writer, count: usize, detailed: usize| {
        for (position, hit) in hits[..count].iter().enumerate() {
            index.write_hit(
                writer,
                hit.node,
                json.then_some(hit.score),
                position < detailed,
            );
        }
    };
    let fits = |count: usize| {
        let mut writer = Writer::new(json, budget);
        write(&mut writer, count, detailed_hits(json));
        !writer.is_full()
    };
    let (mut low, mut high) = (0, hits.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(middle) {
            low = middle
        } else {
            high = middle - 1
        }
    }
    let count = low.max(usize::from(!hits.is_empty()));
    let mut writer = Writer::new(json, budget);
    write(&mut writer, count, detailed_hits(json));
    writer.skip(hits.len() - count);
    writer.finish()
}

/// One `~` line per environment variable the query names: where it is read, the test reaching the
/// reader and a matching document heading, each with its site; `-doc` and `-t` mark no match.
fn env_stars(index: &Index, filter: &Filter, query: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for word in query.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')) {
        let named = word.len() >= 3
            && word
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if !named || lines.len() >= MAX_STARS {
            continue;
        }
        let reads: Vec<_> = index
            .derived
            .envs()
            .iter()
            .filter(|env| {
                env.source == 0
                    && index.string(env.name) == word
                    && filter.includes(index, env.node)
            })
            .collect();
        let Some(read) = reads.first() else {
            continue;
        };
        let mut line = format!("~ {word} <read {}:{}", index.path_of(read.node), read.line);
        match super::show::tests_of(index, filter, read.node).first() {
            Some(test) => line.push_str(&format!(
                " <t{} {}:{}",
                if test.2 { "" } else { "?" },
                index.path_of(test.0.node),
                index.line_of(test.0.node)
            )),
            None => line.push_str(" -t"),
        }
        let document = index
            .derived
            .symbols()
            .iter()
            .enumerate()
            .find(|(_, symbol)| {
                !filter.exact
                    && symbol.kind() == crate::domain::facts::SymKind::Heading
                    && filter.symbol(index, symbol)
                    && crate::domain::filetype::contains_word(index.string(symbol.name), word)
            });
        match document {
            Some((node, symbol)) => line.push_str(&format!(
                " <doc? {}:{}",
                index.path_of(node as u32),
                symbol.line
            )),
            None => line.push_str(" -doc"),
        }
        lines.push(line);
    }
    lines
}

/// Hits with their full signature; later ones print `line name`, and the budget this frees shows
/// further hits: more required facts per answer at the same tokens. JSON keeps its fields.
fn detailed_hits(json: bool) -> usize {
    if json { usize::MAX } else { DETAILED_HITS }
}

/// [`render`] without a clear top hit: the edges joining the leading hits come first, because
/// they answer how the parts of a question connect.
fn render_connected(
    index: &Index,
    filter: &Filter,
    hits: &[Hit],
    json: bool,
    budget: usize,
) -> String {
    let leading: Vec<u32> = hits.iter().take(CHAIN_HITS).map(|hit| hit.node).collect();
    let chains = path::chains(index, filter, &leading, MAX_CHAINS);
    let write = |writer: &mut Writer, count: usize, detailed: usize| {
        for (start, links) in &chains {
            path::write_chain(index, writer, *start, links);
        }
        for (position, hit) in hits[..count].iter().enumerate() {
            index.write_hit(
                writer,
                hit.node,
                json.then_some(hit.score),
                position < detailed,
            );
        }
    };
    let fits = |count: usize| {
        let mut writer = Writer::new(json, budget);
        write(&mut writer, count, detailed_hits(json));
        !writer.is_full()
    };
    let (mut low, mut high) = (0, hits.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(middle) {
            low = middle
        } else {
            high = middle - 1
        }
    }
    let mut writer = Writer::new(json, budget);
    write(&mut writer, low, detailed_hits(json));
    writer.skip(hits.len() - low);
    writer.finish()
}

/// [`render`] with the first answer: the top hit's source and neighbours follow its line, so
/// an agent can conclude without a second call (about 20,000 tokens of re-sent context).
pub fn render_answer(
    index: &Index,
    filter: &Filter,
    root: &Path,
    hits: &[Hit],
    query: &str,
    json: bool,
    budget: usize,
) -> String {
    let stars = if json {
        Vec::new()
    } else {
        env_stars(index, filter, query)
    };
    if !stars.is_empty() {
        let mut writer = Writer::new(false, budget);
        for star in &stars {
            writer.line(star, &[]);
        }
        if writer.is_full() {
            writer.skip(hits.len());
            return writer.finish();
        }
        let mut answer = writer.finish();
        let reserved = answer.len().div_ceil(super::output::BYTES_PER_TOKEN);
        let rest = render_answer_body(
            index,
            filter,
            root,
            hits,
            query,
            json,
            budget.saturating_sub(reserved),
        );
        answer.push_str(&rest);
        return answer;
    }
    render_answer_body(index, filter, root, hits, query, json, budget)
}

fn render_answer_body(
    index: &Index,
    filter: &Filter,
    root: &Path,
    hits: &[Hit],
    query: &str,
    json: bool,
    budget: usize,
) -> String {
    let Some(first) = hits.first() else {
        return render(index, hits, json, budget);
    };
    // A single hit leads by definition.
    if hits
        .get(1)
        .is_some_and(|second| first.score < second.score * DOMINANCE)
    {
        return render_connected(index, filter, hits, json, budget);
    }
    let lines = excerpt(index, root, first.node, query);
    let write = |writer: &mut Writer, count: usize, detailed: usize| {
        for (position, hit) in hits[..count].iter().enumerate() {
            index.write_hit(
                writer,
                hit.node,
                json.then_some(hit.score),
                position < detailed,
            );
            if position == 0 {
                for (line, text) in &lines {
                    let shown = match line {
                        Some(line) => format!("    {line} {text}"),
                        None => format!("    {text}"),
                    };
                    writer.line(
                        &shown,
                        &[
                            ("t", Value::Str("source")),
                            ("line", Value::Int(u64::from(line.unwrap_or(0)))),
                            ("text", Value::Str(text)),
                        ],
                    );
                }
                show::neighbours(index, filter, writer, hit.node, NEIGHBOURS);
                show::verification(index, filter, writer, hit.node, TESTS, CARD_INDENT);
            }
        }
    };
    let fits = |count: usize| {
        let mut writer = Writer::new(json, budget);
        write(&mut writer, count, detailed_hits(json));
        !writer.is_full()
    };
    let (mut low, mut high) = (1, hits.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(middle) {
            low = middle
        } else {
            high = middle - 1
        }
    }
    let mut writer = Writer::new(json, budget);
    write(&mut writer, low, detailed_hits(json));
    writer.skip(hits.len() - low);
    writer.finish()
}

/// Lines of a symbol's body worth reading: all of a short one, otherwise its start and the lines
/// naming a query word; the last entry counts what stayed out.
fn excerpt(index: &Index, root: &Path, node: u32, query: &str) -> Vec<(Option<u32>, String)> {
    let Node::Symbol(symbol) = index.node(node) else {
        return Vec::new();
    };
    // Lines of a PDF or an office document are pages, paragraphs or sheets of a binary file.
    let binary = index
        .file_of(node)
        .file_type()
        .is_some_and(FileType::is_binary_document);
    if binary || symbol.end_line <= symbol.line {
        return Vec::new();
    }
    let Ok(content) = std::fs::read(root.join(index.path_of(node))) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&content);
    let words: Vec<String> = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.len() >= 3)
        .map(str::to_lowercase)
        .collect();
    let body: Vec<(u32, &str)> = text
        .lines()
        .enumerate()
        .map(|(index, line)| (index as u32 + 1, line))
        .skip(symbol.line as usize)
        .take((symbol.end_line - symbol.line) as usize)
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();
    let chosen: Vec<&(u32, &str)> = if body.len() <= EXCERPT_LINES {
        body.iter().collect()
    } else {
        let start = EXCERPT_LINES / 2;
        let named = body[start..].iter().filter(|(_, line)| {
            let line = line.to_lowercase();
            words.iter().any(|word| line.contains(word.as_str()))
        });
        body[..start]
            .iter()
            .chain(named.take(EXCERPT_LINES - start))
            .collect()
    };
    let mut lines: Vec<(Option<u32>, String)> = chosen
        .iter()
        .map(|(line, code)| (Some(*line), clip(code.trim_end())))
        .collect();
    if body.len() > chosen.len() {
        lines.push((None, format!("+{} lines", body.len() - chosen.len())));
    }
    lines
}

fn clip(line: &str) -> String {
    if line.chars().count() <= EXCERPT_WIDTH {
        return line.to_owned();
    }
    let mut clipped: String = line.chars().take(EXCERPT_WIDTH).collect();
    clipped.push('…');
    clipped
}
