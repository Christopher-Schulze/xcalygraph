#!/usr/bin/env bash
# Exercise benchmark setup from checkout roots ending in newlines, without builds or agents.
# Usage: scripts/tests/test-benchmark-checkout-root.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-benchmark-checkout-root.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
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
with tempfile.TemporaryDirectory(prefix="benchmark-checkout-root.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    home, tmp, tools, decoy, foreign = (work / name for name in ("home", "tmp", "tools", "project", "foreign"))
    for directory in (home, tmp, tools, decoy, foreign):
        directory.mkdir()
    sentinel = decoy / "sentinel"
    sentinel.write_text("preserve\n")
    shell = shutil.which("bash")
    assert shell
    for name in ("bash", "dirname", "basename", "awk", "seq", "python3", "graphify"):
        executable = shutil.which(name)
        assert executable, name
        (tools / name).symlink_to(executable)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), XDG_CONFIG_HOME=str(home / "config"), PATH=str(tools))
    assert shutil.which("cargo", path=environment["PATH"]) is None
    for count in (1, 2):
        root = work / ("project" + "\n" * count)
        benchmark_dir = root / "scripts/benchmarks"
        benchmark_dir.mkdir(parents=True)
        for name in ("run-agent-benchmark.sh", "run-benchmarks.sh", "measure-query-compute.sh", "compare-depth.sh"):
            shutil.copyfile(repo / "scripts/benchmarks" / name, benchmark_dir / name)
        graphify = root / "research/upstream/graphify"
        corpus = root / "research/corpora/example"
        graphify.mkdir(parents=True)
        corpus.mkdir(parents=True)
        questions = benchmark_dir / "questions"
        (questions / "private").mkdir(parents=True)
        (questions / "graphify.tsv").write_text("question\texpected\nFind Create\tmain.go::Create\n")
        (questions / "private/corpora.tsv").write_text("special\tmissing-corpus\n")
        (root / ".cache").symlink_to(foreign, target_is_directory=True)

        def invoke(name, *arguments):
            return subprocess.run([shell, str(benchmark_dir / name), *map(str, arguments)],
                                  cwd=work, env=environment, capture_output=True, text=True)

        plan = invoke("run-agent-benchmark.sh", "--runs", "1", "graphify")
        assert plan.returncode == 0 and "claude -p" in plan.stdout and str(graphify) in plan.stdout, plan.stderr
        missing = invoke("run-benchmarks.sh", "special")
        assert missing.returncode != 0 and "missing corpus special" in missing.stderr, missing.stderr
        query = invoke("measure-query-compute.sh", corpus, questions / "graphify.tsv", "1")
        assert query.returncode != 0 and "benchmark cache must not be a symlink" in query.stderr, query.stderr
        depth = invoke("compare-depth.sh", "example")
        assert depth.returncode != 0 and "depth cache must not be a symlink" in depth.stderr, depth.stderr
        assert sentinel.read_text() == "preserve\n" and list(decoy.iterdir()) == [sentinel]
        assert not list(foreign.iterdir())
    print("PASS: benchmark setup preserves newline-ending checkout roots, decoys and linked-cache refusal")
PYTHON
