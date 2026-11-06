# xcalygraph Deferred Features

Considered and deliberately not in `docs/spec.md`. Each entry names the evidence that moves it into the spec.

| feature | trigger |
|---------|---------|
| MCP server | an agent host that cannot run CLI commands, or an evaluation showing better results than CLI plus skill |
| Query daemon / resident memory index | freshness or startup cost misses the latency targets on reference corpora |
| Leiden communities, surprising cross-community edges | evaluation shows module structure insufficient for orientation questions |
| Multi-repository graph | an owner workflow that needs cross-repository queries |
| Full type inference (generic instantiation, return types beyond declared ones) or SCIP ingest | inferred-edge precision below 85 % on a reference corpus |
| Audio and video transcription, image OCR and captioning | a local engine without runtime dependencies plus an evaluation showing agent value |
| Live database schema introspection | an owner workflow where schema files in the tree are insufficient |
| Google Workspace and MCP server ingest | an owner workflow that needs them |
| Framework entrypoints beyond file-based routes (code-registered HTTP routes, CLI frameworks, job handlers) | evaluation questions about service entrypoints that `map` and `find` miss |
| Go `//go:embed` pattern resolution to files (directive facts are extracted) | agent questions about embedded assets that `show` on the directive's variable misses |
| Rust source files declared under several `#[path]` module names | a reference corpus where this pattern produces wrong or missing answers |
| TypeScript type-system depth beyond declared types (declaration merging across scripts and namespaces, generic substitution, literal key canonicalization) | inferred-edge precision below 85 % or agent-benchmark misses traced to it |
| Smaller index through higher shard compression and block-packed postings (measured about 1.5 MB of 20 MB) | a measured quality gain that needs more index bytes than the limit leaves |
| Search terms selected by rarity instead of first occurrence (doubling terms per symbol lowered sealed quality, so truncation is not the limit) | a retrieval miss traced to a term beyond the cap |
| Related-symbol bundle (`ctx`, personalized PageRank; removed because `find` beat it at every budget on the sealed questions) | agent questions whose answer shares no word with the question and is one graph hop from a hit |
| Rust enum variants as graph targets ("where is this variant built or matched?"; they dominate the compiler's type references, `rg` answers this today) | an agent question that `rg` plus `show` on the enum cannot answer within budget |
| Exact confidence for uniquely resolved call results with a declared return (uniform rule today: call results are inferred; the oracle's inferred strata are at least 98.3 % correct) | agent traces showing follow-up reads caused by the inferred marker |
| Profile-guided optimization of the Rust code on top of the C parser profile (Rust-only: 65.2 against 67.1 G instructions on the reference corpus, 2 to 3 % wall; 11 MB profile per target) | a measured gain above 10 % on top of the C parser profile |
| Typed operation facts (env read, cookie write, route registration, render, table reference) as annotations or nodes | a frozen behavior miss the words of the source cannot answer; 11 of 14 behavior questions already show the implementing symbol (`scripts/benchmarks/measure-behavior.sh`) and the 3 misses are vocabulary (`register` for `AddCommand`), which operation facts would not fix |
| Property read and write roles (getter against setter of one name) | a source-labelled agent question that needs the access role; two correct and two incorrect inferred facts seen on a pinned corpus, classified in the call recall work |
| Language meta-knowledge (error propagation, cleanup and lifecycle relations) | a source-labelled agent question whose required facts `find`, `show` or a chain cannot reach today |
| Typo-tolerant search | evaluation misses caused by misspelled terms |
| Media durations of WebM, Matroska, MP3 and Ogg | agent questions that need these durations (format is reported) |
