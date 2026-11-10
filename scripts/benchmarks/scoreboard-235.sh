#!/usr/bin/env bash
# Scoreboard helper for TASK 235 (outside product).
# Reports the baseline commit, the current binary and the owning targets.
# It does NOT run the paired comparison itself; full paired runs happen on a
# calm machine via scripts/benchmarks/compare-binaries.sh and are recorded
# in the TASK notes. Exit 0 means the inventory printed, not that any
# target passed.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BASELINE="834cdff"
echo "baseline commit: $BASELINE"
echo "current HEAD: $(git -C "$ROOT" rev-parse --short HEAD)"
echo "final binary: target/release/xcalygraph"
echo "metrics: reference instructions, large-tree wall, index size, RSS, query tokens"
echo "targets from owners: 225 (-25% scan), 226 (large -15% wall), 227 (10x Go), 228 (<=22 MB, <=300 KiB writes), 232 (quiet -70%), 234 (scoreboard)"
if [[ -f "$ROOT/target/release/xcalygraph" ]]; then
  stat -f "%N %z bytes" "$ROOT/target/release/xcalygraph"
else
  echo "no release binary built; run scripts/build/build-release.sh first"
fi
echo "scaffold only: no comparison executed"
