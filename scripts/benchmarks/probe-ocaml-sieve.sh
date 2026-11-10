#!/usr/bin/env bash
# Sieve table inventory for TASK 231 (outside product).
# Reports the source size of one integer table (ts_parse_table) in the Rust
# grammar's generated parser.c. This is an existence check for mechanical
# extraction, not a saving measurement: binary and load effects need the
# sieve prototype that does not exist yet.
set -euo pipefail
REGISTRY="${CARGO_HOME:-$HOME/.cargo}/registry/src"
PARSER="$(find "$REGISTRY" -path "*/tree-sitter-rust-*/src/parser.c" -print -quit 2>/dev/null)"
if [[ ! -f "$PARSER" ]]; then
  echo "parser.c not found under $REGISTRY" >&2
  exit 2
fi
echo "parser: $PARSER"
# Count the declaration and its initializer, including the closing line.
if ! TABLE_BYTES=$(LC_ALL=C awk '
  /^static const uint16_t ts_parse_table\[/ { in_table = 1 }
  in_table { bytes += length($0) + 1 }
  in_table && /^};[[:space:]]*$/ { complete = 1; print bytes; exit }
  END { if (!complete) exit 1 }
' "$PARSER"); then
  echo "complete ts_parse_table initializer not found" >&2
  exit 1
fi
echo "ts_parse_table bytes: $TABLE_BYTES"
STATE_COUNT=$(grep -o "#define STATE_COUNT [0-9]*" "$PARSER" | awk '{print $3}' || true)
SYMBOL_COUNT=$(grep -o "#define SYMBOL_COUNT [0-9]*" "$PARSER" | awk '{print $3}' || true)
echo "STATE_COUNT ${STATE_COUNT:-unknown} SYMBOL_COUNT ${SYMBOL_COUNT:-unknown}"
echo "inventory only: no saving measured (TASK 231 open)"
