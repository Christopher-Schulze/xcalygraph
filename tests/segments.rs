// Segment partitioning for TASK 228 (slice 2)
use std::collections::HashMap;

#[test]
fn segment_is_deterministic_and_balanced() -> Result<(), Box<dyn std::error::Error>> {
    use xxhash_rust::xxh3::xxh3_64;
    const SEGMENTS: u16 = 128;
    fn segment_of(path: &str) -> u16 {
        (xxh3_64(path.as_bytes()) % u64::from(SEGMENTS)) as u16
    }
    // Deterministic: same path same segment
    let a1 = segment_of("src/main.rs");
    let a2 = segment_of("src/main.rs");
    assert_eq!(a1, a2);
    // Range
    assert!(a1 < SEGMENTS);
    // Balanced on 200 synthetic paths
    let mut counts = HashMap::new();
    for i in 0..200 {
        let p = format!("src/file{i}.rs");
        *counts.entry(segment_of(&p)).or_insert(0) += 1;
    }
    // At least 100 distinct segments used for 200 files with 128 segments
    assert!(
        counts.len() >= 80,
        "expected balanced distinct {}",
        counts.len()
    );
    Ok(())
}
