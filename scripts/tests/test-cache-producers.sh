#!/usr/bin/env bash
# Exercise a real completed benchmark, failed run and safe pruning in an isolated project.
# Usage: scripts/tests/test-cache-producers.sh [BINARY]
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
(( $# <= 1 )) || { echo "usage: test-cache-producers.sh [BINARY]" >&2; exit 2; }
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "$REPO_ROOT" "${1:-}" <<'PYTHON'
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
target = Path(os.environ.get("CARGO_TARGET_DIR", repo / "target"))
if not target.is_absolute():
    target = repo / target
binary = Path(sys.argv[2]).resolve(strict=True) if sys.argv[2] else (target / "debug/xcalygraph").resolve(strict=True)
with tempfile.TemporaryDirectory(prefix="cache-producers.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, tmp, source = (work / name for name in ("project", "home", "tmp", "source"))
    for directory in (root, home, tmp, source):
        directory.mkdir()
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), XDG_CONFIG_HOME=str(home / "config"),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       CARGO_TARGET_DIR=str(work / "target"), CARGO_NET_OFFLINE="true")
    for relative in ("scripts/utils/prepare-corpus.sh", "scripts/utils/prune-cache.sh",
                     "scripts/benchmarks/measure-index-validation.sh", "scripts/benchmarks/measure-query-compute.sh",
                     "scripts/benchmarks/measure-shard-recovery.sh"):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(repo / relative, path)
    functions = "".join(f"// Item {number} returns its identifier.\nfunc Item{number}() int {{ return {number} }}\n"
                        for number in range(60))
    (source / "main.go").write_text("package sample\nfunc Create() string { return \"ok\" }\n" + functions)
    prepared = subprocess.run(["bash", str(root / "scripts/utils/prepare-corpus.sh"), str(source),
                               str(root / ".cache/corpus-snapshots")],
                              env=environment, capture_output=True, text=True, check=True)
    snapshot = Path(prepared.stdout.strip())
    producer = root / "scripts/benchmarks/measure-index-validation.sh"
    benchmark_dir = root / ".cache/benchmarks"

    def run(after):
        return subprocess.run(["bash", str(producer), str(snapshot), str(binary), str(after), "1"],
                              env=environment, capture_output=True, text=True)

    def evidence(result):
        return Path(result.stdout.splitlines()[-1].removeprefix("evidence: "))

    first = run(binary)
    assert first.returncode == 0, first.stderr + first.stdout
    protected = evidence(first)
    assert (protected / ".xcalygraph-disposable").read_bytes() == b"xcalygraph-cache-v1\n"
    assert json.loads((protected / "results.json").read_text())["index_unchanged"]
    (protected / ".keep").touch(exist_ok=False)
    second = run(binary)
    assert second.returncode == 0, second.stderr + second.stdout
    disposable = evidence(second)
    assert (disposable / ".xcalygraph-disposable").read_bytes() == b"xcalygraph-cache-v1\n"
    prior = set(benchmark_dir.glob("index-validation.*"))
    failure = run(work / "missing-binary")
    assert failure.returncode != 0
    failed, = set(benchmark_dir.glob("index-validation.*")) - prior
    assert not (failed / ".xcalygraph-disposable").exists()
    questions = work / "questions.tsv"
    questions.write_text("Create\tmain.go::Create\n")
    (root / "Cargo.toml").write_text('[package]\nname = "no-target"\nversion = "0.0.0"\nedition = "2021"\n')
    query = subprocess.run(["bash", str(root / "scripts/benchmarks/measure-query-compute.sh"),
                            str(source), str(questions), "1"], env=environment, capture_output=True, text=True)
    assert query.returncode != 0 and "no targets specified" in query.stderr
    failed_query, = benchmark_dir.glob("query.*")
    assert not (failed_query / ".xcalygraph-disposable").exists()
    recovery = subprocess.run(["bash", str(root / "scripts/benchmarks/measure-shard-recovery.sh"),
                               str(snapshot), str(binary), "1"], env=environment, capture_output=True, text=True)
    assert recovery.returncode == 0, recovery.stderr + recovery.stdout
    recovered = evidence(recovery)
    assert (recovered / ".xcalygraph-disposable").read_bytes() == b"xcalygraph-cache-v1\n"
    assert json.loads((recovered / "results.json").read_text())["unaffected_shards"] == 63
    pruned = subprocess.run(["bash", str(root / "scripts/utils/prune-cache.sh"), "0"],
                            env=environment, capture_output=True, text=True)
    assert pruned.returncode == 0, pruned.stderr + pruned.stdout
    assert protected.is_dir() and not disposable.exists() and not recovered.exists()
    assert failed.is_dir() and failed_query.is_dir() and snapshot.is_dir()
    print("PASS: real completed and failed benchmark runs, .keep, snapshot retention and eligible pruning")
PYTHON
