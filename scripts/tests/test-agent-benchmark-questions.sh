#!/usr/bin/env bash
# Exercise question validation and scoring without running an agent or release build.
# Usage: scripts/tests/test-agent-benchmark-questions.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-agent-benchmark-questions.sh" >&2; exit 2; }
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
with tempfile.TemporaryDirectory(prefix="agent-benchmark-questions.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, tools, empty_path = work / "project", work / "home", work / "tools", work / "empty-path"
    for directory in (root / "scripts/benchmarks/questions", home, tools, empty_path):
        directory.mkdir(parents=True)
    runner = root / "scripts/benchmarks/run-agent-benchmark.sh"
    shutil.copyfile(repo / "scripts/benchmarks/run-agent-benchmark.sh", runner)
    shell = shutil.which("bash")
    assert shell
    for name in ("bash", "python3", "dirname", "seq", "grep", "jq", "mkdir", "mktemp"):
        executable = shutil.which(name)
        assert executable, name
        (tools / name).symlink_to(executable)
    canary = work / "agent-called"
    agent_canary = tools / "claude"
    agent_canary.write_text('#!/bin/sh\nprintf called > "$AGENT_CANARY"\nexit 99\n')
    agent_canary.chmod(0o755)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "CLAUDE_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), PATH=str(tools), AGENT_CANARY=str(canary), XDG_CONFIG_HOME=str(home / "config"))
    assert shutil.which("cargo", path=environment["PATH"]) is None
    question_file = root / "scripts/benchmarks/questions/xcalygraph.tsv"

    def invoke(*args):
        return subprocess.run([shell, str(runner), *args], cwd=root, env=environment, capture_output=True, timeout=15)

    invalid_files = (b"", b"question\texpected\n", b"question\texpected\nFind Create\n",
                     b"question\texpected\nFind Create\t\n", b"question\texpected\nFind Create\t::Create\n",
                     b"question\texpected\nFind Create\tmain.go::\n",
                     b"question\texpected\nFind Create\t   \n", b"question\texpected\n   \tmain.go\n",
                     b"question\texpected\nFind Create\tmain.go\textra\n",
                     b"question\texpected\nFind\x00 Create\tmain.go\n")
    for content in invalid_files:
        question_file.write_bytes(content)
        dry = invoke("--runs", "1", "xcalygraph")
        assert dry.returncode != 0 and not dry.stdout, (content, dry.stdout, dry.stderr)
        guarded = invoke("--run", "--runs", "1", "xcalygraph")
        assert guarded.returncode != 0 and not guarded.stdout, (content, guarded.stdout, guarded.stderr)
        assert not canary.exists() and not (root / ".cache").exists()

    question_file.write_bytes(b"question\texpected\nFind Create\tmain.go::Create")
    plan = invoke("--runs", "1", "xcalygraph")
    assert plan.returncode == 0 and plan.stdout.count(b"claude -p") == 3, plan.stderr
    repeated = invoke("--runs", "002", "xcalygraph")
    assert repeated.returncode == 0 and repeated.stdout.count(b"claude -p") == 6, repeated.stderr
    sequence, saved_sequence = tools / "seq", tools / "seq.saved"
    sequence.rename(saved_sequence)
    try:
        missing_sequence = invoke("--runs", "1", "xcalygraph")
        assert missing_sequence.returncode != 0 and not missing_sequence.stdout
        guarded_sequence = invoke("--run", "--runs", "1", "xcalygraph")
        assert guarded_sequence.returncode != 0 and not guarded_sequence.stdout
        assert not canary.exists() and not (root / ".cache").exists()
    finally:
        saved_sequence.rename(sequence)
    for value in ("0", "000", "-1", "abc", "", "1.5"):
        bad_count = invoke("--runs", value, "xcalygraph")
        assert bad_count.returncode != 0 and not bad_count.stdout, (value, bad_count.stdout, bad_count.stderr)
        guarded_count = invoke("--run", "--runs", value, "xcalygraph")
        assert guarded_count.returncode != 0 and not guarded_count.stdout
    missing_count = invoke("--runs")
    assert missing_count.returncode != 0 and not missing_count.stdout
    mixed = invoke("--runs", "1", "xcalygraph", "unknown-corpus")
    assert mixed.returncode != 0 and not mixed.stdout
    guarded_mixed = invoke("--run", "--runs", "1", "xcalygraph", "unknown-corpus")
    assert guarded_mixed.returncode != 0 and not guarded_mixed.stdout
    unknown = invoke("--runs", "1", "unknown-corpus")
    assert unknown.returncode != 0 and not unknown.stdout

    def score(path, answer, path_override=None):
        command = 'source "$1" xcalygraph >/dev/null; '
        if path_override is not None:
            command += 'PATH="$4"; '
        command += 'score_answer "$2" "$3"'
        return subprocess.run([shell, "-c", command, "score", str(runner), path, answer, str(path_override or tools)],
                              cwd=root, env=environment, capture_output=True)

    for path, answer, expected in (("main.go", "See main.go for the function.", b"1"),
                                   ("-entry.rs", "Inspect -entry.rs.", b"1"),
                                   ("-Afile.rs", "Inspect -Afile.rs.", b"1"),
                                   ("missing.rs", "No matching path here.", b"0")):
        result = score(path, answer)
        assert result.returncode == 0 and result.stdout == expected, (path, result.stdout, result.stderr)
    assert score("", "unrelated answer").returncode != 0
    assert score("   ", "unrelated answer").returncode != 0
    failed_matcher = score("main.go", "main.go", empty_path)
    assert failed_matcher.returncode != 0 and b"matcher" in failed_matcher.stderr
    assert not canary.exists() and not (root / ".cache").exists()
    print("PASS: invalid questions and run counts fail before work; final rows and literal scorer verified without agents")
PYTHON
