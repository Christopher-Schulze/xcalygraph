# xcalygraph Documentation

- [Overview](#overview)
- [Upstream reference](#upstream-reference)

## Overview

xcalygraph is a single-binary CLI for AI agents. It turns any directory (code repository, server filesystem, document corpus) into a dense, agent-readable knowledge graph plus precomputed analyses, so an agent gets verified structural context with few tokens and little orchestration work. It ships with one compact agent skill that the binary installs and updates together with itself.

It is a clean-room rewrite of a selected subset of Graphify, without LLM calls, without visualization, without runtime dependencies and optimized for build speed, incremental updates and token efficiency of its output. Target blueprint: `docs/spec.md`; deferred features: `docs/deferred.md`.

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
