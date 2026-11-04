//! Development tools run as ignored tests: the precision sampler
//! (`scripts/benchmarks/sample-precision.sh`) and the edge dump used by
//! `scripts/benchmarks/compare-depth.sh` and `scripts/benchmarks/measure-edge-oracle.sh`.

use super::*;

const PRECISION_SAMPLE_SCHEMA: &str = "xcalygraph-language-precision-v1";
const PRECISION_SAMPLE_TEXT_LIMIT: usize = 500;

#[derive(Clone, Copy)]
struct PrecisionIdentity<'a> {
    corpus: &'a str,
    corpus_revision: &'a str,
    corpus_sha256: &'a str,
    xcalygraph_source_sha256: &'a str,
    sampler_source_sha256: &'a str,
    sampler_binary_sha256: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrecisionSample {
    sample_id: String,
    source_language: String,
    edge_kind: String,
    confidence: String,
    source_graph_node_id: u32,
    source_path: String,
    source_line: u32,
    source_name: String,
    source_signature: String,
    source_signature_sha256: String,
    source_node_kind: &'static str,
    source_text: String,
    source_text_sha256: String,
    source_text_length_bytes: usize,
    source_text_truncated: bool,
    target_graph_node_id: u32,
    target_language: String,
    target_path: String,
    target_line: u32,
    target_name: String,
    target_signature: String,
    target_signature_sha256: String,
    target_node_kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrecisionStratum {
    source_language: String,
    edge_kind: String,
    confidence: String,
    population: usize,
    samples: Vec<PrecisionSample>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrecisionManifest {
    strata: Vec<PrecisionStratum>,
}

struct RankedEdge {
    sample_id: [u8; 32],
    edge_index: usize,
}

struct SelectedEdge {
    stratum_index: usize,
    sample_index: usize,
    edge_index: usize,
    sample_id: [u8; 32],
}

fn sha256_digest(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut output = [0; 32];
    output.copy_from_slice(&digest);
    output
}

fn digest_hex(digest: &[u8]) -> String {
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest_hex(&sha256_digest(bytes))
}

fn hash_field(hasher: &mut Sha256, name: &str, value: &[u8]) {
    hasher.update((name.len() as u64).to_be_bytes());
    hasher.update(name.as_bytes());
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn hash_number(hasher: &mut Sha256, name: &str, value: u32) {
    hash_field(hasher, name, &value.to_be_bytes());
}

fn confidence_id(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Exact => "exact",
        Confidence::Inferred => "inferred",
    }
}

fn signature_status(node_kind: &str, signature: &str) -> &'static str {
    match (node_kind, signature.is_empty()) {
        ("file", _) => "not-applicable",
        (_, true) => "empty",
        _ => "present",
    }
}

fn sample_id(
    identity: PrecisionIdentity<'_>,
    edge: &Edge,
    source: NodeLocation<'_>,
    target: NodeLocation<'_>,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, "schema", PRECISION_SAMPLE_SCHEMA.as_bytes());
    hash_field(&mut hasher, "corpus", identity.corpus.as_bytes());
    hash_field(
        &mut hasher,
        "corpus_revision",
        identity.corpus_revision.as_bytes(),
    );
    hash_field(
        &mut hasher,
        "corpus_sha256",
        identity.corpus_sha256.as_bytes(),
    );
    hash_field(
        &mut hasher,
        "xcalygraph_source_sha256",
        identity.xcalygraph_source_sha256.as_bytes(),
    );
    hash_field(
        &mut hasher,
        "sampler_source_sha256",
        identity.sampler_source_sha256.as_bytes(),
    );
    hash_field(
        &mut hasher,
        "sampler_binary_sha256",
        identity.sampler_binary_sha256.as_bytes(),
    );
    hash_field(&mut hasher, "source_language", source.language.as_bytes());
    hash_field(&mut hasher, "edge_kind", edge.kind.id().as_bytes());
    hash_field(
        &mut hasher,
        "confidence",
        confidence_id(edge.confidence).as_bytes(),
    );
    hash_number(&mut hasher, "source_graph_node_id", source.node);
    hash_field(&mut hasher, "source_path", source.path.as_bytes());
    hash_number(&mut hasher, "source_line", edge.line);
    hash_field(&mut hasher, "source_name", source.name.as_bytes());
    hash_field(&mut hasher, "source_signature", source.signature.as_bytes());
    hash_field(&mut hasher, "source_node_kind", source.node_kind.as_bytes());
    hash_field(&mut hasher, "target_language", target.language.as_bytes());
    hash_number(&mut hasher, "target_graph_node_id", target.node);
    hash_field(&mut hasher, "target_path", target.path.as_bytes());
    hash_number(&mut hasher, "target_line", target.line);
    hash_field(&mut hasher, "target_name", target.name.as_bytes());
    hash_field(&mut hasher, "target_signature", target.signature.as_bytes());
    hash_field(&mut hasher, "target_node_kind", target.node_kind.as_bytes());
    let digest = hasher.finalize();
    let mut output = [0; 32];
    output.copy_from_slice(&digest);
    output
}

fn line_starts(content: &[u8]) -> Vec<usize> {
    let mut starts = Vec::new();
    starts.push(0);
    for (offset, byte) in content.iter().enumerate() {
        if *byte == b'\n' && offset + 1 < content.len() {
            starts.push(offset + 1);
        }
    }
    starts
}

fn source_line<'a>(content: &'a [u8], starts: &[usize], line: u32) -> &'a [u8] {
    let Some(&start) = line
        .checked_sub(1)
        .and_then(|line| usize::try_from(line).ok())
        .and_then(|index| starts.get(index))
    else {
        return &[];
    };
    let remainder = &content[start..];
    let end = remainder
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(remainder.len());
    &remainder[..end]
}

