#!/usr/bin/env bash
# Exercise release staging with real debug Cargo artifacts and source locks.
# Usage: scripts/tests/test-release-provenance.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-release-provenance.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="release-provenance.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, target, extracted = (work / name for name in ("project", "home", "target with spaces\nand quotes", "extracted"))
    dist = root / "dist"
    for directory in (root / "src", root / "scripts/build", root / "scripts/utils", home, target, dist, extracted):
        directory.mkdir(parents=True)
    (root / "Cargo.toml").write_text('[package]\nname="xcalygraph"\nversion="0.0.0"\nedition="2024"\n')
    (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "xcalygraph"\nversion = "0.0.0"\n')
    (root / ".gitignore").write_text('/dist/\n')
    (root / "LICENSE").write_text("fixture license\n")
    source = root / "src/main.rs"
    source.write_text('fn main() { println!("FIRST"); }\n')
    sentinel = dist / "foreign"
    sentinel.write_text("preserve\n")
    release = root / "scripts/build/build-release.sh"
    shutil.copyfile(repo / "scripts/build/build-release.sh", release)
    for name in ("verification-target.sh", "verification-source.sh", "verification-lock.sh", "guard-benchmark.sh",
                 "check-build-space.sh", "cargo-artifact.sh"):
        shutil.copyfile(repo / "scripts/utils" / name, root / "scripts/utils" / name)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    environment.update(HOME=str(home), CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE="true",
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    subprocess.run(["git", "init", "-q", str(root)], env=environment, check=True)
    rustc = subprocess.run(["rustc", "-vV"], cwd=root, env=environment, capture_output=True, text=True, check=True)
    triple = next(line.removeprefix("host: ") for line in rustc.stdout.splitlines() if line.startswith("host: "))
    platform = subprocess.run(["bash", "-c", 'source "$1"; platform "$2"', "platform", str(release), triple],
                              cwd=root, env=environment, capture_output=True, text=True, check=True).stdout.strip()
    name = f"xcalygraph-0.0.0-{platform}"
    guard = root / "scripts/utils/guard-benchmark.sh"

    def disk():
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        assert shutil.disk_usage(work).free > 2 * 1024**3 + 128 * 1024**2

    def direct_build():
        disk()
        result = subprocess.run(["cargo", "build", "-v", "--locked", "--bin", "xcalygraph", "--target", triple,
                                 "--message-format=json"], cwd=root, env=environment, capture_output=True, check=True)
        selected = subprocess.run(["bash", str(root / "scripts/utils/cargo-artifact.sh")], cwd=root, env=environment,
                                  input=result.stdout, capture_output=True, check=True)
        return Path(os.fsdecode(selected.stdout).removesuffix("\n")), result.stderr.decode()

    first, _ = direct_build()
    assert subprocess.check_output([str(first)], text=True).strip() == "FIRST"
    before = source.stat()
    source.write_text('fn main() { println!("OTHER"); }\n')
    os.utime(source, ns=(before.st_atime_ns, before.st_mtime_ns))
    stale, log = direct_build()
    assert stale == first and "Fresh xcalygraph" in log
    assert subprocess.check_output([str(stale)], text=True).strip() == "FIRST"

    def stage_command(stage, suffix="", version="0.0.0"):
        script = 'source "$1"; shift; stage_releases "$@"' + suffix
        return ["bash", str(guard), "bash", "-c", script, "stage", str(release), str(stage), version, "--debug", triple]

    def stage_run(stage, suffix="", version="0.0.0"):
        stage.mkdir()
        disk()
        return subprocess.run(stage_command(stage, suffix, version), cwd=root, env=environment, capture_output=True, text=True)

    unguarded_stage = work / "unguarded-stage"
    unguarded_stage.mkdir()
    unguarded = subprocess.run(["bash", "-c", 'source "$1"; stage_releases "$2" 0.0.0 --debug "$3"',
                                "stage", str(release), str(unguarded_stage), triple], cwd=root, env=environment,
                               capture_output=True, text=True)
    assert unguarded.returncode != 0 and "verification lock is required" in unguarded.stderr
    assert not list(unguarded_stage.iterdir())
    checksum_stage, checksum_dest = work / "checksum-stage", work / "checksum-dest"
    checksum_stage.mkdir()
    checksum_dest.mkdir()
    subprocess.run(["tar", "-czf", str(checksum_stage / "valid.tar.gz"), "-C", str(root), "LICENSE"], check=True)
    checksum_failure = subprocess.run(["bash", "-c", 'source "$1"; if publish_releases "$2" "$3" valid.tar.gz missing.tar.gz; then exit 90; else exit 1; fi',
                                       "checksum", str(release), str(checksum_stage), str(checksum_dest)],
                                      cwd=root, env=environment, capture_output=True, text=True)
    assert checksum_failure.returncode != 0 and "missing.tar.gz" in checksum_failure.stderr
    assert not list(checksum_dest.iterdir())

    staged = work / "staged"
    result = stage_run(staged)
    assert result.returncode == 0, result.stderr
    archive = staged / (name + ".tar.gz")
    subprocess.run(["tar", "-xzf", str(archive), "-C", str(extracted)], check=True)
    assert subprocess.check_output([str(extracted / name / "xcalygraph")], text=True).strip() == "OTHER"
    assert sentinel.read_text() == "preserve\n" and list(dist.iterdir()) == [sentinel]

    mismatch = stage_run(work / "wrong-version", version="9.9.9")
    assert mismatch.returncode != 0 and "version changed before build" in mismatch.stderr
    assert sentinel.read_text() == "preserve\n" and list(dist.iterdir()) == [sentinel]
    interrupted = stage_run(work / "interrupted", suffix="; exit 7")
    assert interrupted.returncode != 0 and (work / "interrupted" / (name + ".tar.gz")).is_file()
    assert list(dist.iterdir()) == [sentinel]
    changed = stage_run(work / "changed", suffix='; printf "// changed\\n" >> src/main.rs')
    assert changed.returncode != 0 and "verification inputs changed during the gate" in changed.stderr
    assert (work / "changed" / (name + ".tar.gz")).is_file() and list(dist.iterdir()) == [sentinel]

    held_stage, waiting_stage = work / "held-stage", work / "waiting-stage"
    held_stage.mkdir()
    waiting_stage.mkdir()
    held, release_lock = work / "held", work / "release"
    suffix = '; touch "$HELD"; while [[ ! -e "$RELEASE" ]]; do sleep 0.01; done'
    concurrent_env = dict(environment, HELD=str(held), RELEASE=str(release_lock))
    disk()
    first_process = subprocess.Popen(stage_command(held_stage, suffix), cwd=root, env=concurrent_env,
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        deadline = time.monotonic() + 30
        while not held.exists() and time.monotonic() < deadline:
            if first_process.poll() is not None:
                raise RuntimeError(first_process.stderr.read().decode())
            time.sleep(0.01)
        assert held.exists(), "timed out waiting for guarded stage"
        disk()
        second_process = subprocess.Popen(stage_command(waiting_stage), cwd=root, env=environment,
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            time.sleep(0.2)
            assert second_process.poll() is None and not list(waiting_stage.iterdir())
            release_lock.touch()
            assert first_process.wait(timeout=30) == 0
            assert second_process.wait(timeout=30) == 0 and (waiting_stage / (name + ".tar.gz")).is_file()
        finally:
            if second_process.poll() is None:
                second_process.kill()
                second_process.wait()
    finally:
        release_lock.touch(exist_ok=True)
        if first_process.poll() is None:
            first_process.kill()
            first_process.wait()
    assert sentinel.read_text() == "preserve\n" and list(dist.iterdir()) == [sentinel]
    license_file, saved_license = root / "LICENSE", root / "LICENSE.saved"
    original_license = license_file.read_bytes()
    assert not saved_license.exists()
    license_file.rename(saved_license)
    assert not license_file.exists() and saved_license.read_bytes() == original_license
    try:
        disk()
        missing_license = subprocess.run(["bash", "-c", 'source "$1"; build_archives --debug "$2"',
                                          "build", str(release), triple], cwd=root, env=environment,
                                         capture_output=True, text=True)
        assert missing_license.returncode != 0 and "LICENSE" in missing_license.stderr
        assert list(dist.iterdir()) == [sentinel] and not list(dist.glob(".xcalygraph-release.*"))
    finally:
        saved_license.rename(license_file)
    assert not saved_license.exists() and license_file.read_bytes() == original_license
    build_script = root / "build.rs"
    generated = root / "new-build-input"
    build_script.write_text('use std::io::Write;\nfn main() -> std::io::Result<()> {\n'
                            'let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open("new-build-input")?;\n'
                            'file.write_all(b"new input\\n")?;\nOk(())\n}\n')
    disk()
    changed_build = subprocess.run(["bash", "-c", 'source "$1"; build_archives --debug "$2"',
                                    "build", str(release), triple], cwd=root, env=environment,
                                   capture_output=True, text=True)
    assert changed_build.returncode != 0 and "verification inputs changed during the gate" in changed_build.stderr
    assert generated.read_text() == "new input\n" and list(dist.iterdir()) == [sentinel]
    assert not list(dist.glob(".xcalygraph-release.*"))
    generated.unlink()
    build_script.unlink()
    disk()
    complete = subprocess.run(["bash", "-c", 'source "$1"; build_archives --debug "$2"',
                               "build", str(release), triple], cwd=root, env=environment, capture_output=True, text=True)
    assert complete.returncode == 0, complete.stderr
    published = dist / (name + ".tar.gz")
    assert published.is_file() and sentinel.read_text() == "preserve\n"
    checksum = hashlib.sha256(published.read_bytes()).hexdigest()
    assert (dist / "SHA256SUMS").read_text() == f"{checksum}  {published.name}\n"
    assert not list(dist.glob(".xcalygraph-release.*"))
    published_extracted = work / "published-extracted"
    published_extracted.mkdir()
    subprocess.run(["tar", "-xzf", str(published), "-C", str(published_extracted)], check=True)
    assert subprocess.check_output([str(published_extracted / name / "xcalygraph")], text=True).strip() == "OTHER"
    conflict = subprocess.run(["bash", "-c", 'source "$1"; build_archives --debug "$2"',
                               "build", str(release), triple], cwd=root, env=environment, capture_output=True, text=True)
    assert conflict.returncode != 0 and "output already exists" in conflict.stderr
    assert hashlib.sha256(published.read_bytes()).hexdigest() == checksum and sentinel.read_text() == "preserve\n"
    print("PASS: stale Cargo reproduction, guarded target-triple archive, interrupted and concurrent stages, safe local output")
PYTHON
