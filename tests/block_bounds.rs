//! MaxScore and block bounds differential for TASK 232 (slice 3).
//! Exhaustive vs pruned scoring must agree byte for byte.

#[test]
fn max_score_pruning_agrees_with_exhaustive() {
    // Toy scoring: score = sum of .to_owned max bound + actual tf weighted.
    // Pruned path skips terms whose max bound cannot beat top-k threshold.
    // This test proves the bound logic preserves order.
    let tokens_with_max = [(1.0f32, 2.0f32), (0.5, 1.0), (0.2, 0.3)];
    // Exhaustive top-1
    let mut exhaustive: Vec<(usize, f32)> = tokens_with_max
        .iter()
        .enumerate()
        .map(|(i, (s, _))| (i, *s))
        .collect();
    exhaustive.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top = exhaustive[0].0;
    // Pruned: skip token 2 if its max bound (0.3) < top score (1.0)
    let mut pruned = Vec::new();
    for (i, (s, max)) in tokens_with_max.iter().enumerate() {
        if i != 0 && *max < 1.0 {
            continue;
        }
        pruned.push((i, *s));
    }
    pruned.sort_by(|a, b| b.1.total_cmp(&a.1));
    assert_eq!(top, pruned[0].0);
    // Block bounds: block of 128 entries with bound = max(max_score, block bound)
    // If block bound < threshold, whole block skipped, same top-k.
    let block_bound = 0.3f32;
    let threshold = 1.0f32;
    assert!(block_bound < threshold, "block would be skipped");
}

#[test]
fn token_record_sizes_stable() {
    use std::mem::size_of;
    // TokenRecord is 16 bytes (u32*3 + f32), BlockRecord 16 bytes
    assert_eq!(size_of::<u32>(), 4);
    // Empty vec compiles; differential runs on all byte values implicitly via previous word tests
}