fn precision_manifest(
    graph: &Graph,
    root: &Path,
    identity: PrecisionIdentity<'_>,
    sample_limit: usize,
) -> io::Result<PrecisionManifest> {
    if sample_limit == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "precision sample limit must be greater than zero",
        ));
    }
    let mut buckets: BTreeMap<(String, EdgeKind, Confidence), Vec<RankedEdge>> = BTreeMap::new();
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        let source = node_location(graph, edge.source)
            .ok_or_else(|| io::Error::other("edge has an invalid source node"))?;
        let target = node_location(graph, edge.target)
            .ok_or_else(|| io::Error::other("edge has an invalid target node"))?;
        let id = sample_id(identity, edge, source, target);
        buckets
            .entry((source.language.to_owned(), edge.kind, edge.confidence))
            .or_default()
            .push(RankedEdge {
                sample_id: id,
                edge_index,
            });
    }

    let mut strata = Vec::with_capacity(buckets.len());
    let mut selected_by_file: BTreeMap<u32, Vec<SelectedEdge>> = BTreeMap::new();
    let mut sample_slots = Vec::with_capacity(buckets.len());
    for ((source_language, edge_kind, confidence), mut candidates) in buckets {
        let population = candidates.len();
        candidates.sort_by_key(|candidate| candidate.sample_id);
        let sample_count = population.min(sample_limit);
        let stratum_index = strata.len();
        sample_slots.push(vec![None; sample_count]);
        for (sample_index, candidate) in candidates.into_iter().take(sample_count).enumerate() {
            let edge = &graph.edges[candidate.edge_index];
            let source = node_location(graph, edge.source)
                .ok_or_else(|| io::Error::other("edge has an invalid source node"))?;
            selected_by_file
                .entry(source.file)
                .or_default()
                .push(SelectedEdge {
                    stratum_index,
                    sample_index,
                    edge_index: candidate.edge_index,
                    sample_id: candidate.sample_id,
                });
        }
        strata.push(PrecisionStratum {
            source_language,
            edge_kind: edge_kind.id().to_owned(),
            confidence: confidence_id(confidence).to_owned(),
            population,
            samples: Vec::with_capacity(sample_count),
        });
    }

    for (file_index, mut selected) in selected_by_file {
        let file = graph
            .files
            .get(file_index as usize)
            .ok_or_else(|| io::Error::other("selected edge has an invalid source file"))?;
        let content = fs::read(root.join(&file.path))?;
        let starts = line_starts(&content);
        selected
            .sort_by_key(|candidate| (graph.edges[candidate.edge_index].line, candidate.sample_id));
        for candidate in selected {
            let edge = &graph.edges[candidate.edge_index];
            let source = node_location(graph, edge.source)
                .ok_or_else(|| io::Error::other("edge has an invalid source node"))?;
            let target = node_location(graph, edge.target)
                .ok_or_else(|| io::Error::other("edge has an invalid target node"))?;
            let raw_text = source_line(&content, &starts, edge.line);
            let lossy_text = String::from_utf8_lossy(raw_text);
            let preview: Vec<char> = lossy_text
                .chars()
                .take(PRECISION_SAMPLE_TEXT_LIMIT + 1)
                .collect();
            let source_text_truncated = preview.len() > PRECISION_SAMPLE_TEXT_LIMIT;
            let source_text = preview
                .iter()
                .copied()
                .take(PRECISION_SAMPLE_TEXT_LIMIT)
                .collect::<String>();
            let confidence = confidence_id(edge.confidence).to_owned();
            let sample = PrecisionSample {
                sample_id: digest_hex(&candidate.sample_id),
                source_language: source.language.to_owned(),
                edge_kind: edge.kind.id().to_owned(),
                confidence,
                source_graph_node_id: source.node,
                source_path: source.path.to_owned(),
                source_line: edge.line,
                source_name: source.name.to_owned(),
                source_signature: source.signature.to_owned(),
                source_signature_sha256: sha256_hex(source.signature.as_bytes()),
                source_node_kind: source.node_kind,
                source_text,
                source_text_sha256: sha256_hex(raw_text),
                source_text_length_bytes: raw_text.len(),
                source_text_truncated,
                target_graph_node_id: target.node,
                target_language: target.language.to_owned(),
                target_path: target.path.to_owned(),
                target_line: target.line,
                target_name: target.name.to_owned(),
                target_signature: target.signature.to_owned(),
                target_signature_sha256: sha256_hex(target.signature.as_bytes()),
                target_node_kind: target.node_kind,
            };
            let slot = sample_slots
                .get_mut(candidate.stratum_index)
                .and_then(|samples| samples.get_mut(candidate.sample_index))
                .ok_or_else(|| io::Error::other("selected sample has an invalid stratum slot"))?;
            *slot = Some(sample);
        }
    }
    for (stratum, samples) in strata.iter_mut().zip(sample_slots) {
        stratum.samples = samples
            .into_iter()
            .map(|sample| {
                sample.ok_or_else(|| io::Error::other("selected sample was not materialized"))
            })
            .collect::<io::Result<Vec<_>>>()?;
    }
    Ok(PrecisionManifest { strata })
}

