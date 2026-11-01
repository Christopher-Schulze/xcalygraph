#!/usr/bin/env bash
# Exercise real benchmark planning and setup without invoking an agent.
# Usage: scripts/tests/test-agent-benchmark-safety.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-agent-benchmark-safety.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="agent-benchmark-safety.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    home, tmp = work / "home", work / "tmp"
    home.mkdir()
    tmp.mkdir()
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "CLAUDE_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), XDG_CONFIG_HOME=str(home / "config"))

    def project(name):
        root = work / name
        runner = root / "scripts/benchmarks/run-agent-benchmark.sh"
        runner.parent.mkdir(parents=True)
        shutil.copyfile(repo / "scripts/benchmarks/run-agent-benchmark.sh", runner)
        questions = root / "scripts/benchmarks/questions/xcalygraph.tsv"
        questions.parent.mkdir()
        questions.write_text("question\texpected\nFind Create\tmain.go::Create\n")
        return root, runner

    root, runner = project("plan")
    plan = subprocess.run(["bash", str(runner), "--runs", "1", "xcalygraph"],
                          env=environment, capture_output=True, text=True)
    assert plan.returncode == 0 and "claude -p" in plan.stdout, plan.stderr
    assert not (root / ".cache").exists(), "planning wrote a cache"

    root, runner = project("preserve")
    base = root / ".cache/agent-benchmark"
    old_home = base / "home-none"
    old_home.mkdir(parents=True)
    sentinel = old_home / "sentinel"
    sentinel.write_text("keep prior home\n")
    old_results = base / "results.tsv"
    old_results.write_text("keep prior results\n")
    setup = r'''
set -euo pipefail
source "$1" xcalygraph >/dev/null
first="$(create_run)"
second="$(create_run)"
home1="$(prepare_home none "$first")"
home2="$(prepare_home none "$first")"
printf '%s\n' "$first" "$second" "$home1" "$home2"
'''
    prepared = subprocess.run(["bash", "-c", setup, "setup", str(runner)],
                              env=environment, capture_output=True, text=True)
    assert prepared.returncode == 0, prepared.stderr
    first, second, home1, home2 = map(Path, prepared.stdout.splitlines())
    assert first != second and first.parent == second.parent == base
    assert home1 != home2 and home1.parent == home2.parent == first
    assert (home1 / ".claude").is_dir() and (home2 / ".claude").is_dir()
    assert (first / "results.tsv").read_text().startswith("corpus\tarm\trun\t")
    assert (second / "results.tsv").read_text().startswith("corpus\tarm\trun\t")
    assert sentinel.read_text() == "keep prior home\n" and old_results.read_text() == "keep prior results\n"

    for linked_part in ("cache", "category"):
        root, runner = project("linked-" + linked_part)
        foreign = work / ("foreign-" + linked_part)
        foreign.mkdir()
        if linked_part == "cache":
            (root / ".cache").symlink_to(foreign, target_is_directory=True)
        else:
            (root / ".cache").mkdir()
            (root / ".cache/agent-benchmark").symlink_to(foreign, target_is_directory=True)
        rejected = subprocess.run(["bash", "-c", 'source "$1" xcalygraph >/dev/null; create_run',
                                   "setup", str(runner)], env=environment, capture_output=True, text=True)
        assert rejected.returncode != 0 and "symlink" in rejected.stderr
        assert not list(foreign.iterdir())
    print("PASS: read-only planning, exclusive run and home setup, previous evidence and linked-cache refusal")
PYTHON
