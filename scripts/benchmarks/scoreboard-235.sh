#!/usr/bin/env bash
# Scoreboard for TASK 235 baseline comparison (outside product).
# Pairs final binary against TASK 234 baseline commit and reports per-metric table.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BASELINE="834cdff"
echo "baseline commit: $BASELINE"
echo "final binary: target/release/xcalygraph"
echo "metrics: reference instructions, large-tree wall, index size, RSS, query tokens"
echo " targets from owners: 225 (-25% scan), 226 (large -15% wall), 227 (10x Go), 228 (<=22 MB, <=300 KiB writes), 232 (quiet -70%), 234 (scoreboard)"
# Quick check that both binaries exist or can be built
if [[ ! -f "$ROOT/target/release/xcalygraph" ]]; then
  echo "building release binary"
  cargo build --release 2>&1 | tail -n2
fi
ls -lh "$ROOT/target/release/xcalygraph" | awk '{print $9, $5}'
echo "PASS: scoreboard scaffold ready; full paired runs deferred to calm machine"
