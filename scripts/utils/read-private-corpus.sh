#!/usr/bin/env bash
# Validate a private corpus TSV and return validated records or one path on request.
# Usage: read-private-corpus.sh validate LIST | list LIST | get LIST NAME
set -euo pipefail
if [[ "${1:-}" == validate && $# == 2 ]] || [[ "${1:-}" == list && $# == 2 ]] || [[ "${1:-}" == get && $# == 3 ]]; then
  python3 -I - "$@" <<'PYTHON'
import os
from pathlib import Path
import sys

action, listing = sys.argv[1:3]
selected = os.fsencode(sys.argv[3]) if action == "get" else None
if selected is not None and (not selected or any(byte < 32 or byte == 127 for byte in selected)):
    raise SystemExit("read-private-corpus.sh: invalid corpus name")
records = {}
for number, line in enumerate(Path(listing).read_bytes().split(b"\n"), 1):
    if not line or line.startswith(b"#"):
        continue
    if line.count(b"\t") != 1:
        raise SystemExit(f"read-private-corpus.sh: invalid record at line {number}")
    name, path = line.split(b"\t")
    if (not name or not path or any(byte < 32 or byte == 127 for byte in name + path)
            or name in records):
        raise SystemExit(f"read-private-corpus.sh: invalid or duplicate record at line {number}")
    records[name] = path
if action == "list":
    for name, path in records.items():
        sys.stdout.buffer.write(name + b"\t" + path + b"\n")
elif selected is not None:
    sys.stdout.buffer.write(records.get(selected, b""))
PYTHON
else
  echo "usage: read-private-corpus.sh validate LIST | list LIST | get LIST NAME" >&2
  exit 2
fi
