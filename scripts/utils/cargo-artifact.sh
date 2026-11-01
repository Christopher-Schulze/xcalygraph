#!/usr/bin/env bash
# Select this repository's executable from a Cargo JSON build stream.
# Usage: cargo ... --message-format=json | scripts/utils/cargo-artifact.sh [--test --passthrough]
set -euo pipefail
if (( $# == 0 )); then
  mode="binary"
elif (( $# == 2 )) && [[ "$1" == --test && "$2" == --passthrough ]]; then
  mode="test"
else
  echo "usage: cargo-artifact.sh [--test --passthrough]" >&2
  exit 2
fi
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 -I -c '
import json
import os
from pathlib import Path
import sys

manifest = (Path(sys.argv[1]) / "Cargo.toml").resolve()
test_mode = sys.argv[2] == "test"
output = sys.stdout if test_mode else sys.stderr
executables = []
for line in sys.stdin:
    if not line.startswith("{"):
        output.write(line)
        output.flush()
        continue
    try:
        item = json.loads(line)
    except json.JSONDecodeError:
        output.write(line)
        output.flush()
        continue
    if not isinstance(item, dict):
        continue
    if item.get("reason") == "compiler-message":
        rendered = item.get("message", {}).get("rendered")
        if rendered:
            sys.stderr.write(rendered)
        continue
    target = item.get("target", {})
    profile = item.get("profile", {})
    path = item.get("manifest_path")
    executable = item.get("executable")
    if (item.get("reason") == "compiler-artifact" and isinstance(target, dict)
            and isinstance(target.get("kind"), list) and "bin" in target["kind"]
            and target.get("name") == "xcalygraph" and isinstance(profile, dict)
            and profile.get("test") is test_mode
            and isinstance(path, str) and Path(path).resolve() == manifest and isinstance(executable, str)):
        executables.append(Path(executable))
if len(executables) != 1 or not executables[0].is_absolute() or not executables[0].is_file():
    raise SystemExit("cargo-artifact.sh: Cargo did not report exactly one existing xcalygraph executable")
if test_mode:
    with os.fdopen(3, "w", closefd=False) as stream:
        print(executables[0], file=stream)
else:
    print(executables[0])
' "${REPO_ROOT}" "${mode}"
