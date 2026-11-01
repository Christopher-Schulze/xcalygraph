#!/usr/bin/env bash
# Exercise content-aware verification with real Cargo and exclusively owned fixtures.
# Usage: scripts/tests/test-verification-source.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-verification-source.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import fcntl
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="verification-source.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, tmp, targets = (work / name for name in ("project", "home", "tmp", "targets"))
    for directory in (root / "src", root / "dep/src", root / "scripts/utils", home, tmp, targets):
        directory.mkdir(parents=True)
    (root / ".gitignore").write_text("/target/\n")
    (root / "Cargo.toml").write_text('[package]\nname = "xcalygraph"\nversion = "0.0.0"\nedition = "2024"\n'
                                      '[dependencies]\nfixture-dep = { path = "dep" }\n')
    (root / "dep/Cargo.toml").write_text('[package]\nname = "fixture-dep"\nversion = "0.0.0"\nedition = "2024"\n')
    (root / "dep/src/lib.rs").write_text('pub fn value() -> &\'static str { "dependency" }\n')
    (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "fixture-dep"\nversion = "0.0.0"\n'
                                      '\n[[package]]\nname = "xcalygraph"\nversion = "0.0.0"\n'
                                      'dependencies = [\n "fixture-dep",\n]\n')
    source = root / "src/main.rs"
    first = ('fn value() -> &\'static str { "FIRST" }\n'
             'fn main() { println!("{} {}", value(), fixture_dep::value()); }\n'
             '#[test] fn reports_value() { println!("{}", value()); }\n')
    other = first.replace("FIRST", "OTHER")
    source.write_text(first)
    for name in ("verification-target.sh", "verification-source.sh"):
        shutil.copyfile(repo / "scripts/utils" / name, root / "scripts/utils" / name)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), XDG_CONFIG_HOME=str(home / "config"),
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       CARGO_TARGET_DIR=str(targets), CARGO_NET_OFFLINE="true",
                       GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1")
    subprocess.run(["git", "init", "-q", str(root)], check=True, env=environment)
    selected = subprocess.run(["bash", str(root / "scripts/utils/verification-target.sh")], cwd=root,
                              env=environment, capture_output=True, text=True, check=True)
    target = Path(selected.stdout.strip())
    environment.update(CARGO_TARGET_DIR=str(target), CARGO_BUILD_BUILD_DIR=str(target))
    sentinel = target / "unrelated"
    sentinel.write_text("preserve\n")

    def cargo(*arguments, guarded=False, check=True):
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        assert shutil.disk_usage(root).free > 2 * 1024**3 + 128 * 1024**2
        command = ["cargo", *arguments]
        if guarded:
            command = ["bash", str(root / "scripts/utils/verification-source.sh"), *command]
        return subprocess.run(command, cwd=root, env=environment, capture_output=True, text=True, check=check)

    def replace_preserving_mtime(content):
        before = source.stat()
        assert len(source.read_bytes()) == len(content.encode())
        source.write_text(content)
        os.utime(source, ns=(before.st_atime_ns, before.st_mtime_ns))
        assert source.stat().st_mtime_ns == before.st_mtime_ns

    def binary_output():
        return subprocess.run([str(target / "debug/xcalygraph")], env=environment,
                              capture_output=True, text=True, check=True).stdout.strip()

    cargo("build", "--locked")
    replace_preserving_mtime(other)
    stale = cargo("build", "--locked", "-v")
    assert "Fresh xcalygraph" in stale.stderr and binary_output() == "FIRST dependency"
    replace_preserving_mtime(first)
    initial = cargo("test", "--locked", "--", "--nocapture", guarded=True)
    assert "FIRST" in initial.stdout
    cargo("build", "--locked", guarded=True)
    dependency = next((target / "debug/deps").glob("libfixture_dep-*.rlib"))
    dependency_stat = dependency.stat()
    replace_preserving_mtime(other)
    stale_test = cargo("test", "--locked", "--", "--nocapture")
    assert "FIRST" in stale_test.stdout and "OTHER" not in stale_test.stdout
    updated = cargo("test", "--locked", "--", "--nocapture", guarded=True)
    assert "OTHER" in updated.stdout and "FIRST" not in updated.stdout
    cargo("build", "--locked", guarded=True)
    assert binary_output() == "OTHER dependency"
    assert dependency.stat().st_ino == dependency_stat.st_ino
    reused = cargo("build", "--locked", "-v", guarded=True)
    assert "Fresh xcalygraph" in reused.stderr

    failed = subprocess.run(["bash", str(root / "scripts/utils/verification-source.sh"),
                             "bash", "-c", "exit 7"], cwd=root, env=environment, capture_output=True, text=True)
    assert failed.returncode == 7
    recovered = cargo("build", "--locked", "-v", guarded=True)
    assert "Compiling xcalygraph" in recovered.stderr and binary_output() == "OTHER dependency"
    assert dependency.stat().st_ino == dependency_stat.st_ino

    safe = 'fn main() { let _value = std::env::args().next().unwrap_or_default(); }\n'
    unsafe = 'fn main() { let _value = std::env::args().next().unwrap()           ; }\n'
    assert len(safe) == len(unsafe)
    source.write_text(safe)
    cargo("clippy", "--locked", "--", "-D", "clippy::unwrap_used", guarded=True)
    replace_preserving_mtime(unsafe)
    stale_lint = cargo("clippy", "--locked", "--", "-D", "clippy::unwrap_used")
    assert stale_lint.returncode == 0
    caught = cargo("clippy", "--locked", "--", "-D", "clippy::unwrap_used", guarded=True, check=False)
    assert caught.returncode != 0 and "unwrap_used" in caught.stderr

    changed = subprocess.run(["bash", str(root / "scripts/utils/verification-source.sh"), "python3", "-c",
                              "from pathlib import Path; import sys; Path(sys.argv[1]).write_text(sys.argv[2])",
                              str(source), first], cwd=root, env=environment, capture_output=True, text=True)
    assert changed.returncode != 0 and "inputs changed during the gate" in changed.stderr
    cargo("build", "--locked", guarded=True)
    assert binary_output() == "FIRST dependency"

    state = target / ".xcalygraph-verification-source"
    with state.open("ab") as stream:
        stream.write(b"incomplete")
    recovered = cargo("build", "--locked", "-v", guarded=True)
    assert "Compiling xcalygraph" in recovered.stderr
    assert "Fresh xcalygraph" in cargo("build", "--locked", "-v", guarded=True).stderr

    def wait_for(path):
        deadline = time.monotonic() + 10
        while not path.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        assert path.exists(), f"timed out waiting for {path.name}"

    held, release, second_started, second_ran = (work / name for name in ("held", "release", "started", "ran"))
    helper = str(root / "scripts/utils/verification-source.sh")
    first_command = ("from pathlib import Path; import sys, time; Path(sys.argv[1]).touch(); "
                     "exec('while not Path(sys.argv[2]).exists():\\n time.sleep(0.01)')")
    first_process = subprocess.Popen(["bash", helper, "python3", "-c", first_command, str(held), str(release)],
                                     cwd=root, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        wait_for(held)
        with state.open("rb") as stream:
            try:
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                raise AssertionError("guard did not hold its lock while the command ran")
            except BlockingIOError:
                pass
        second_process = subprocess.Popen(["bash", "-c", 'touch "$1"; shift; exec "$@"', "guard-test",
                                           str(second_started), "bash", helper, "python3", "-c",
                                           "from pathlib import Path; import sys; Path(sys.argv[1]).touch()",
                                           str(second_ran)], cwd=root, env=environment,
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            wait_for(second_started)
            time.sleep(0.2)
            assert second_process.poll() is None and not second_ran.exists()
            release.touch()
            assert first_process.wait(timeout=10) == 0
            assert second_process.wait(timeout=10) == 0 and second_ran.exists()
        finally:
            if second_process.poll() is None:
                second_process.kill()
                second_process.wait()
    finally:
        release.touch(exist_ok=True)
        if first_process.poll() is None:
            first_process.kill()
            first_process.wait()

    state.unlink()
    foreign = work / "foreign"
    foreign.write_text("preserve\n")
    state.symlink_to(foreign)
    rejected = cargo("build", "--locked", guarded=True, check=False)
    assert rejected.returncode != 0 and "state ownership" in rejected.stderr
    assert foreign.read_text() == "preserve\n"
    assert sentinel.read_text() == "preserve\n"
    print("PASS: real Cargo freshness, reuse, interrupted recovery, concurrent gates and foreign state")
PYTHON
