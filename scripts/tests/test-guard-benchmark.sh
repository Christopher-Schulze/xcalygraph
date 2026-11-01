#!/usr/bin/env bash
# Exercise benchmark target isolation and inherited verification locks with real Cargo.
# Usage: scripts/tests/test-guard-benchmark.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-guard-benchmark.sh" >&2; exit 2; }
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
import time

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="guard-benchmark.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home = work / "project with spaces\nand quotes", work / "home"
    for directory in (root / "src", root / "scripts/utils", root / "build output", home):
        directory.mkdir(parents=True)
    (root / "Cargo.toml").write_text('[package]\nname="xcalygraph"\nversion="0.0.0"\nedition="2024"\n')
    (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "xcalygraph"\nversion = "0.0.0"\n')
    source = root / "src/main.rs"
    source.write_text('fn main() { println!("FIRST"); }\n')
    sentinel = root / "build output/foreign"
    sentinel.write_text("preserve\n")
    for name in ("verification-target.sh", "verification-source.sh", "verification-lock.sh", "guard-benchmark.sh"):
        shutil.copyfile(repo / "scripts/utils" / name, root / "scripts/utils" / name)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    environment.update(HOME=str(home), TMPDIR=str(work), XDG_CONFIG_HOME=str(home / "config"),
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       CARGO_TARGET_DIR="build output", CARGO_NET_OFFLINE="true",
                       GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1")
    subprocess.run(["git", "init", "-q", str(root)], env=environment, check=True)
    guard = root / "scripts/utils/guard-benchmark.sh"
    lock = root / "scripts/utils/verification-lock.sh"
    fake = subprocess.run(["bash", str(lock)], cwd=root,
                          env=dict(environment, XCALYGRAPH_VERIFICATION_FD="999"), capture_output=True)
    assert fake.returncode != 0

    def guarded(*args):
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        assert shutil.disk_usage(root).free > 2 * 1024**3 + 128 * 1024**2
        return subprocess.run(["bash", str(guard), *args], cwd=root, env=environment,
                              capture_output=True, text=True)

    command = 'set -e; exec 3>"$1"; bash "$2"; cargo build --locked -q; "$CARGO_TARGET_DIR/debug/xcalygraph"'
    first = guarded("bash", "-c", command, "benchmark", str(work / "descriptor-3"), str(lock))
    assert first.returncode == 0 and first.stdout.strip() == "FIRST", first.stderr
    before = source.stat()
    source.write_text('fn main() { println!("OTHER"); }\n')
    os.utime(source, ns=(before.st_atime_ns, before.st_mtime_ns))
    second = guarded("bash", "-c", command, "benchmark", str(work / "descriptor-3"), str(lock))
    assert second.returncode == 0 and second.stdout.strip() == "OTHER", second.stderr
    assert sentinel.read_text() == "preserve\n"

    def wait_for(path):
        deadline = time.monotonic() + 10
        while not path.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        assert path.exists(), f"timed out waiting for {path.name}"

    held, release, ran = (work / name for name in ("held", "release", "ran"))
    wait_command = 'touch "$1"; while [[ ! -e "$2" ]]; do sleep 0.01; done'
    first_process = subprocess.Popen(["bash", str(guard), "bash", "-c", wait_command, "first",
                                      str(held), str(release)], cwd=root, env=environment,
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        wait_for(held)
        second_process = subprocess.Popen(["bash", str(guard), "touch", str(ran)], cwd=root, env=environment,
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            time.sleep(0.2)
            assert second_process.poll() is None and not ran.exists()
            release.touch()
            assert first_process.wait(timeout=10) == 0
            assert second_process.wait(timeout=10) == 0 and ran.exists()
        finally:
            if second_process.poll() is None:
                second_process.kill()
                second_process.wait()
    finally:
        release.touch(exist_ok=True)
        if first_process.poll() is None:
            first_process.kill()
            first_process.wait()
    assert sentinel.read_text() == "preserve\n"
    print("PASS: owned in-repository target, restored-mtime rebuild, inherited lock and serialization")
PYTHON
