#!/usr/bin/env bash
# Verify invalidation accounting with real files, syscalls and the real CLI.
# Usage: test-cold-build.sh BINARY
set -euo pipefail
(( $# == 1 )) || { echo "usage: test-cold-build.sh BINARY" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && /bin/pwd -P)"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" "$1" <<'PYTHON'
import os
from pathlib import Path
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
binary = Path(sys.argv[2]).resolve(strict=True)
runner = repo / "scripts/benchmarks/measure-cold-build.sh"
with tempfile.TemporaryDirectory(prefix="cold-build-test.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    source = work / "source"
    source.mkdir()
    (source / "main.rs").write_text("pub fn checked() {}\n")
    (source / "empty").touch()
    outside = work / "outside"
    outside.write_bytes(b"preserve")
    (source / "linked").symlink_to(outside)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH")}
    environment.update(HOME=str(work), TMPDIR=str(work), XDG_CONFIG_HOME=str(work / "config"),
                       COLD_INDEX_BASE=str(work), GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")

    def run(script):
        return subprocess.run(["bash", str(script), "--runs", "1", str(source), str(binary)],
                              cwd=work, env=environment, capture_output=True, text=True, timeout=60)

    original = runner.read_text()
    assert original.count("MS_INVALIDATE = 2") == 1
    invalid = work / "invalid-flags.sh"
    invalid.write_text(original.replace("MS_INVALIDATE = 2", "MS_INVALIDATE = -1", 1))
    failed = run(invalid)
    assert failed.returncode == 0, failed.stderr
    assert "invalidate_ok 0" in failed.stdout and "invalidate_failed 1" in failed.stdout, failed.stdout
    assert "errno=" in failed.stdout and "evicted 1" not in failed.stdout, failed.stdout
    normal = run(runner)
    assert normal.returncode == 0, normal.stderr
    assert "mapped 1" in normal.stdout and "map_failed 0" in normal.stdout, normal.stdout
    assert "unmap_failed 0" in normal.stdout, normal.stdout
    assert "cache_state unproven" in normal.stdout and "metadata stays cached" in normal.stdout, normal.stdout
    assert "evicted" not in normal.stdout, normal.stdout
    assert (source / "main.rs").read_text() == "pub fn checked() {}\n"
    assert outside.read_bytes() == b"preserve"
    assert not list(work.glob("cold-index.*"))
    print("PASS: real invalidation failures are counted; cache state remains unproven")
PYTHON
