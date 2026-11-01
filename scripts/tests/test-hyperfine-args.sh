#!/usr/bin/env bash
# Exercise real Hyperfine commands and prepare scripts with unusual paths, without a Cargo build.
# Usage: scripts/tests/test-hyperfine-args.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-hyperfine-args.sh" >&2; exit 2; }
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
with tempfile.TemporaryDirectory(prefix="hyperfine-args.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, tools = (work / name for name in ("project", "home", "tools"))
    for directory in (root, home, tools):
        directory.mkdir()
    shell = shutil.which("bash")
    assert shell
    for name in ("dirname", "python3", "hyperfine", "touch", "sh"):
        executable = shutil.which(name)
        assert executable, name
        (tools / name).write_text("#!" + shell + "\nexec " + shlex.quote(executable) + ' "$@"\n')
        (tools / name).chmod(0o755)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(work), XDG_CONFIG_HOME=str(home / "config"), PATH=str(tools))
    assert shutil.which("cargo", path=environment["PATH"]) is None
    script = root / "scripts/benchmarks/run-benchmarks.sh"
    script.parent.mkdir(parents=True)
    shutil.copyfile(repo / "scripts/benchmarks/run-benchmarks.sh", script)
    path = work / "space 'quote' $(touch INJECTED)\nü"
    path.mkdir()
    target = path / "source 'quote' $(touch INJECTED).txt"
    payload = "\n// quote ' safe\n"
    checks = r'''
set -euo pipefail
source "$1"
measured="$(hyperfine_command /bin/test -d "$2")"
hyperfine -N --warmup 0 --runs 1 "$measured" >/dev/null
prepare="$(hyperfine_append_command "$4" "$3")"
measured="$(hyperfine_command /bin/test -f "$3")"
hyperfine -N --warmup 0 --runs 1 --prepare "$prepare" "$measured" >/dev/null
'''
    result = subprocess.run([shell, "-c", checks, "hyperfine", str(script), str(path), str(target), payload],
                            cwd=work, env=environment, capture_output=True, text=True)
    assert result.returncode == 0, result.stderr + result.stdout
    assert target.read_text() == payload
    assert not (work / "INJECTED").exists()
    print("PASS: real Hyperfine preserves unusual measured arguments and safe prepare paths")
PYTHON
