#!/usr/bin/env bash
# Retain source- and sampler-bound precision samples for all pinned roots.
# Usage: scripts/benchmarks/sample-precision.sh
# Output stays local under .cache/task192-manifests/; each run is exclusive and retained with .keep.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
readonly SNAPSHOTS="${REPO_ROOT}/.cache/corpus-snapshots"
readonly WORK_BASE="${REPO_ROOT}/.cache/task192-manifests"

if (( $# != 0 )); then
  echo "usage: scripts/benchmarks/sample-precision.sh" >&2
  exit 2
fi
for tool in cargo git python3; do
  command -v "${tool}" >/dev/null || {
    echo "sample-precision.sh: ${tool} is required" >&2
    exit 1
  }
done
if [[ -L "${REPO_ROOT}/.cache" || -L "${SNAPSHOTS}" || -L "${WORK_BASE}" ]]; then
  echo "sample-precision.sh: evidence cache must not be a symlink" >&2
  exit 1
fi

if ! bash "${REPO_ROOT}/scripts/utils/verification-lock.sh"; then
  exec bash "${REPO_ROOT}/scripts/utils/guard-benchmark.sh" \
    bash "${REPO_ROOT}/scripts/benchmarks/sample-precision.sh" "$@"
fi

bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
mkdir -p "${WORK_BASE}"
WORK="$(mktemp -d "${WORK_BASE}/run.XXXXXXXX")"
readonly WORK
readonly BENCHMARK_HOME="${WORK}/home"
mkdir -p "${BENCHMARK_HOME}" "${WORK}/corpora" "${WORK}/logs"
( set -o noclobber; : > "${WORK}/roots.tsv" )

declare -a CORPUS_NAMES=()
declare -a CORPUS_REVISIONS=()
declare -a CORPUS_HASHES=()
declare -a SNAPSHOT_SOURCES=()

add_root() {
  local name="$1" source_root="$2" expected_revision="$3" allow_dirty="$4"
  local revision status snapshot snapshot_hash
  [[ -d "${source_root}" ]] || {
    echo "sample-precision.sh: source root missing for ${name}" >&2
    return 1
  }
  revision="$(git -C "${source_root}" rev-parse --verify HEAD)"
  [[ "${revision}" == "${expected_revision}" ]] || {
    echo "sample-precision.sh: ${name} revision ${revision} differs from pinned ${expected_revision}" >&2
    return 1
  }
  status="$(git -C "${source_root}" status --porcelain --untracked-files=normal)"
  if [[ "${allow_dirty}" != 1 && -n "${status}" ]]; then
    echo "sample-precision.sh: ${name} has uncommitted source changes" >&2
    return 1
  fi
  snapshot="$(env HOME="${BENCHMARK_HOME}" bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" \
    "${source_root}" "${SNAPSHOTS}")"
  bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" --verify "${snapshot}" >/dev/null
  snapshot_hash="$(python3 -I - "${snapshot}/../snapshot.json" <<'PYTHON'
import json
import sys

print(json.load(open(sys.argv[1], encoding="utf-8"))["source_sha256"])
PYTHON
)"
  CORPUS_NAMES+=("${name}")
  CORPUS_REVISIONS+=("${revision}")
  CORPUS_HASHES+=("${snapshot_hash}")
  SNAPSHOT_SOURCES+=("${snapshot}")
  printf '%s\t%s\t%s\t%s\n' "${name}" "${revision}" "${snapshot_hash}" "${snapshot}" \
    >> "${WORK}/roots.tsv"
}

xcalygraph_revision="$(git -C "${REPO_ROOT}" rev-parse --verify HEAD)"
add_root xcalygraph "${REPO_ROOT}" "${xcalygraph_revision}" 1
readonly XCALYGRAPH_SOURCE_SHA256="${CORPUS_HASHES[0]}"

readonly GRAPHIFY_ROOT="${REPO_ROOT}/research/upstream/graphify"
graphify_pin="$(python3 -I - "${REPO_ROOT}/docs/documentation.md" <<'PYTHON'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
match = re.search(r"^\| Analysis commit \| `([0-9a-f]{40})`", text, re.MULTILINE)
if match is None:
    raise SystemExit("sample-precision.sh: pinned Graphify commit is missing from documentation")
print(match.group(1))
PYTHON
)"
add_root graphify "${GRAPHIFY_ROOT}" "${graphify_pin}" 0

