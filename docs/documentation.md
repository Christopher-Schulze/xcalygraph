# xcalygraph Documentation

- [Overview](#overview)
- [Upstream reference](#upstream-reference)

## Overview

xcalygraph is a single-binary CLI for AI agents. It turns any directory (code repository, server filesystem, document corpus) into a dense, agent-readable knowledge graph plus precomputed analyses, so an agent gets verified structural context with few tokens and little orchestration work. It ships with one compact agent skill that the binary installs and updates together with itself.

It is a clean-room rewrite of a selected subset of Graphify, without visualization, without runtime dependencies and optimized for build speed, incremental updates and token efficiency of its output. Project rules: `AGENTS.md`.

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

The clone is analysis material only: behavior is reimplemented, code is not copied (see `AGENTS.md`, Upstream reference).
