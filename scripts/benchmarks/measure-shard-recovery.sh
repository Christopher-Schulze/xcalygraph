#!/usr/bin/env bash
# Compare one-shard recovery with full builds on the same verified snapshot.
# Usage: measure-shard-recovery.sh SNAPSHOT BINARY [RUNS]
set -euo pipefail

if (( $# < 2 || $# > 3 )); then
  echo "usage: measure-shard-recovery.sh SNAPSHOT BINARY [RUNS]" >&2
  exit 2
fi
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
readonly RUNS="${3:-5}"
if [[ ! "${RUNS}" =~ ^[0-9]{1,2}$ ]] || (( 10#${RUNS} < 1 || 10#${RUNS} > 20 )); then
  echo "measure-shard-recovery.sh: runs must be 1 to 20" >&2
  exit 2
fi
if [[ -L "${REPO_ROOT}/.cache" || -L "${REPO_ROOT}/.cache/benchmarks" ]]; then
  echo "measure-shard-recovery.sh: benchmark cache must not be a symlink" >&2
  exit 1
fi
mkdir -p "${REPO_ROOT}/.cache/benchmarks"
WORK="$(mktemp -d "${REPO_ROOT}/.cache/benchmarks/shard-recovery.XXXXXXXX")"
readonly WORK
mkdir "${WORK}/home" "${WORK}/tmp"
echo "evidence: ${WORK}"
env HOME="${WORK}/home" TMPDIR="${WORK}/tmp" XDG_CONFIG_HOME="${WORK}/home/config" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" --verify "$1" >/dev/null
python3 -I - "$1" "$2" "${RUNS}" "${WORK}" <<'PYTHON'
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import time

source, binary = (Path(name).resolve(strict=True) for name in sys.argv[1:3])
runs, work = int(sys.argv[3]), Path(sys.argv[4])
environment = dict(os.environ, HOME=str(work / "home"), TMPDIR=str(work / "tmp"),
                   XDG_CONFIG_HOME=str(work / "home/config"),
                   GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1")
environment.pop("XCALYGRAPH_INDEX_DIR", None)
minimum_free = 2 * 1024**3
snapshot_metadata = json.loads((source.parent / "snapshot.json").read_text())
source_bytes = sum(item["bytes"] for item in snapshot_metadata["source"]["files"])
if shutil.disk_usage(work).free < minimum_free + max(1024**3, 3 * source_bytes):
    raise ValueError("insufficient free space for the seed index and the 2 GiB reserve")

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def invoke(base):
    started = time.monotonic_ns()
    result = subprocess.run([str(binary), "index", "-C", str(source), "--index-dir", str(base)],
                            env=environment, capture_output=True)
    elapsed = (time.monotonic_ns() - started) / 1_000_000
    if result.returncode:
        raise ValueError("index command failed: " + result.stderr.decode(errors="replace"))
    stores = list(base.iterdir())
    if len(stores) != 1 or not stores[0].is_dir() or stores[0].is_symlink():
        raise ValueError("expected one isolated index directory")
    return stores[0], elapsed

def state(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError("expected an isolated regular index file")
    info = path.stat()
    return info.st_ino, info.st_mtime_ns, digest(path)

def flip_payload(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size <= 64:
        raise ValueError("invalid benchmark fixture")
    with path.open("r+b") as stream:
        stream.seek(-1, os.SEEK_END)
        value = stream.read(1)
        stream.seek(-1, os.SEEK_END)
        stream.write(bytes([value[0] ^ 1]))

store, seed_ms = invoke(work / "recovery-index")
seed_paths = list(store.rglob("*"))
if any(path.is_symlink() for path in seed_paths):
    raise ValueError("unexpected symlink in benchmark index")
seed_bytes = sum(path.stat().st_size for path in seed_paths if path.is_file())
if shutil.disk_usage(work).free < minimum_free + 2 * seed_bytes * runs:
    raise ValueError("insufficient free space for full-build samples and the 2 GiB reserve")
derived = store / "derived"
shards = sorted((store / "facts").iterdir())
shard = max(shards, key=lambda path: path.stat().st_size)
if len(shards) != 64 or shard.stat().st_size <= 256:
    raise ValueError("benchmark needs a nonempty fact shard")
expected_derived, expected_shard = digest(derived), digest(shard)
unaffected = {path.name: state(path) for path in shards if path != shard}
observations = []

def recover():
    if shutil.disk_usage(work).free < minimum_free + 2 * seed_bytes:
        raise ValueError("insufficient free space for recovery and the 2 GiB reserve")
    flip_payload(shard)
    flip_payload(derived)
    _, elapsed = invoke(work / "recovery-index")
    if digest(derived) != expected_derived or digest(shard) != expected_shard:
        raise ValueError("recovered content differs from the seed index")
    if {path.name: state(path) for path in shards if path != shard} != unaffected:
        raise ValueError("recovery changed an unaffected shard")
    return elapsed

def full(number):
    if shutil.disk_usage(work).free < minimum_free + 2 * seed_bytes:
        raise ValueError("insufficient free space for a full build and the 2 GiB reserve")
    fresh, elapsed = invoke(work / f"full-{number}")
    if digest(fresh / "derived") != expected_derived:
        raise ValueError("full build differs from the seed index")
    return elapsed

for number in range(runs):
    order = (("recovery", lambda: recover()), ("full", lambda: full(number)))
    if number % 2:
        order = tuple(reversed(order))
    for name, measure in order:
        load = os.getloadavg()
        observations.append({"kind": name, "wall_ms": measure(), "load": load})

def summary(kind):
    values = [item["wall_ms"] for item in observations if item["kind"] == kind]
    return {"median_ms": statistics.median(values), "max_ms": max(values)}

report = {"snapshot_metadata_sha256": digest(source.parent / "snapshot.json"),
          "binary_sha256": digest(binary), "runs_per_kind": runs, "seed_ms": seed_ms,
          "fact_shard_bytes": shard.stat().st_size, "derived_bytes": derived.stat().st_size,
          "unaffected_shards": len(unaffected), "recovery": summary("recovery"),
          "full": summary("full"), "observations": observations}
with (work / "results.json").open("x") as stream:
    json.dump(report, stream, sort_keys=True)
    stream.write("\n")
print(json.dumps({key: value for key, value in report.items() if key != "observations"}, sort_keys=True))
PYTHON
env HOME="${WORK}/home" TMPDIR="${WORK}/tmp" XDG_CONFIG_HOME="${WORK}/home/config" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" --verify "$1" >/dev/null
echo "evidence: ${WORK}"
( set -o noclobber; printf 'xcalygraph-cache-v1\n' > "${WORK}/.xcalygraph-disposable" )
