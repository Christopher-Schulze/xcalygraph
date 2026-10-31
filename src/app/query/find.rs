//! `find`: ranked symbols and files, one line each (`docs/spec.md` section 7.2).

use std::collections::HashMap;

use super::{Filter, Index, Node};
use crate::domain::search::{self, B, FIELD_WEIGHTS, FIELDS, K1};
use crate::infra::store::derived::PostingRecord;

const EXACT_NAME_BONUS: f32 = 5.0;
const RANK_WEIGHT: f32 = 2.0;
/// BM25 scales by the share of query terms a node matches (coordination): a node covering
/// three of five terms in its body beats one matching a single term in its name.
const COORD_EXPONENT: f32 = 1.0;
const DIVERSITY: f32 = 0.4;
/// Hits below this share of the top score are noise.
const RELEVANCE_FLOOR: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub node: u32,
    pub score: f32,
}

/// Scores every candidate of the query; sorted by score, then path, then line.
pub fn search(index: &Index, filter: &Filter, query: &str) -> Vec<Hit> {
    let terms = search::query_terms(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let derived = &index.derived;
    let symbol_count = index.symbol_count();
    let node_count = derived.node_count() as f32;
    let averages = derived.averages();
    let lengths = derived.lengths();
    let postings: Vec<Vec<PostingRecord>> =
        terms.iter().map(|term| derived.postings(term)).collect();
    let mut candidates: Vec<u32> = postings
        .iter()
        .flat_map(|list| list.iter().map(|p| p.node))
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    candidates.retain(|&node| filter.node(index, node));
    let raw = query.trim().to_lowercase().replace([' ', '_', '-'], "");
    let qualified_query =
        (query.contains('.') || query.contains("::")).then(|| query.trim().replace("::", "."));
    let max_log_rank = derived
        .symbols()
        .iter()
        .map(|symbol| (1.0 + symbol.rank * node_count).ln())
        .fold(0.0f32, f32::max)
        .max(f32::EPSILON);
    let mut scores: HashMap<u32, (f32, u32)> = HashMap::with_capacity(candidates.len());
    for list in &postings {
        let df = list.len() as f32;
        let idf = (1.0 + (node_count - df + 0.5) / (df + 0.5)).ln();
        for &node in &candidates {
            let mut tf = [0u8; FIELDS];
            if let Some(found) = posting(list, node) {
                for (field, slot) in tf.iter_mut().enumerate() {
                    *slot = found.tf(field);
                }
            }
            if node < symbol_count {
                // Path field of a symbol: the term frequency in its file's path.
                let file_node = symbol_count + derived.symbols()[node as usize].file;
                if let Some(file_posting) = posting(list, file_node) {
                    tf[search::PATH] = file_posting.tf(search::PATH);
                }
            }
            let length = lengths
                .get(node as usize)
                .map_or([0; FIELDS], |record| record.lengths);
            let mut weighted = 0.0;
            for field in 0..FIELDS {
                if tf[field] == 0 {
                    continue;
                }
                let normalization = 1.0 - B + B * f32::from(length[field]) / averages[field];
                weighted += FIELD_WEIGHTS[field] * f32::from(tf[field]) / normalization.max(0.01);
            }
            if weighted > 0.0 {
                let entry = scores.entry(node).or_default();
                entry.0 += idf * weighted / (K1 + weighted);
                entry.1 += 1;
            }
        }
    }
    let mut hits: Vec<Hit> = scores
        .into_iter()
        .map(|(node, (bm25, matched))| {
            let coverage = matched as f32 / terms.len() as f32;
            let mut score = bm25 * coverage.powf(COORD_EXPONENT);
            let mut rank = 0.0;
            if let Node::Symbol(symbol) = index.node(node) {
                let name = index.string(symbol.name).to_lowercase().replace('_', "");
                // Only the whole query naming the symbol earns the bonus; single terms are already
                // weighted by the name field, and a bonus per term lets common words win.
                if name == raw {
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
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| index.path_of(a.node).cmp(index.path_of(b.node)))
            .then_with(|| index.line_of(a.node).cmp(&index.line_of(b.node)))
    });
    // Diversity: each further hit of a file counts DIVERSITY times the one before, so one file's
    // near-identical siblings (a UI kit's `NavigationMenu*`) cannot fill the first results.
    let mut seen_per_file: HashMap<u32, i32> = HashMap::new();
    for hit in &mut hits {
        let seen = seen_per_file
            .entry(index.file_index_of(hit.node))
            .or_default();
        hit.score *= DIVERSITY.powi(*seen);
        *seen += 1;
    }
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| index.path_of(a.node).cmp(index.path_of(b.node)))
            .then_with(|| index.line_of(a.node).cmp(&index.line_of(b.node)))
    });
    if let Some(top) = hits.first().map(|hit| hit.score) {
        hits.retain(|hit| hit.score >= top * RELEVANCE_FLOOR);
    }
    // A file hit below one of its own symbols repeats a path already shown.
    let mut shown_files = std::collections::HashSet::new();
    hits.retain(|hit| {
        let file = index.file_index_of(hit.node);
        let is_file = hit.node >= symbol_count;
        shown_files.insert(file) || !is_file
    });
    hits
}

