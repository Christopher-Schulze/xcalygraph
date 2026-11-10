#!/usr/bin/env bash
# Corpus inventory for the TASK 227 Go gate (outside product).
# Reports Go corpus bytes available for a future lexer-throughput harness.
# It measures NO throughput itself: the stage-2 harness does not exist yet.
# Target when it does: 10x over tree-sitter on the frozen Go corpus.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CORPUS="$ROOT/.cache/corpus-snapshots/go"
if [[ ! -d "$CORPUS" ]]; then
  CORPUS="$ROOT/research/corpora/cobra"
fi
if [[ ! -d "$CORPUS" ]]; then
  echo "no Go corpus available" >&2
  exit 2
fi
FILES=$(find "$CORPUS" -name "*.go" 2>/dev/null | head -n 20)
if [[ -z "$FILES" ]]; then
  echo "no Go files under $CORPUS; inventory only, nothing measured" >&2
  exit 2
fi
TOTAL_BYTES=0
COUNT=0
for f in $FILES; do
  TOTAL_BYTES=$((TOTAL_BYTES + $(wc -c < "$f" | tr -d ' ')))
  COUNT=$((COUNT + 1))
done
echo "go files: $COUNT"
echo "go corpus bytes: $TOTAL_BYTES"
echo "no throughput measured: stage-2 lexer harness missing (TASK 227 open)"
