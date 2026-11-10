#!/usr/bin/env bash
# Grammar table inventory for the TASK 231 decision (outside product).
# Sums the generated parser.c source bytes of every tree-sitter grammar in
# the cargo registry. This bounds the INTEGER_TABLE saving from above:
# extracted tables cannot exceed their source, and the binary saving needs
# a separate load measurement once a sieve prototype exists.
set -euo pipefail
REGISTRY="${CARGO_HOME:-$HOME/.cargo/registry/src}"
TOTAL=0
COUNT=0
while IFS= read -r parser; do
  SZ=$(wc -c < "$parser" | tr -d ' ')
  TOTAL=$((TOTAL + SZ))
  COUNT=$((COUNT + 1))
  echo "$parser: $SZ bytes"
done < <(find "$REGISTRY" -path "*/tree-sitter-*/src/parser.c" 2>/dev/null | sort)
if ((COUNT == 0)); then
  echo "no tree-sitter parser.c found under $REGISTRY" >&2
  exit 2
fi
echo "grammars: $COUNT"
echo "total parser.c bytes: $TOTAL"
echo "upper bound only: no INTEGER_TABLE saving measured (TASK 231 open)"
