#!/usr/bin/env bash
# Exercise relative private corpus selection through the real source guard.
# Usage: scripts/tests/test-evaluation-relative-corpus.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-evaluation-relative-corpus.sh" >&2; exit 2; }
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
with tempfile.TemporaryDirectory(prefix="evaluation-relative-corpus.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, caller, home, target = work / "project", work / "caller\n\n", work / "home", work / "target"
    for directory in (root / "src", root / "scripts/benchmarks/questions/private", root / "scripts/utils",
                      caller / "relative", home, target):
        directory.mkdir(parents=True)
    (root / "Cargo.toml").write_text('[package]\nname="xcalygraph"\nversion="0.0.0"\nedition="2024"\n')
    (root / "src/main.rs").write_text(
        '#[cfg(debug_assertions)]\ncompile_error!("evaluation reached debug build");\n'
        '#[cfg(not(debug_assertions))]\ncompile_error!("evaluation reached release build");\n')
    (root / "scripts/benchmarks/questions/private/corpora.tsv").write_text("fixture\trelative\n")
    (root / "scripts/benchmarks/questions/private/fixture.tsv").write_text("question\texpected\nFind Create\tmain.go::Create\n")
    script = root / "scripts/benchmarks/run-evaluation.sh"
    shutil.copyfile(repo / "scripts/benchmarks/run-evaluation.sh", script)
    for name in ("read-private-corpus.sh", "guard-benchmark.sh", "verification-target.sh", "verification-source.sh",
                 "verification-lock.sh", "check-build-space.sh", "cargo-artifact.sh"):
        shutil.copyfile(repo / "scripts/utils" / name, root / "scripts/utils" / name)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    environment.update(HOME=str(home), CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE="true",
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    subprocess.run(["git", "init", "-q", str(root)], env=environment, check=True)
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, env=environment, capture_output=True, check=True)
    subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
    assert shutil.disk_usage(work).free > 2 * 1024**3 + 128 * 1024**2
    result = subprocess.run(["bash", str(script), "--no-upstream", "fixture"], cwd=caller, env=environment, capture_output=True)
    assert result.returncode != 0 and b"error: evaluation reached release build" in result.stderr, result.stderr
    assert b"error: evaluation reached debug build" not in result.stderr, result.stderr
    assert b"missing corpus or questions" not in result.stderr
    assert not (root / "relative").exists() and (caller / "relative").is_dir()
    print("PASS: real source guard preserves the caller-relative corpus and selects the release compiler profile")
PYTHON