fn posting(list: &[PostingRecord], node: u32) -> Option<&PostingRecord> {
    list.binary_search_by_key(&node, |posting| posting.node)
        .ok()
        .map(|found| &list[found])
}

/// Renders the best hits that fit the budget in score order; adjacent hits of one file share
/// its path line, so a weak sibling never pushes a stronger hit of another file down.
pub fn render(index: &Index, hits: &[Hit], json: bool, budget: usize) -> String {
    let nodes: Vec<(u32, Option<f32>, bool)> = hits
        .iter()
        .map(|hit| (hit.node, json.then_some(hit.score), false))
        .collect();
    render_nodes(index, &nodes, json, budget, false)
}

/// Writes the longest prefix of `nodes` (node, score, with doc) whose form grouped under files
/// in order of first appearance fits.
pub fn render_grouped(
    index: &Index,
    nodes: &[(u32, Option<f32>, bool)],
    json: bool,
    budget: usize,
) -> String {
    render_nodes(index, nodes, json, budget, true)
}

/// Writes the longest prefix of `nodes` that fits; `regroup` gathers each file's nodes under its
/// first appearance, otherwise only adjacent nodes of one file share its path line.
fn render_nodes(
    index: &Index,
    nodes: &[(u32, Option<f32>, bool)],
    json: bool,
    budget: usize,
    regroup: bool,
) -> String {
    let write = |writer: &mut super::output::Writer, nodes: &[(u32, Option<f32>, bool)]| {
        if regroup {
            write_groups(index, writer, nodes);
        } else {
            for &(node, score, doc) in nodes {
                index.write_symbol(writer, node, score, doc);
            }
        }
    };
    let fits = |count: usize| {
        let mut writer = super::output::Writer::new(json, budget);
        write(&mut writer, &nodes[..count]);
        !writer.is_full()
    };
    let (mut low, mut high) = (0, nodes.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(middle) {
            low = middle
        } else {
            high = middle - 1
        }
    }
    let count = low.max(usize::from(!nodes.is_empty()));
    let mut writer = super::output::Writer::new(json, budget);
    write(&mut writer, &nodes[..count]);
    writer.skip(nodes.len() - count);
    writer.finish()
}

fn write_groups(
    index: &Index,
    writer: &mut super::output::Writer,
    nodes: &[(u32, Option<f32>, bool)],
) {
    let mut order: Vec<u32> = Vec::new();
    let mut groups: HashMap<u32, Vec<(u32, Option<f32>, bool)>> = HashMap::new();
    for &entry in nodes {
        let file = index.file_index_of(entry.0);
        if !groups.contains_key(&file) {
            order.push(file);
        }
        groups.entry(file).or_default().push(entry);
    }
    for file in order {
        for (node, score, doc) in groups.remove(&file).unwrap_or_default() {
            index.write_symbol(writer, node, score, doc);
        }
    }
}
