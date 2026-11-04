//! Development tool run as an ignored test (`scripts/benchmarks/measure-query-cost.sh`): grammar
//! parse, query compile and query match time per language pack, and per pattern by ablation.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator, Tree};

use super::lang;
use crate::infra::scan;

struct Source {
    bytes: Vec<u8>,
    tree: Tree,
}

/// Runs the query over every file without using a match; returns the time and matches per pattern.
fn run(query: &Query, sources: &[Source]) -> (Duration, Vec<u64>) {
    let mut counts = vec![0; query.pattern_count()];
    let start = Instant::now();
    for source in sources {
        let mut cursor = QueryCursor::new();
        cursor.set_match_limit(4096);
        let mut matches = cursor.matches(query, source.tree.root_node(), source.bytes.as_slice());
        while let Some(found) = matches.next() {
            counts[found.pattern_index] += 1;
        }
    }
    (start.elapsed(), counts)
}

/// Best of three, against scheduler noise.
fn best(query: &Query, sources: &[Source]) -> (Duration, Vec<u64>) {
    let (mut fastest, counts) = run(query, sources);
    for _ in 0..2 {
        fastest = fastest.min(run(query, sources).0);
    }
    (fastest, counts)
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[test]
#[ignore = "development measurement: needs XCALYGRAPH_COST_ROOT"]
fn query_cost() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::var("XCALYGRAPH_COST_ROOT")?;
    let patterns = std::env::var_os("XCALYGRAPH_COST_PATTERNS").is_some();
    let changes = scan::scan(Path::new(&root), &[], &[], i64::MAX, None)?;
    let mut groups: BTreeMap<(String, usize), (&'static str, Language, Vec<String>)> =
        BTreeMap::new();
    for record in &changes.records {
        let Some(file_type) = record.file_type.filter(|_| record.role.is_extracted()) else {
            continue;
        };
        let Some((pack, language, _)) = lang::lookup(file_type, &record.path) else {
            continue;
        };
        groups
            .entry((file_type.id().to_owned(), language.abi_version()))
            .or_insert((pack.query, language, Vec::new()))
            .2
            .push(record.path.clone());
    }
    println!("pack\tfiles\tmegabytes\tparse_ms\tcompile_ms\tmatch_ms\tpatterns\tmatches");
    for ((name, _), (query_text, language, paths)) in groups {
        let mut parser = Parser::new();
        parser.set_language(&language)?;
        let mut sources = Vec::with_capacity(paths.len());
        let parse_start = Instant::now();
        let mut bytes = 0;
        for path in &paths {
            let content = std::fs::read(Path::new(&root).join(path))?;
            bytes += content.len();
            if let Some(tree) = parser.parse(&content, None) {
                sources.push(Source {
                    bytes: content,
                    tree,
                });
            }
        }
        let parse = parse_start.elapsed();
        let compile_start = Instant::now();
        let query = Query::new(&language, query_text)?;
        let compile = compile_start.elapsed();
        let (matching, counts) = best(&query, &sources);
        println!(
            "{name}\t{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{}\t{}",
            paths.len(),
            bytes as f64 / 1e6,
            milliseconds(parse),
            milliseconds(compile),
            milliseconds(matching),
            query.pattern_count(),
            counts.iter().sum::<u64>(),
        );
        if !patterns {
            continue;
        }
        for (pattern, count) in counts.iter().enumerate() {
            let mut without = Query::new(&language, query_text)?;
            without.disable_pattern(pattern);
            let (time, _) = best(&without, &sources);
            let start = query.start_byte_for_pattern(pattern);
            let end = query.end_byte_for_pattern(pattern);
            let first = query_text[start..].lines().next().unwrap_or_default();
            let remaining = format!("{}{}", &query_text[..start], &query_text[end..]);
            let compile_start = Instant::now();
            let compiled = Query::new(&language, &remaining).is_ok();
            let compile_without = compile_start.elapsed();
            println!(
                "  pattern\t{pattern}\tsaves_ms\t{:.1}\tcompile_saves_ms\t{:.1}\tmatches\t{}\t{}",
                milliseconds(matching) - milliseconds(time),
                if compiled {
                    milliseconds(compile) - milliseconds(compile_without)
                } else {
                    0.0
                },
                count,
                first.trim(),
            );
        }
    }
    Ok(())
}
