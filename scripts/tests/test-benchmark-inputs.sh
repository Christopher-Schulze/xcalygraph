#!/usr/bin/env bash
# Exercise real benchmark input validation without starting Cargo or an agent.
# Usage: scripts/tests/test-benchmark-inputs.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-benchmark-inputs.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="benchmark-inputs.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, temporary_directory, tools = (work / name for name in ("project", "home", "tmp", "tools"))
    for directory in (root, home, temporary_directory, tools):
        directory.mkdir()
    shell = shutil.which("bash")
    assert shell
    # Forward to real utilities; Cargo is absent so a regression cannot start a build.
    for name in ("dirname", "basename", "awk"):
        executable = shutil.which(name)
        assert executable, name
        (tools / name).write_text("#!" + shell + "\nexec " + shlex.quote(executable) + ' "$@"\n')
        (tools / name).chmod(0o755)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(temporary_directory), XDG_CONFIG_HOME=str(home / "config"),
                       PATH=str(tools))
    assert shutil.which("cargo", path=environment["PATH"]) is None
    for relative in ("scripts/benchmarks/run-benchmarks.sh", "scripts/benchmarks/run-evaluation.sh"):
        destination = root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(repo / relative, destination)
    benchmark = root / "scripts/benchmarks/run-benchmarks.sh"
    evaluation = root / "scripts/benchmarks/run-evaluation.sh"
    public = root / "research/corpora/hono"
    public.mkdir(parents=True)
    private = work / "private corpus"
    private.mkdir()
    private_list = root / "scripts/benchmarks/questions/private/corpora.tsv"
    private_list.parent.mkdir(parents=True)
    private_list.write_text(f"private-name\t{private}\nabsent\t{work / 'missing'}\n")
    questions = root / "scripts/benchmarks/questions/xcalygraph.tsv"
    questions.write_text("question\texpected\n")

    def invoke(script, *args):
        return subprocess.run([shell, str(script), *args], env=environment, capture_output=True, text=True)

    unknown = invoke(benchmark, "xcalygraph", "unknown-corpus")
    assert unknown.returncode != 0 and "unknown corpus" in unknown.stderr, unknown.stderr
    absent = invoke(benchmark, "absent")
    assert absent.returncode != 0 and "missing" in absent.stderr, absent.stderr
    for name in ("xcalygraph", "hono", "private-name"):
        accepted = invoke(benchmark, name)
        assert accepted.returncode != 0 and "hyperfine missing" in accepted.stderr, accepted.stderr
    default = invoke(benchmark)
    assert default.returncode != 0 and "hyperfine missing" in default.stderr, default.stderr
    assert "absent" in default.stderr
    empty = invoke(evaluation, "--no-upstream", "xcalygraph")
    assert empty.returncode != 0 and "no questions" in empty.stderr, empty.stderr
    missing = invoke(evaluation, "--no-upstream", "unknown-corpus")
    assert missing.returncode != 0 and "missing corpus or questions" in missing.stderr, missing.stderr
    questions.write_text("question\texpected\nFind Create\n")
    malformed = invoke(evaluation, "--no-upstream", "xcalygraph")
    assert malformed.returncode != 0 and "no expected location" in malformed.stderr, malformed.stderr
    questions.write_text("question\texpected\nFind Create\tmain.go::Create")
    accepted = invoke(evaluation, "--no-upstream", "xcalygraph")
    assert accepted.returncode != 0 and "jq missing" in accepted.stderr, accepted.stderr
    assert not (root / ".cache").exists()
    print("PASS: known, unknown and missing corpora, empty questions and final unterminated question before any build")
PYTHON
