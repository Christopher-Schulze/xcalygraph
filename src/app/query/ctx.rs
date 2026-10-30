//! `ctx`: context bundle of seeds plus related symbols by personalized PageRank
//! (`docs/spec.md` section 7.3).

use std::collections::HashMap;

use super::find::{self, Hit};
use super::{Filter, Index, Node};
use crate::domain::graph::EdgeKind;

const SEEDS: usize = 5;
const SEED_FLOOR: f32 = 0.25;
const RESTART: f32 = 0.3;
const ITERATIONS: usize = 15;
/// Mass below this is dropped to keep the walk local.
const EPSILON: f32 = 1e-5;

pub fn render(
    index: &Index,
    filter: &Filter,
    query: &str,
    json: bool,
    budget: usize,
) -> Option<String> {
    let hits = find::search(index, filter, query);
    let top = hits.first()?.score;
    let seeds: Vec<Hit> = hits
        .iter()
        .copied()
        .filter(|hit| hit.score >= top * SEED_FLOOR)
        .take(SEEDS)
        .collect();
    let scores = personalized_rank(index, filter, &seeds);
    let mut related: Vec<(u32, f32)> = scores
        .into_iter()
        .filter(|(node, _)| !seeds.iter().any(|seed| seed.node == *node))
        .filter(|(node, _)| {
            matches!(index.node(*node), Node::Symbol(_)) && filter.node(index, *node)
        })
        .map(|(node, score)| (node, score * index.role_weight(node)))
        .collect();
    related.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut order: Vec<(u32, bool, f32)> = seeds
        .iter()
        .map(|seed| (seed.node, true, seed.score))
        .collect();
    order.extend(
        related
            .into_iter()
            .map(|(node, score)| (node, false, score)),
    );
    let nodes: Vec<(u32, Option<f32>, bool)> = order
        .into_iter()
        .map(|(node, seed, score)| (node, json.then_some(score), seed))
        .collect();
    Some(find::render_grouped(index, &nodes, json, budget))
}

/// Sparse personalized PageRank over the symbol graph in both directions.
fn personalized_rank(index: &Index, filter: &Filter, seeds: &[Hit]) -> HashMap<u32, f32> {
    let mut restart: HashMap<u32, f32> = HashMap::new();
    let total: f32 = seeds
        .iter()
        .map(|seed| seed.score)
        .sum::<f32>()
        .max(f32::EPSILON);
    for seed in seeds {
        *restart.entry(seed.node).or_default() += seed.score / total;
    }
    let mut scores = restart.clone();
    for _ in 0..ITERATIONS {
        let mut next: HashMap<u32, f32> = restart
            .iter()
            .map(|(&node, &mass)| (node, RESTART * mass))
            .collect();
        for (&node, &mass) in &scores {
            let neighbors: Vec<u32> = index
                .derived
                .outgoing(node)
                .iter()
                .chain(index.derived.incoming(node))
                .filter(|edge| !matches!(edge.kind(), EdgeKind::Imports) && filter.edge(edge))
                .map(|edge| edge.node)
                .collect();
            if neighbors.is_empty() {
                *next.entry(node).or_default() += (1.0 - RESTART) * mass;
                continue;
            }
            let share = (1.0 - RESTART) * mass / neighbors.len() as f32;
            if share < EPSILON {
                continue;
            }
            for neighbor in neighbors {
                *next.entry(neighbor).or_default() += share;
            }
        }
        scores = next;
    }
    scores
}