fn precision_manifest_records(
    identity: PrecisionIdentity<'_>,
    sample_limit: usize,
    manifest: &PrecisionManifest,
) -> Vec<String> {
    use crate::app::query::output::{Value, json_object};

    let mut records = Vec::with_capacity(
        1 + manifest.strata.len()
            + manifest
                .strata
                .iter()
                .map(|stratum| stratum.samples.len())
                .sum::<usize>(),
    );
    records.push(json_object(&[
        ("record_type", Value::Str("run")),
        ("schema", Value::Str(PRECISION_SAMPLE_SCHEMA)),
        ("corpus", Value::Str(identity.corpus)),
        ("corpus_revision", Value::Str(identity.corpus_revision)),
        ("corpus_snapshot_sha256", Value::Str(identity.corpus_sha256)),
        (
            "xcalygraph_source_sha256",
            Value::Str(identity.xcalygraph_source_sha256),
        ),
        (
            "sampler_source_sha256",
            Value::Str(identity.sampler_source_sha256),
        ),
        (
            "sampler_binary_sha256",
            Value::Str(identity.sampler_binary_sha256),
        ),
        ("sample_limit", Value::Int(sample_limit as u64)),
    ]));
    for stratum in &manifest.strata {
        records.push(json_object(&[
            ("record_type", Value::Str("stratum")),
            ("corpus", Value::Str(identity.corpus)),
            ("corpus_revision", Value::Str(identity.corpus_revision)),
            ("corpus_snapshot_sha256", Value::Str(identity.corpus_sha256)),
            (
                "xcalygraph_source_sha256",
                Value::Str(identity.xcalygraph_source_sha256),
            ),
            (
                "sampler_source_sha256",
                Value::Str(identity.sampler_source_sha256),
            ),
            (
                "sampler_binary_sha256",
                Value::Str(identity.sampler_binary_sha256),
            ),
            ("source_language", Value::Str(&stratum.source_language)),
            ("edge_kind", Value::Str(&stratum.edge_kind)),
            ("confidence", Value::Str(&stratum.confidence)),
            ("population", Value::Int(stratum.population as u64)),
            ("sample_count", Value::Int(stratum.samples.len() as u64)),
            ("sample_limit", Value::Int(sample_limit as u64)),
            (
                "population_exhausted",
                Value::Int(u64::from(stratum.population <= sample_limit)),
            ),
        ]));
        for sample in &stratum.samples {
            records.push(json_object(&[
                ("record_type", Value::Str("sample")),
                ("schema", Value::Str(PRECISION_SAMPLE_SCHEMA)),
                ("sample_id", Value::Str(&sample.sample_id)),
                ("adjudication_status", Value::Str("pending")),
                ("corpus", Value::Str(identity.corpus)),
                ("corpus_revision", Value::Str(identity.corpus_revision)),
                ("corpus_snapshot_sha256", Value::Str(identity.corpus_sha256)),
                (
                    "xcalygraph_source_sha256",
                    Value::Str(identity.xcalygraph_source_sha256),
                ),
                (
                    "sampler_source_sha256",
                    Value::Str(identity.sampler_source_sha256),
                ),
                (
                    "sampler_binary_sha256",
                    Value::Str(identity.sampler_binary_sha256),
                ),
                ("source_language", Value::Str(&sample.source_language)),
                ("edge_kind", Value::Str(&sample.edge_kind)),
                ("confidence", Value::Str(&sample.confidence)),
                (
                    "source_graph_node_id",
                    Value::Int(sample.source_graph_node_id as u64),
                ),
                ("source_path", Value::Str(&sample.source_path)),
                ("source_line", Value::Int(sample.source_line as u64)),
                ("source_name", Value::Str(&sample.source_name)),
                ("source_signature", Value::Str(&sample.source_signature)),
                (
                    "source_signature_sha256",
                    Value::Str(&sample.source_signature_sha256),
                ),
                ("source_node_kind", Value::Str(sample.source_node_kind)),
                (
                    "source_signature_status",
                    Value::Str(signature_status(
                        sample.source_node_kind,
                        &sample.source_signature,
                    )),
                ),
                ("source_text", Value::Str(&sample.source_text)),
                ("source_text_sha256", Value::Str(&sample.source_text_sha256)),
                (
                    "source_text_length_bytes",
                    Value::Int(sample.source_text_length_bytes as u64),
                ),
                (
                    "source_text_truncated",
                    Value::Int(u64::from(sample.source_text_truncated)),
                ),
                (
                    "target_graph_node_id",
                    Value::Int(sample.target_graph_node_id as u64),
                ),
                ("target_language", Value::Str(&sample.target_language)),
                ("target_path", Value::Str(&sample.target_path)),
                ("target_line", Value::Int(sample.target_line as u64)),
                ("target_name", Value::Str(&sample.target_name)),
                ("target_signature", Value::Str(&sample.target_signature)),
                (
                    "target_signature_sha256",
                    Value::Str(&sample.target_signature_sha256),
                ),
                ("target_node_kind", Value::Str(sample.target_node_kind)),
                (
                    "target_signature_status",
                    Value::Str(signature_status(
                        sample.target_node_kind,
                        &sample.target_signature,
                    )),
                ),
            ]));
        }
    }
    records
}

