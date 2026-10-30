# xcalygraph Documentation

- [Overview](#overview)
- [Command line](#command-line)
- [Output format](#output-format)
- [Architecture](#architecture)
- [Build and test](#build-and-test)
- [Upstream reference](#upstream-reference)

## Overview

xcalygraph is a single-binary CLI for AI agents. It turns any directory (code repository, server filesystem, document corpus) into a dense, agent-readable knowledge graph plus precomputed analyses, so an agent gets verified structural context with few tokens and little orchestration work. It ships with one compact agent skill that the binary installs and updates together with itself.

It is a clean-room rewrite of a selected subset of Graphify, without LLM calls, without visualization, without runtime dependencies and optimized for build speed, incremental updates and token efficiency of its output. Target blueprint: `docs/spec.md`; deferred features: `docs/deferred.md`.

License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).

## Command line

Implemented commands; the full target surface is `docs/spec.md` section 2. `xcalygraph help` is the syntax reference.

| command | budget | output |
|---------|--------|--------|
| `map` | 2500 | root line (files, symbols, files per language), then sections `modules` (40 largest modules at path depth max 3: files, symbols, top 3 symbols by rank), `entry`, `hubs`, `cycles`, `layers`, `ext`, `env`; the budget cuts the lowest-priority section first |
| `find TERMS...` | 600 | ranked symbols and files, one line each, grouped under their file |
| `ctx TERMS...` | 2000 | up to 5 seeds (with doc sentence) plus related symbols by personalized PageRank, grouped by file |
| `show NAME [--all]` | 1200 | card: header, signature, doc, members (`m`), field names (`f`), supertypes (`^`), implementations (`v`), incoming (`<`, call sites), outgoing (`>`), tests (`t`, `t2` through one caller), env reads (`e`), ambiguity notice (`!`) |
| `path FROM TO` | 400 | one line per hop (`.` start, `>` forward, `<` against edge direction); without a directed path the undirected one, marked `?` |
| `index [--drop]` | none | `files N changed C deleted D`; `--drop` deletes the index of the root |
| `version` | none | `xcalygraph <version>`; aliases `-V`, `--version` |
| `help [COMMAND]` | none | syntax overview or one command with its flags; aliases `-h`, `--help`, `<command> --help` |

- Query flags: `-C, --root PATH`, `-b, --budget N` (tokens, bytes / 3), `--json`, `--read-only`, `--index-dir PATH`, `-p, --path GLOB` (gitignore syntax, repeatable), `-l, --lang LIST`, `-k, --kind LIST`, `--exact`, `--no-tests`; `show` adds `-d, --depth N` and `--all`.
- NAME forms: `qualified.name` or `qualified::name`, a qualified suffix (`Store.Get`), `path:line` (innermost symbol containing the line), `path::name`, a file path, or a bare name.
- Query commands refresh the index first; `--read-only` never writes and exits 5 when the index is missing or stale.
- Exit codes: 0 answer, 1 runtime error, 2 usage error, 3 ambiguous NAME (candidates on stdout), 4 no match, 5 index missing or stale under `--read-only`.
- Usage errors print one stderr line `xcalygraph: <error>; run 'xcalygraph help'` and nothing on stdout. A reader that closes stdout early (`| head`) is not an error.

## Output format

- Symbol lines are grouped under their file line and indented two spaces: `<line> <signature>`, or `<line> <kind> <name>` for symbols without a signature; `ctx` seeds append ` // <doc sentence>`.
- Relation lines: `<marker> <qualified name> <path>:<line>`; incoming edges cite the reference line, outgoing ones the target definition; edge kinds other than calls and type hierarchy are appended (`uses`, `imports`); a trailing `?` marks an inferred edge.
- Truncation by budget ends the output with `+<n> more`. Grouping never lets a lower-ranked hit displace a higher-ranked one: the best hits that fit are chosen first, then grouped.
- `--json` writes JSON Lines with the fields of `docs/spec.md` section 8; empty fields are omitted.

## Architecture

Single binary crate. Layers and modules under `src/`:

| layer | modules | responsibility |
|-------|---------|----------------|
| `cli` | `cli.rs` | argument parsing, help text, exit statuses |
| `app` | `app/` | one use case per command: workspace resolution, refresh pipeline, queries (`app/query/`) |
| `domain` | `domain/` | pure model: file types and roles, corpus records and change detection, facts, graph, tokenizer |
| `extract` | `extract/` | pure fact extraction: generic tree-sitter query engine (`engine.rs`), one language pack per grammar (`lang/`, queries in `queries/*.scm`), manifests |
| `resolve` | `resolve/` | pure global resolution of all facts to the symbol graph (module rules, resolution order steps 1 to 8, implementations, dispatch) and derived data (rank, module import graph, cycles, layers) |
| `infra` | `infra/` | parallel walk, scan and hashing, writer lock, index store (container, codec, fact shards, zero-copy `derived` file) |

`src/main.rs` wires the layers and owns process IO: stdout, stderr and the exit code.

Refresh pipeline: lock -> walk (`ignore` crate, parallel) -> stat diff against the file table -> hash and classify candidates (parallel) -> extract changed files (parallel) -> rewrite only the affected fact shards -> resolve all facts -> write `derived` -> write file table and meta. An empty change set writes nothing. `derived` is rebuilt when content changed, when its header is invalid, or when it was written by another binary or resolver version; a checksum failure found when a query opens it re-derives it once. Each language query compiles lazily, only when a file of that language is extracted.

Index files (`<index-dir>/<root name>-<hash>/`): `meta`, `files` (front-coded paths, stat, XXH3-128, role, file type), `facts/00..63` (varint-coded per-file facts referencing a shard-local string table), `derived` (fixed-size records cast in place from the memory map: string table, files, symbols with rank, forward and reverse CSR adjacency, modules, module import graph, externals, env sites, fields, type members, search postings with per-field term frequencies). Every file carries a header with magic, format version, kind and XXH3-64 checksum and is replaced by temp file plus rename.

Grammars are cargo features (`lang-go`, `lang-rust`; all on by default). Adding a language means a pack in `src/extract/lang/`, a query file and fixtures; the engine and resolver are shared.

## Build and test

- Toolchain: `rust-toolchain.toml` pins Rust 1.99.1 with rustfmt and clippy; rustup selects it inside the repository. `Cargo.toml` sets `rust-version` to the same release.
- Development builds: `cargo build` (debug profile).
- Gate: `scripts/tests/test.sh` runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` with `RUSTFLAGS=-Dwarnings`; any finding fails the run. Below 2 GiB free disk space it runs `cargo clean` first and aborts if that is not enough.
- Lints: clippy denies `unwrap_used`, `expect_used`, `panic`, `todo` and `unimplemented` for all targets (`Cargo.toml`); tests return `Result` instead of unwrapping.
- Tests: unit tests next to the code in `#[cfg(test)]` modules; end-to-end tests of the binary (stdout, stderr, exit codes) in `tests/`.
- Golden fixtures: `tests/fixtures/languages/<id>/` holds sources whose extracted facts are compared with `<file>.facts`; `tests/fixtures/resolve/<case>/` holds small roots whose resolved edges are compared with `<case>.edges`. `XCALYGRAPH_BLESS=1 cargo test` rewrites the golden files; every changed line is checked against the fixture source before commit. A determinism test compares `derived` images built with 1 and 8 threads.
- Precision sampling: `XCALYGRAPH_SAMPLE_ROOT=<dir> cargo test --release sample_edges -- --ignored --nocapture` prints up to `XCALYGRAPH_SAMPLE_SIZE` (default 50) edges per kind and confidence, each with the reference line and the target signature, for checking against source (`docs/spec.md` section 12).
- Benchmarks: `scripts/benchmarks/run-benchmarks.sh [CORPUS...]` builds the release binary and reports per corpus: files, full build ms, peak RSS, warm freshness ms, single-file refresh ms, index KB and KB rewritten by a single-file change (on a copy below `.cache/`). Corpora: this repository, the upstream clone and private ones listed as `name<TAB>path` in the gitignored `scripts/benchmarks/questions/private/corpora.tsv`. Requires `hyperfine`, `jq`, `rsync`.
- Publication check: `scripts/audits/check-publication.sh [RANGE]` runs before every push and fails on tracked local-only files, absolute home paths and private names (extended regexes in the gitignored `scripts/audits/private-patterns.txt`) in the tree or message of each commit; RANGE defaults to the commits not yet on `origin/main`.

## Upstream reference

| Field | Value |
|-------|-------|
| Source | https://github.com/Graphify-Labs/graphify |
| License | Apache-2.0 (earlier contributions also MIT, see upstream `NOTICE`) |
| Analysis commit | `0a8cd4e1e342235fe53e881fb5d591bf0460071f` (v0.9.97, 2026-10-27) |
| Local clone | `research/upstream/graphify` (gitignored) |

Refresh or pin the clone:

- `scripts/utils/sync-upstream.sh` checks out the upstream default branch head.
- `scripts/utils/sync-upstream.sh <COMMIT>` checks out an exact commit, for example the analysis commit above.

The clone is analysis material only: behavior is reimplemented, code is not copied; ported code or data keeps its license notice, with attribution in a root `NOTICE` file.

Upstream locations at the analysis commit:

| concern | location | behavior |
|---------|----------|----------|
| pipeline | `research/upstream/graphify/ARCHITECTURE.md` | detect -> extract -> build -> cluster -> analyze -> report -> export, NetworkX graphs |
| language dispatch | `research/upstream/graphify/graphify/extract.py:8889-9006` | 117 extensions, 56 extractor functions, about 45 languages; Dart, Apex, COBOL, Razor, Blade regex-based |
| syntax extraction | `research/upstream/graphify/graphify/extractors/engine.py:6090` | `_extract_generic`: hand-written recursive tree-sitter walks, no queries; node kind encoded in the label |
| node ids | `research/upstream/graphify/graphify/ids.py` | `make_id`, collisions salted with path plus sha1 |
| cross-file resolution | `research/upstream/graphify/graphify/extractors/resolution.py:1802` | single-threaded after the process pool; JS/TS and Python parsed twice; EXTRACTED for in-file calls and cross-file calls with import evidence, INFERRED 0.85 otherwise |
| extraction cache | `research/upstream/graphify/graphify/extract.py:9239` | JS/TS bypass the AST cache |
| incremental update | `research/upstream/graphify/graphify/watch.py:2416-2420` | manifest with mtime plus MD5, per-file re-extraction against the unchanged graph |
| clustering | `research/upstream/graphify/graphify/cluster.py` | Leiden, Louvain fallback, cohesion = intra edges / possible pairs |
| analyses | `research/upstream/graphify/graphify/analyze.py` | god nodes, surprising connections, suggested questions, import cycles up to length 5, graph diff |
| query | `research/upstream/graphify/graphify/serve.py:580`, `research/upstream/graphify/graphify/serve.py:1418` | label term scoring, up to 3 seeds, BFS depth 2 (CLI) or 3 (MCP), 3 chars per token budget |
| MCP tools | `research/upstream/graphify/graphify/serve.py:2305-2460` | query, node, neighbors, community, god nodes, stats, shortest path, three PR tools |
| LLM features | `research/upstream/graphify/graphify/llm.py` | docs and media semantics, hyperedges, AMBIGUOUS edges, community names |
| benchmark | `research/upstream/graphify/graphify/benchmark.py:55` | corpus size vs budget-capped query output, no answer quality |
| agent skill | `research/upstream/graphify/graphify/skill.md` plus per-host variants `skill-*.md` | protocol of inline `python -c` blocks; no self-update |
