//! Derived data: rank, module graph, cycles and layers (`docs/spec.md` section 6, derived items).

use super::modules::ModuleMap;
use crate::domain::facts::{FLAG_GENERATED, FLAG_TEST};
use crate::domain::graph::{Confidence, EdgeKind, Graph, Module, ModuleEdge};

const DAMPING: f32 = 0.85;
const ITERATIONS: usize = 30;

pub fn finish(graph: &mut Graph, modules: &ModuleMap) {
    graph.modules = modules
        .modules
        .iter()
        .map(|module| Module {
            name: module.name.clone(),
            files: 0,
            symbols: 0,
            layer: 0,
            cycle: 0,
        })
        .collect();
    for file in &graph.files {
        let module = &mut graph.modules[file.module as usize];
        module.files += 1;
        module.symbols += file.symbol_count;
    }
    let components = strongly_connected(graph.modules.len(), &graph.module_edges);
    for (module, component) in components.iter().enumerate() {
        graph.modules[module].cycle = *component;
    }
    let layers = layers(graph.modules.len(), &graph.module_edges, &components);
    for (module, layer) in layers.into_iter().enumerate() {
        graph.modules[module].layer = layer;
    }
    rank(graph);
}

/// Tarjan's algorithm, iterative; returns a component id per node.
pub fn strongly_connected(count: usize, edges: &[ModuleEdge]) -> Vec<u32> {
    let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); count];
    for edge in edges {
        adjacency[edge.source as usize].push(edge.target);
    }
    let mut index = vec![u32::MAX; count];
    let mut low = vec![0u32; count];
    let mut on_stack = vec![false; count];
    let mut component = vec![u32::MAX; count];
    let mut stack: Vec<u32> = Vec::new();
    let mut next_index = 0u32;
    let mut next_component = 0u32;
    for start in 0..count {
        if index[start] != u32::MAX {
            continue;
        }
        let mut work: Vec<(u32, usize)> = vec![(start as u32, 0)];
        while let Some(&mut (node, ref mut child)) = work.last_mut() {
            let node_usize = node as usize;
            if *child == 0 && index[node_usize] == u32::MAX {
                index[node_usize] = next_index;
                low[node_usize] = next_index;
                next_index += 1;
                stack.push(node);
                on_stack[node_usize] = true;
            }
            if let Some(&next) = adjacency[node_usize].get(*child) {
                *child += 1;
                let next_usize = next as usize;
                if index[next_usize] == u32::MAX {
                    work.push((next, 0));
                } else if on_stack[next_usize] {
                    low[node_usize] = low[node_usize].min(index[next_usize]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent as usize] = low[parent as usize].min(low[node_usize]);
            }
            if low[node_usize] == index[node_usize] {
                while let Some(member) = stack.pop() {
                    on_stack[member as usize] = false;
                    component[member as usize] = next_component;
                    if member == node {
                        break;
                    }
                }
                next_component += 1;
            }
        }
    }
    component
}

/// Longest path to a sink on the condensed DAG; modules without dependencies are layer 0.
fn layers(count: usize, edges: &[ModuleEdge], components: &[u32]) -> Vec<u32> {
    let component_count = components
        .iter()
        .map(|&c| c as usize + 1)
        .max()
        .unwrap_or(0);
    let mut successors: Vec<Vec<u32>> = vec![Vec::new(); component_count];
    for edge in edges {
        let (source, target) = (
            components[edge.source as usize],
            components[edge.target as usize],
        );
        if source != target {
            successors[source as usize].push(target);
        }
    }
    // Tarjan numbers components in reverse topological order: successors have smaller ids.
    let mut depth = vec![0u32; component_count];
    for component in 0..component_count {
        depth[component] = successors[component]
            .iter()
            .map(|&next| depth[next as usize] + 1)
            .max()
            .unwrap_or(0);
    }
    (0..count)
        .map(|module| depth[components[module] as usize])
        .collect()
}

/// PageRank over symbol edges; test and generated symbols take no part.
fn rank(graph: &mut Graph) {
    let count = graph.symbols.len();
    if count == 0 {
        return;
    }
    let symbols = count as u32;
    let active: Vec<bool> = graph
        .symbols
        .iter()
        .map(|symbol| symbol.flags & (FLAG_TEST | FLAG_GENERATED) == 0)
        .collect();
    let mut out_weight = vec![0f32; count];
    let mut links: Vec<(u32, u32, f32)> = Vec::new();
    for edge in &graph.edges {
        if edge.source >= symbols
            || edge.target >= symbols
            || !active[edge.source as usize]
            || !active[edge.target as usize]
            || matches!(
                edge.kind,
                EdgeKind::Tests | EdgeKind::Imports | EdgeKind::Mentions
            )
        {
            continue;
        }
        let weight = if edge.confidence == Confidence::Exact {
            1.0
        } else {
            0.5
        };
        out_weight[edge.source as usize] += weight;
        links.push((edge.source, edge.target, weight));
    }
    let active_count = active.iter().filter(|&&a| a).count().max(1) as f32;
    let mut scores: Vec<f32> = active
        .iter()
        .map(|&a| if a { 1.0 / active_count } else { 0.0 })
        .collect();
    for _ in 0..ITERATIONS {
        let dangling: f32 = scores
            .iter()
            .enumerate()
            .filter(|&(node, _)| active[node] && out_weight[node] == 0.0)
            .map(|(_, score)| score)
            .sum();
        let base = (1.0 - DAMPING + DAMPING * dangling) / active_count;
        let mut next: Vec<f32> = active.iter().map(|&a| if a { base } else { 0.0 }).collect();
        for &(source, target, weight) in &links {
            next[target as usize] +=
                DAMPING * scores[source as usize] * weight / out_weight[source as usize];
        }
        scores = next;
    }
    for (symbol, score) in graph.symbols.iter_mut().zip(scores) {
        symbol.rank = score;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(source: u32, target: u32) -> ModuleEdge {
        ModuleEdge {
            source,
            target,
            count: 1,
        }
    }

    #[test]
    fn components_and_layers_of_a_module_graph() {
        // 0 -> 1 -> 2 -> 1 (cycle 1-2), 0 -> 3, 3 -> 2
        let edges = [edge(0, 1), edge(1, 2), edge(2, 1), edge(0, 3), edge(3, 2)];
        let components = strongly_connected(4, &edges);
        assert_eq!(components[1], components[2]);
        assert_ne!(components[0], components[1]);
        assert_ne!(components[3], components[1]);
        let layers = layers(4, &edges, &components);
        assert_eq!(layers, [2, 0, 0, 1]);
    }
}