while IFS=$'\t' read -r name url commit languages; do
  [[ "${name}" == name ]] && continue
  [[ -n "${name}" && -n "${url}" && -n "${commit}" && -n "${languages}" ]] || {
    echo "sample-precision.sh: malformed row in scripts/benchmarks/corpora.tsv" >&2
    exit 1
  }
  add_root "${name}" "${REPO_ROOT}/research/corpora/${name}" "${commit}" 0
done < "${REPO_ROOT}/scripts/benchmarks/corpora.tsv"

(( ${#CORPUS_NAMES[@]} == 50 )) || {
  echo "sample-precision.sh: expected 50 configured roots, found ${#CORPUS_NAMES[@]}" >&2
  exit 1
}

sampler_source_sha256="$(python3 -I - "${REPO_ROOT}" <<'PYTHON'
import hashlib
import os
from pathlib import Path
import stat
import sys

root = Path(sys.argv[1])
paths = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "docs/documentation.md",
    "docs/spec.md",
    "scripts/benchmarks/corpora.tsv",
    "scripts/benchmarks/sample-precision.sh",
    "src/resolve/tests.rs",
)
digest = hashlib.sha256()
for relative in paths:
    path = root / relative
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode):
        raise SystemExit("sample-precision.sh: sampler identity input is not a regular file: " + relative)
    data = path.read_bytes()
    digest.update(len(os.fsencode(relative)).to_bytes(8, "big"))
    digest.update(os.fsencode(relative))
    digest.update(stat.S_IMODE(info.st_mode).to_bytes(4, "big"))
    digest.update(len(data).to_bytes(8, "big"))
    digest.update(data)
print(digest.hexdigest())
PYTHON
)"
python3 -I - "${REPO_ROOT}/scripts/benchmarks/corpora.tsv" "${WORK}/corpora.tsv" <<'PYTHON'
import os
from pathlib import Path
import sys

with Path(sys.argv[2]).open("xb") as destination:
    destination.write(Path(sys.argv[1]).read_bytes())
    destination.flush()
    os.fsync(destination.fileno())
PYTHON

bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
cargo test --locked --release --no-run --bin xcalygraph --message-format=json \
  > "${WORK}/cargo-build.jsonl"
sampler_binary="$(python3 -I - "${WORK}/cargo-build.jsonl" <<'PYTHON'
import json
import sys

artifacts = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8")]
executables = [item["executable"] for item in artifacts
               if item.get("reason") == "compiler-artifact"
               and item["target"].get("name") == "xcalygraph"
               and "bin" in item["target"].get("kind", [])
               and item.get("profile", {}).get("test")
               and item.get("executable")]
if len(executables) != 1:
    raise SystemExit("sample-precision.sh: Cargo did not report exactly one test binary")
print(executables[0])
PYTHON
)"
sampler_binary_sha256="$(python3 -I - "${sampler_binary}" <<'PYTHON'
import hashlib
import sys

digest = hashlib.sha256()
with open(sys.argv[1], "rb") as source:
    for chunk in iter(lambda: source.read(1024 * 1024), b""):
        digest.update(chunk)
print(digest.hexdigest())
PYTHON
)"

