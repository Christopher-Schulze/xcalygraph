#!/usr/bin/env bash
# Compare unchanged explicit-index latency and writes on one verified source snapshot.
# Usage: measure-index-validation.sh SNAPSHOT BEFORE_BINARY AFTER_BINARY [RUNS]
set -euo pipefail

if (( $# < 3 || $# > 4 )); then
  echo "usage: measure-index-validation.sh SNAPSHOT BEFORE_BINARY AFTER_BINARY [RUNS]" >&2
  exit 2
fi
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly RUNS="${4:-15}"
if [[ ! "${RUNS}" =~ ^[0-9]{1,3}$ ]] || (( 10#${RUNS} < 1 || 10#${RUNS} > 100 )); then
  echo "measure-index-validation.sh: runs must be 1 to 100" >&2
  exit 2
fi
if [[ -L "${REPO_ROOT}/.cache" || -L "${REPO_ROOT}/.cache/benchmarks" ]]; then
  echo "measure-index-validation.sh: benchmark cache must not be a symlink" >&2
  exit 1
fi
mkdir -p "${REPO_ROOT}/.cache/benchmarks"
WORK="$(mktemp -d "${REPO_ROOT}/.cache/benchmarks/index-validation.XXXXXXXX")"
readonly WORK
mkdir "${WORK}/home"
echo "evidence: ${WORK}"
env HOME="${WORK}/home" bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" --verify "$1" >/dev/null
python3 -I - "$1" "$2" "$3" "${RUNS}" "${WORK}" <<'PYTHON'
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time

source, before, after = (Path(name).resolve(strict=True) for name in sys.argv[1:4])
runs, work = int(sys.argv[4]), Path(sys.argv[5])
index = work / "index"
environment = dict(os.environ, HOME=str(work / "home"))
environment.pop("XCALYGRAPH_INDEX_DIR", None)

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def invoke(binary):
    started = time.monotonic_ns()
    result = subprocess.run([str(binary), "index", "-C", str(source), "--index-dir", str(index)],
                            env=environment, capture_output=True)
    elapsed = (time.monotonic_ns() - started) / 1_000_000
    if result.returncode:
        raise ValueError("index command failed: " + result.stderr.decode(errors="replace"))
    return result.stdout, result.stderr, elapsed

def snapshot():
    entries = {}
    for path in sorted(index.rglob("*")):
        if path.is_symlink():
            raise ValueError("unexpected symlink in benchmark index")
        info = path.stat()
        entries[str(path.relative_to(index))] = {
            "inode": info.st_ino, "mtime_ns": info.st_mtime_ns,
            "sha256": digest(path) if path.is_file() else None}
    return entries

invoke(before)  # Build the isolated index once.
baseline = invoke(before)[:2]
if invoke(after)[:2] != baseline or invoke(before)[:2] != baseline:
    raise ValueError("unchanged index output differs between binaries")
initial = snapshot()
samples = {"before": [], "after": []}
observations = []
for number in range(runs):
    order = (("before", before), ("after", after)) if number % 2 == 0 else (("after", after), ("before", before))
    for label, binary in order:
        load = os.getloadavg()
        stdout, stderr, elapsed = invoke(binary)
        if (stdout, stderr) != baseline:
            raise ValueError("unchanged index output changed during measurement")
        samples[label].append(elapsed)
        observations.append({"binary": label, "wall_ms": elapsed, "load": load})
final = snapshot()
if initial != final:
    raise ValueError("unchanged index was modified during measurement")

def summarize(values):
    ordered = sorted(values)
    return {"median_ms": statistics.median(ordered), "p95_ms": ordered[math.ceil(len(ordered) * 0.95) - 1],
            "max_ms": ordered[-1]}

report = {"source_metadata_sha256": digest(source.parent / "snapshot.json"),
          "before_sha256": digest(before), "after_sha256": digest(after), "runs_per_binary": runs,
          "before": summarize(samples["before"]), "after": summarize(samples["after"]),
          "index_unchanged": True, "observations": observations}
for name, value in (("index-before.json", initial), ("index-after.json", final), ("results.json", report)):
    with (work / name).open("x") as stream:
        json.dump(value, stream, sort_keys=True)
        stream.write("\n")
print(json.dumps({key: value for key, value in report.items() if key != "observations"}, sort_keys=True))
PYTHON
env HOME="${WORK}/home" bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" --verify "$1" >/dev/null
echo "evidence: ${WORK}"
