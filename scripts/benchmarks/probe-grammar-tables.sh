#!/usr/bin/env bash
# Probe grammar table sizes for TASK 231 decision (outside product).
# Sums parser.c sizes for all tree-sitter crates in Cargo.lock to estimate INTEGER_TABLE saving.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOCK="$ROOT/Cargo.lock"
if [[ ! -f "$LOCK" ]]; then echo "no lock" >&2; exit 1; fi
TOTAL=0
for crate in tree-sitter-go tree-sitter-rust tree-sitter-python tree-sitter-typescript tree-sitter-javascript; do
  # Find any parser.c under cargo registry
  FOUND=$(find "${CARGO_HOME:-$HOME/.cargo/registry/src}" -path "*${crate}*/src/parser.c" 2>/dev/null | head -n 1 || true)
  if [[ -n "$FOUND" && -f "$FOUND" ]]; then
    SZ=$(wc -c < "$FOUND" | tr -d ' ')
    TOTAL=$((TOTAL + SZ))
    echo "$crate: $SZ bytes"
  else
    echo "$crate: not found"
  fi
done
echo "total sampled: $TOTAL bytes"
echo "estimated all grammars (40 langs): $((TOTAL * 8)) bytes (8x sampling ~ 25 MB threshold)"
if (( TOTAL * 8 > 25*1024*1024 )); then
  echo "PASS: INTEGER_TABLE path pays (>25 MB)"
else
  echo "FAIL: below threshold" >&2
  exit 1
fi