for ((index = 0; index < ${#CORPUS_NAMES[@]}; index++)); do
  name="${CORPUS_NAMES[index]}"
  source="${SNAPSHOT_SOURCES[index]}"
  output="${WORK}/corpora/${name}.jsonl"
  bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" --verify "${source}" >/dev/null
  env \
    XCALYGRAPH_SAMPLE_ROOT="${source}" \
    XCALYGRAPH_SAMPLE_CORPUS="${name}" \
    XCALYGRAPH_SAMPLE_REVISION="${CORPUS_REVISIONS[index]}" \
    XCALYGRAPH_SAMPLE_CORPUS_SHA256="${CORPUS_HASHES[index]}" \
    XCALYGRAPH_SAMPLE_XCALYGRAPH_SHA256="${XCALYGRAPH_SOURCE_SHA256}" \
    XCALYGRAPH_SAMPLE_SOURCE_SHA256="${sampler_source_sha256}" \
    XCALYGRAPH_SAMPLE_BINARY_SHA256="${sampler_binary_sha256}" \
    XCALYGRAPH_SAMPLE_SIZE=50 \
    XCALYGRAPH_SAMPLE_OUTPUT="${output}" \
    "${sampler_binary}" sample_edges --ignored --nocapture \
    > "${WORK}/logs/${name}.log" 2>&1
done

python3 -I - "${WORK}" <<'PYTHON'
import csv
import json
import os
from pathlib import Path
import re
import sys

work = Path(sys.argv[1])
roots = list(csv.reader(open(work / "roots.tsv", encoding="utf-8"), delimiter="\t"))
manifest_path = work / "manifest.jsonl"
adjudication_path = work / "adjudication.tsv"
sample_ids = set()
stratum_count = 0
sample_count = 0
identity_fields = (
    "corpus_snapshot_sha256", "sampler_binary_sha256", "xcalygraph_source_sha256",
    "sampler_source_sha256",
)
digest_pattern = re.compile(r"[0-9a-f]{64}")
required_sample = (
    "sample_id", "corpus", "corpus_revision", "corpus_snapshot_sha256", "source_language",
    "edge_kind", "confidence", "source_graph_node_id", "source_path", "source_line", "source_name",
    "source_signature_sha256", "source_node_kind", "source_signature_status", "source_text_sha256",
    "source_text_length_bytes", "source_text_truncated", "target_graph_node_id",
    "target_language", "target_path",
    "target_line", "target_name", "target_signature_sha256", "target_node_kind",
    "target_signature_status", "adjudication_status",
)

with manifest_path.open("xb") as manifest, adjudication_path.open("x", encoding="utf-8", newline="") as adjudication:
    rows = csv.writer(adjudication, delimiter="\t", lineterminator="\n")
    rows.writerow(("sample_id", "corpus", "corpus_revision", "corpus_snapshot_sha256",
                   "source_language", "edge_kind", "confidence", "source_path", "source_line",
                   "target_path", "target_line", "verdict"))
    for name, revision, source_hash, source_path in roots:
        records_path = work / "corpora" / (name + ".jsonl")
        active = None
        expected = None
        observed = 0
        run_count = 0
        run_identity = None
        seen_strata = set()
        local_strata = 0
        local_samples = 0
        for raw in records_path.open("rb"):
            record = json.loads(raw)
            kind = record.get("record_type")
            if kind == "run":
                run_count += 1
                if (run_count != 1 or active is not None or local_strata != 0
                        or record.get("corpus") != name or record.get("corpus_revision") != revision
                        or record.get("corpus_snapshot_sha256") != source_hash
                        or record.get("sample_limit") != 50
                        or record.get("schema") != "xcalygraph-language-precision-v1"
                        or any(not isinstance(record.get(field), str)
                               or not digest_pattern.fullmatch(record[field])
                               for field in identity_fields)):
                    raise SystemExit("sample-precision.sh: run identity mismatch for " + name)
                run_identity = tuple(record[field] for field in identity_fields)
            elif kind == "stratum":
                if active is not None and observed != expected:
                    raise SystemExit("sample-precision.sh: incomplete prior stratum in " + name)
                if (run_count != 1 or record.get("corpus") != name
                        or record.get("corpus_revision") != revision
                        or record.get("corpus_snapshot_sha256") != source_hash
                        or record.get("sample_limit") != 50
                        or any(not isinstance(record.get(field), str)
                               or not digest_pattern.fullmatch(record[field])
                               for field in identity_fields)
                        or tuple(record[field] for field in identity_fields) != run_identity):
                    raise SystemExit("sample-precision.sh: invalid stratum identity for " + name)
                population = record.get("population")
                expected = record.get("sample_count")
                if (not isinstance(population, int) or isinstance(population, bool)
                        or not isinstance(expected, int) or isinstance(expected, bool)
                        or population <= 0):
                    raise SystemExit("sample-precision.sh: invalid population denominator for " + name)
                if expected != min(population, 50):
                    raise SystemExit("sample-precision.sh: sample cap mismatch for " + name)
                population_exhausted = record.get("population_exhausted")
                if (not isinstance(population_exhausted, int)
                        or isinstance(population_exhausted, bool)
                        or population_exhausted != int(population <= 50)):
                    raise SystemExit("sample-precision.sh: exhausted population marker mismatch for " + name)
                active = (record.get("source_language"), record.get("edge_kind"), record.get("confidence"))
                if any(not isinstance(value, str) or not value for value in active):
                    raise SystemExit("sample-precision.sh: missing source stratum identity for " + name)
                if active in seen_strata:
                    raise SystemExit("sample-precision.sh: duplicate stratum in " + name)
                seen_strata.add(active)
                observed = 0
                local_strata += 1
            elif kind == "sample":
                if active is None:
                    raise SystemExit("sample-precision.sh: sample appears before its stratum in " + name)
                if any(field not in record for field in required_sample):
                    raise SystemExit("sample-precision.sh: sample is missing required provenance in " + name)
                sample_id = record["sample_id"]
                if (not isinstance(sample_id, str) or not digest_pattern.fullmatch(sample_id)
                        or sample_id in sample_ids):
                    raise SystemExit("sample-precision.sh: invalid or duplicate sample ID in " + name)
                if (record.get("corpus") != name
                        or record.get("corpus_revision") != revision
                        or record.get("corpus_snapshot_sha256") != source_hash
                        or (record.get("source_language"), record.get("edge_kind"), record.get("confidence")) != active
                        or record.get("adjudication_status") != "pending"
                        or tuple(record.get(field) for field in identity_fields) != run_identity):
                    raise SystemExit("sample-precision.sh: sample escaped its source stratum in " + name)
                for field in ("source_path", "target_path"):
                    path = Path(record[field])
                    if path.is_absolute() or ".." in path.parts:
                        raise SystemExit("sample-precision.sh: sample path is not corpus-relative in " + name)
                if record["source_text_length_bytes"] > 0 and "source_text" not in record:
                    raise SystemExit("sample-precision.sh: non-empty source line has no text preview in " + name)
                for prefix in ("source", "target"):
                    node_kind = record[prefix + "_node_kind"]
                    status = record[prefix + "_signature_status"]
                    if node_kind not in ("file", "symbol"):
                        raise SystemExit("sample-precision.sh: invalid " + prefix + " node kind in " + name)
                    if status not in ("not-applicable", "empty", "present"):
                        raise SystemExit("sample-precision.sh: invalid " + prefix + " signature status in " + name)
                    if (status == "not-applicable") != (node_kind == "file"):
                        raise SystemExit("sample-precision.sh: " + prefix + " signature status disagrees with node kind in " + name)
                    if status == "present" and not record.get(prefix + "_signature"):
                        raise SystemExit("sample-precision.sh: non-empty " + prefix + " signature is missing in " + name)
                for field in ("source_text_sha256", "source_signature_sha256", "target_signature_sha256"):
                    if not digest_pattern.fullmatch(record[field]):
                        raise SystemExit("sample-precision.sh: invalid evidence digest in " + name)
                for field in ("source_graph_node_id", "target_graph_node_id", "source_line",
                              "target_line", "source_text_length_bytes"):
                    value = record[field]
                    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                        raise SystemExit("sample-precision.sh: invalid evidence location or size in " + name)
                if record["source_line"] == 0 or record["target_line"] == 0:
                    raise SystemExit("sample-precision.sh: source line numbers must be one-based in " + name)
                if (not isinstance(record["source_text_truncated"], int)
                        or isinstance(record["source_text_truncated"], bool)
                        or record["source_text_truncated"] not in (0, 1)):
                    raise SystemExit("sample-precision.sh: invalid source preview truncation marker in " + name)
                sample_ids.add(sample_id)
                observed += 1
                local_samples += 1
                rows.writerow((sample_id, name, revision, source_hash, *active,
                               record["source_path"], record["source_line"], record["target_path"],
                               record["target_line"], "pending"))
            else:
                raise SystemExit("sample-precision.sh: unknown manifest record in " + name)
            manifest.write(raw)
        if run_count != 1 or active is not None and observed != expected:
            raise SystemExit("sample-precision.sh: incomplete manifest for " + name)
        stratum_count += local_strata
        sample_count += local_samples
    manifest.flush()
    os.fsync(manifest.fileno())
    adjudication.flush()
    os.fsync(adjudication.fileno())

if len(roots) != 50 or not sample_ids:
    raise SystemExit("sample-precision.sh: incomplete 50-root run")
print(f"roots={len(roots)} strata={stratum_count} samples={sample_count}")
PYTHON

( set -o noclobber; printf 'xcalygraph-cache-v1\n' > "${WORK}/.keep" )
echo "manifest: ${WORK}/manifest.jsonl"
echo "adjudication: ${WORK}/adjudication.tsv"