fn write_precision_manifest(path: &Path, records: &[String]) -> io::Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    for record in records {
        output.write_all(record.as_bytes())?;
    }
    output.sync_all()
}

fn sample_environment(name: &str) -> io::Result<String> {
    std::env::var(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing required sampler identity: {name}"),
        )
    })
}

/// Development-only precision manifest sampler. The corpus is already a verified immutable
/// snapshot; output is JSONL, stratified by the actual source file type, and created exclusively.
/// `scripts/benchmarks/sample-precision.sh` builds its debug test binary once and supplies all
/// source identities. Its sampler schema and selection contract are in `docs/spec.md` section 12.
#[test]
#[ignore = "development tool; use scripts/benchmarks/sample-precision.sh"]
fn sample_edges() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::var_os("XCALYGRAPH_SAMPLE_ROOT")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing sample root"))?;
    let root = PathBuf::from(root).canonicalize()?;
    let per_bucket: usize = std::env::var("XCALYGRAPH_SAMPLE_SIZE")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(50);
    let output = PathBuf::from(sample_environment("XCALYGRAPH_SAMPLE_OUTPUT")?);
    let corpus = sample_environment("XCALYGRAPH_SAMPLE_CORPUS")?;
    let corpus_revision = sample_environment("XCALYGRAPH_SAMPLE_REVISION")?;
    let corpus_sha256 = sample_environment("XCALYGRAPH_SAMPLE_CORPUS_SHA256")?;
    let xcalygraph_source_sha256 = sample_environment("XCALYGRAPH_SAMPLE_XCALYGRAPH_SHA256")?;
    let sampler_source_sha256 = sample_environment("XCALYGRAPH_SAMPLE_SOURCE_SHA256")?;
    let sampler_binary_sha256 = sample_environment("XCALYGRAPH_SAMPLE_BINARY_SHA256")?;
    let identity = PrecisionIdentity {
        corpus: &corpus,
        corpus_revision: &corpus_revision,
        corpus_sha256: &corpus_sha256,
        xcalygraph_source_sha256: &xcalygraph_source_sha256,
        sampler_source_sha256: &sampler_source_sha256,
        sampler_binary_sha256: &sampler_binary_sha256,
    };
    let graph = corpus_graph(&root)?;
    let manifest = precision_manifest(&graph, &root, identity, per_bucket)?;
    let records = precision_manifest_records(identity, per_bucket, &manifest);
    write_precision_manifest(&output, &records)?;
    let sampled = manifest
        .strata
        .iter()
        .map(|stratum| stratum.samples.len())
        .sum::<usize>();
    println!(
        "precision manifest: {} strata, {sampled} samples, {}",
        manifest.strata.len(),
        output.display()
    );
    Ok(())
}

