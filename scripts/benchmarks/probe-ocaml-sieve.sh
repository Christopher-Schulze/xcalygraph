#!/usr/bin/env bash
# Sieve table inventory for TASK 231 (outside product).
# Reports the source size of one integer table (ts_parse_table) in the Rust
# grammar's generated parser.c. This is an existence check for mechanical
# extraction, not a saving measurement: binary and load effects need the
# sieve prototype that does not exist yet.
set -euo pipefail
REGISTRY="${CARGO_HOME:-$HOME/.cargo/registry/src}"
PARSER="$(find "$REGISTRY" -path "*tree-sitter-rust*/src/parser.c" 2>/dev/null | head -n 1)"
if [[ ! -f "$PARSER" ]]; then
  echo "parser.c not found under $REGISTRY" >&2
  exit 2
fi
echo "parser: $PARSER"
# Count bytes of the large table block
TABLE_BYTES=$(awk '/static const uint16_t ts_parse_table/{flag=1} flag{count+=length($0)+1} /];/{if(flag) exit} END{print count+0}' "$PARSER")
echo "ts_parse_table bytes: $TABLE_BYTES"
STATE_COUNT=$(grep -o "#define STATE_COUNT [0-9]*" "$PARSER" | awk '{print $3}' || true)
SYMBOL_COUNT=$(grep -o "#define SYMBOL_COUNT [0-9]*" "$PARSER" | awk '{print $3}' || true)
echo "STATE_COUNT ${STATE_COUNT:-unknown} SYMBOL_COUNT ${SYMBOL_COUNT:-unknown}"
if ((TABLE_BYTES < 1024)); then
  echo "table too small to matter" >&2
  exit 1
fi
echo "inventory only: no saving measured (TASK 231 open)"
