//! `ctx`: context bundle of seeds plus related symbols by personalized PageRank
//! (`docs/spec.md` section 7.3).

use super::find::{self, Hit};
use super::{Filter, Index, Node};
use crate::domain::graph::EdgeKind;
use crate::domain::hash::FastMap;

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
        .take_while(|hit| hit.score >= top * SEED_FLOOR)
        .take(SEEDS)
        .collect();
    let covered = seeds.iter().fold(0, |mask, hit| mask | hit.name_evidence);
    let extra = hits.iter().find(|hit| {
        hit.name_evidence & !covered != 0 && !seeds.iter().any(|seed| seed.node == hit.node)
    });
    let scores = personalized_rank(index, filter, &seeds);
    let mut related: Vec<(u32, f32)> = scores
        .into_iter()
        .filter(|(node, _)| {
            !seeds
                .iter()
                .any(|seed| index.derived.interface_canonical(seed.node) == *node)
        })
        .filter(|(node, _)| {
            !extra.is_some_and(|hit| index.derived.interface_canonical(hit.node) == *node)
        })
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
    if let Some(hit) = extra {
        order.push((hit.node, false, hit.score));
    }
    order.extend(
        related
            .into_iter()
            .map(|(node, score)| (node, false, score)),
    );
    let nodes: Vec<(u32, Option<f32>, bool)> = order
        .into_iter()
        .map(|(node, seed, score)| (node, json.then_some(score), seed))
        .collect();
    let seed_nodes: Vec<u32> = seeds.iter().map(|seed| seed.node).collect();
    Some(find::render_grouped(
        index,
        filter,
        &nodes,
        &seed_nodes,
        json,
        budget,
    ))
}

/// Sparse personalized PageRank over the symbol graph in both directions.
fn personalized_rank(index: &Index, filter: &Filter, seeds: &[Hit]) -> FastMap<u32, f32> {
    // Node IDs and insertion order are deterministic; the fixed hasher keeps accumulation order
    // stable without sorting every iteration or depending on per-process random hash seeds.
    let mut restart: FastMap<u32, f32> = FastMap::default();
    let total: f32 = seeds
        .iter()
        .map(|seed| seed.score)
        .sum::<f32>()
        .max(f32::EPSILON);
    for seed in seeds {
        *restart
            .entry(index.derived.interface_canonical(seed.node))
            .or_default() += seed.score / total;
    }
    let mut scores = restart.clone();
    for _ in 0..ITERATIONS {
        let mut next: FastMap<u32, f32> = restart
            .iter()
            .map(|(&node, &mass)| (node, RESTART * mass))
            .collect();
        for (&node, &mass) in &scores {
            let neighbors: Vec<u32> = index
                .derived
                .interface_parts(node)
                .flat_map(|part| {
                    index
                        .derived
                        .outgoing(part)
                        .iter()
                        .chain(index.derived.incoming(part))
                })
                .filter(|edge| !matches!(edge.kind(), EdgeKind::Imports) && filter.edge(edge))
                .map(|edge| index.derived.interface_canonical(edge.node))
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