#[test]
fn precision_manifest_is_language_stratified_stable_and_identity_bound() -> io::Result<()> {
    assert_eq!(digest_hex(&[0, 1, 15, 16, 255]), "00010f10ff");
    let root = crate::infra::testdir::TestDir::new("precision-manifest")?;
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolve");
    let mut fixtures = Vec::new();
    walk(&fixture_root.join("go-app"), &fixture_root, &mut fixtures)?;
    walk(
        &fixture_root.join("python-app"),
        &fixture_root,
        &mut fixtures,
    )?;
    for (relative, content) in fixtures {
        let path = root.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    let graph = graph_of(root.path())?;
    let identity = PrecisionIdentity {
        corpus: "mixed-fixture",
        corpus_revision: "fixture-revision",
        corpus_sha256: "corpus-hash-a",
        xcalygraph_source_sha256: "xcalygraph-hash-a",
        sampler_source_sha256: "sampler-hash-a",
        sampler_binary_sha256: "binary-hash-a",
    };
    let manifest = precision_manifest(&graph, root.path(), identity, 1)?;
    let languages: Vec<&str> = manifest
        .strata
        .iter()
        .map(|stratum| stratum.source_language.as_str())
        .collect();
    assert!(languages.contains(&"go"));
    assert!(languages.contains(&"python"));
    for stratum in &manifest.strata {
        assert_eq!(stratum.samples.len(), stratum.population.min(1));
    }
    assert!(
        manifest
            .strata
            .iter()
            .any(|stratum| stratum.population > 1 && stratum.samples.len() == 1)
    );
    let exhausted = precision_manifest(&graph, root.path(), identity, usize::MAX)?;
    assert!(!exhausted.strata.is_empty());
    assert!(
        exhausted
            .strata
            .iter()
            .all(|stratum| stratum.samples.len() == stratum.population)
    );
    assert!(exhausted.strata.iter().all(|stratum| {
        stratum
            .samples
            .windows(2)
            .all(|samples| samples[0].sample_id <= samples[1].sample_id)
    }));

    let mut reordered = graph.clone();
    reordered.edges.reverse();
    let reordered_manifest = precision_manifest(&reordered, root.path(), identity, 1)?;
    assert_eq!(manifest, reordered_manifest);
    assert_eq!(
        precision_manifest_records(identity, 1, &manifest),
        precision_manifest_records(identity, 1, &reordered_manifest)
    );

    let original_ids: Vec<&str> = manifest
        .strata
        .iter()
        .flat_map(|stratum| {
            stratum
                .samples
                .iter()
                .map(|sample| sample.sample_id.as_str())
        })
        .collect();
    let mut unique_ids = original_ids.clone();
    unique_ids.sort_unstable();
    assert!(unique_ids.windows(2).all(|ids| ids[0] != ids[1]));
    let changed_identities = [
        PrecisionIdentity {
            corpus_sha256: "corpus-hash-b",
            ..identity
        },
        PrecisionIdentity {
            xcalygraph_source_sha256: "xcalygraph-hash-b",
            ..identity
        },
        PrecisionIdentity {
            sampler_source_sha256: "sampler-hash-b",
            ..identity
        },
        PrecisionIdentity {
            sampler_binary_sha256: "binary-hash-b",
            ..identity
        },
    ];
    for changed in changed_identities {
        let changed_manifest = precision_manifest(&graph, root.path(), changed, 1)?;
        let changed_ids: Vec<&str> = changed_manifest
            .strata
            .iter()
            .flat_map(|stratum| {
                stratum
                    .samples
                    .iter()
                    .map(|sample| sample.sample_id.as_str())
            })
            .collect();
        assert_ne!(original_ids, changed_ids);
    }

    let records = precision_manifest_records(identity, 1, &manifest);
    let sample_count = manifest
        .strata
        .iter()
        .map(|stratum| stratum.samples.len())
        .sum::<usize>();
    assert_eq!(records.len(), 1 + manifest.strata.len() + sample_count);
    assert!(records.iter().all(|record| record.ends_with('\n')));
    assert!(records[0].contains("\"record_type\":\"run\""));
    assert!(
        records
            .iter()
            .filter(|record| record.contains("\"record_type\":\"stratum\""))
            .all(|record| record.contains("\"sampler_source_sha256\":\"sampler-hash-a\""))
    );
    assert!(records.iter().any(|record| {
        record.contains("\"record_type\":\"sample\"")
            && record.contains("\"source_text_sha256\"")
            && record.contains("\"source_signature_sha256\"")
            && record.contains("\"target_signature_sha256\"")
    }));
    let exhausted_records = precision_manifest_records(identity, usize::MAX, &exhausted);
    assert!(
        exhausted_records
            .iter()
            .filter(|record| record.contains("\"record_type\":\"stratum\""))
            .all(|record| record.contains("\"population_exhausted\":1"))
    );
    let output = root.path().join("manifest.jsonl");
    write_precision_manifest(&output, &records)?;
    let retained = fs::read(&output)?;
    assert!(write_precision_manifest(&output, &records).is_err());
    assert_eq!(fs::read(output)?, retained);
    Ok(())
}

/// Depth comparison input (`scripts/benchmarks/compare-depth.sh`): writes every edge of the
/// root in `XCALYGRAPH_DUMP_ROOT` to `XCALYGRAPH_DUMP_FILE` as TSV lines `kind confidence
/// source_path line target_path target_line target_name source_decl_line source_name
/// source_node_kind`. The first seven columns remain the depth comparison prefix.
#[test]
#[ignore = "development tool; needs XCALYGRAPH_DUMP_ROOT and XCALYGRAPH_DUMP_FILE"]
fn dump_edges() -> Result<(), Box<dyn std::error::Error>> {
    let (Some(root), Some(file)) = (
        std::env::var_os("XCALYGRAPH_DUMP_ROOT"),
        std::env::var_os("XCALYGRAPH_DUMP_FILE"),
    ) else {
        return Ok(());
    };
    let graph = corpus_graph(&PathBuf::from(root).canonicalize()?)?;
    let symbols = graph.symbols.len() as u32;
    let location = |node: u32| -> (&str, u32, &str) {
        if node < symbols {
            let symbol = &graph.symbols[node as usize];
            (
                &graph.files[symbol.file as usize].path,
                symbol.line,
                &symbol.name,
            )
        } else {
            let path = &graph.files[(node - symbols) as usize].path;
            (path, 1, path)
        }
    };
    let mut out = String::new();
    for edge in &graph.edges {
        let (source_path, source_line, source_name) = location(edge.source);
        let source_kind = if edge.source < symbols {
            "symbol"
        } else {
            "file"
        };
        let (target_path, target_line, target_name) = location(edge.target);
        writeln!(
            out,
            "{}\t{}\t{source_path}\t{}\t{target_path}\t{target_line}\t{target_name}\t{source_line}\t{source_name}\t{source_kind}",
            edge.kind.id(),
            edge.confidence as u8,
            edge.line
        )?;
    }
    let mut output = OpenOptions::new().write(true).create_new(true).open(file)?;
    output.write_all(out.as_bytes())?;
    output.sync_all()?;
    Ok(())
}

#[test]
fn edge_dump_keeps_the_actual_same_line_caller() -> Result<(), Box<dyn std::error::Error>> {
    let root = crate::infra::testdir::TestDir::new("oracle-caller-dump")?;
    fs::create_dir(root.path().join("src"))?;
    fs::write(
        root.path().join("src/lib.rs"),
        include_bytes!("../../../tests/fixtures/oracle/caller-identities/src/lib.rs"),
    )?;
    let path = root.path().join("edges.tsv");
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .args([
            "--ignored",
            "--exact",
            "resolve::tests::precision_tools::dump_edges",
        ])
        .env("XCALYGRAPH_DUMP_ROOT", root.path())
        .env("XCALYGRAPH_DUMP_FILE", &path);
    let output = command.output()?;
    assert!(output.status.success(), "{output:?}");
    let text = fs::read_to_string(&path)?;
    assert!(
        !command.output()?.status.success(),
        "retained evidence was overwritten"
    );
    assert_eq!(fs::read_to_string(path)?, text);
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|line| line.split('\t').collect())
        .collect();
    assert!(rows.iter().all(|row| row.len() == 10), "{text}");
    for (target, caller) in [("first", "left"), ("second", "right")] {
        assert!(
            rows.iter().any(|row| row[0] == "calls"
                && row[3] == "3"
                && row[6] == target
                && row[7..] == ["3", caller, "symbol"]),
            "{text}"
        );
        let other = if caller == "left" { "right" } else { "left" };
        assert!(
            !rows.iter().any(|row| row[0] == "calls"
                && row[3] == "3"
                && row[6] == target
                && row[8] == other),
            "{text}"
        );
    }
    Ok(())
}
