#!/usr/bin/env bash
# Verify snapshot identity and Graphify reuse using real source files and tools.
# Usage: scripts/tests/test-corpus-snapshots.sh
# Requires git, python3, graphify and a development xcalygraph binary.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
mkdir -p "${REPO_ROOT}/.cache"
work="$(mktemp -d "${REPO_ROOT}/.cache/test-corpus.XXXXXXXX")"
readonly work
trap 'rm -rf "${work}"' EXIT
python3 -I - "${REPO_ROOT}" "${work}" <<'PYTHON'
import json
import os
from pathlib import Path
import subprocess
import sys

repo, work = map(Path, sys.argv[1:])
root, cache = work / "root with spaces", work / "snapshots"
root.mkdir()
home = work / "home"
home.mkdir()
environment = dict(os.environ, HOME=str(home))
helper = repo / "scripts/utils/prepare-corpus.sh"
(root / "main.go").write_text("package sample\nfunc Earlier() {}\nfunc Entry() { Earlier() }\n")
(root / ".gitignore").write_text("ignored/\n")
(root / "ignored").mkdir()
(root / "ignored/private.go").write_text("package ignored\n")
subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True, env=environment)
subprocess.run(["git", "-C", str(root), "add", "."], check=True, env=environment)


def prepare(upstream=False, succeeds=True):
    arguments = ["bash", str(helper), str(root), str(cache)]
    if upstream:
        arguments.append("--graphify")
    result = subprocess.run(arguments, env=environment, capture_output=True, text=True)
    assert (result.returncode == 0) == succeeds, result.stderr
    return Path(result.stdout.strip()) if succeeds else result.stderr


first = prepare()
metadata = first.parent / "snapshot.json"
before = metadata.stat()
assert not (first / "ignored/private.go").exists()
assert prepare() == first
assert metadata.stat().st_ino == before.st_ino and metadata.stat().st_mtime_ns == before.st_mtime_ns

# These exact additions were absent from the previously reused comparison copy.
(root / "routes.rs").write_text("pub fn classify() {}\n")
(root / "terms.rs").write_text("pub fn without_class_lists() {}\n")
added = prepare()
assert added != first and (added / "routes.rs").exists() and (added / "terms.rs").exists()
assert not (first / "routes.rs").exists()
(root / "terms.rs").rename(root / "words.rs")
renamed = prepare()
assert renamed != added and not (renamed / "terms.rs").exists() and (renamed / "words.rs").exists()
(root / "routes.rs").unlink()
deleted = prepare()
assert deleted != renamed and not (deleted / "routes.rs").exists()
(root / "main.go").write_text("package sample\nfunc Later() {}\nfunc Entry() { Later() }\n")
changed = prepare()
assert changed != deleted and "Earlier" in (first / "main.go").read_text()

graph_source = prepare(upstream=True)
graph = graph_source / "graphify-out/graph.json"
graph_before = graph.stat()
assert prepare(upstream=True) == graph_source
assert graph.stat().st_ino == graph_before.st_ino and graph.stat().st_mtime_ns == graph_before.st_mtime_ns
content = json.loads(graph.read_text())
assert any("Later" in node.get("label", "") for node in content["nodes"])
assert not any("Earlier" in node.get("label", "") for node in content["nodes"])
binary = repo / "target/debug/xcalygraph"
index = work / "index"
result = subprocess.run([str(binary), "show", "Later", "-C", str(graph_source), "--index-dir", str(index)],
                        env=environment, capture_output=True, text=True, check=True)
assert "Later" in result.stdout and "main.go" in result.stdout

# A damaged cache is rejected and preserved, never silently accepted or overwritten.
original = graph.read_bytes()
graph.write_bytes(original + b" ")
assert "identity mismatch" in prepare(upstream=True, succeeds=False)
assert graph.read_bytes() == original + b" "
(changed / "unexpected.go").write_text("package sample\n")
assert "file set changed" in prepare(succeeds=False)
assert (changed / "unexpected.go").exists()
unsupported = work / "unsupported"
unsupported.mkdir()
(unsupported / "data.bin").write_bytes(b"\x00\x01\x02")
plain = subprocess.run(["bash", str(helper), str(unsupported), str(cache)],
                       env=environment, capture_output=True, text=True, check=True)
assert (Path(plain.stdout.strip()) / "data.bin").read_bytes() == b"\x00\x01\x02"
result = subprocess.run(["bash", str(helper), str(unsupported), str(cache), "--graphify"],
                        env=environment, capture_output=True, text=True)
assert result.returncode != 0 and not result.stdout, result.stdout
failed_snapshots = [p.parent for p in cache.glob("*/*/snapshot.json")
                    if json.loads(p.read_text())["upstream"] is not None
                    and [f["path"] for f in json.loads(p.read_text())["source"]["files"]] == ["data.bin"]]
assert len(failed_snapshots) == 1 and (failed_snapshots[0] / "graphify.log").exists()
assert not (failed_snapshots[0] / "graph.json.identity").exists()
print("PASS: reuse, add/remove/rename/edit invalidation, isolated HOME, real Graphify/xcalygraph agreement, corrupt graph/extra-file rejection and unsupported corpus failure")
PYTHON
