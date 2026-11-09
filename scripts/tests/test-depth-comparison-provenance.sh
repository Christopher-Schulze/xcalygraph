#!/usr/bin/env bash
# Reproduce stale dump_edges output with a real debug Cargo fixture and preserved mtimes.
# Usage: scripts/tests/test-depth-comparison-provenance.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-depth-comparison-provenance.sh" >&2; exit 2; }
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

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="depth-comparison.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root = work / "fixture repo"
    home, temporary_directory = work / "home", work / "tmp"
    for directory in (root / "scripts/benchmarks", root / "scripts/utils", root / "src",
                      root / "research/corpora/fixture", home, temporary_directory):
        directory.mkdir(parents=True)

    (root / ".gitignore").write_text("/.cache/\n/target/\n")
    (root / "Cargo.toml").write_text(
        '[package]\nname = "xcalygraph"\nversion = "0.0.0"\nedition = "2024"\n')
    (root / "Cargo.lock").write_text(
        'version = 4\n\n[[package]]\nname = "xcalygraph"\nversion = "0.0.0"\n')
    shutil.copyfile(repo / "rust-toolchain.toml", root / "rust-toolchain.toml")
    (root / "research/corpora/fixture/main.py").write_text(
        "def answer():\n    return calculate()\n\ndef calculate():\n    return 42\n")
    source = root / "src/lib.rs"
    source.write_text('''use std::path::PathBuf;

const BUILD_MARKER: &str = "BEFORE";

#[test]
#[ignore = "isolated depth provenance fixture"]
fn dump_edges() -> Result<(), std::io::Error> {
    let root = std::env::var_os("XCALYGRAPH_DUMP_ROOT")
        .ok_or_else(|| std::io::Error::other("missing dump root"))?;
    let output = std::env::var_os("XCALYGRAPH_DUMP_FILE")
        .ok_or_else(|| std::io::Error::other("missing dump file"))?;
    if !PathBuf::from(root).is_dir() {
        return Err(std::io::Error::other("dump root is not a directory"));
    }
    let output = PathBuf::from(output);
    std::fs::write(&output, "calls\\t0\\tmain.py\\t1\\tmain.py\\t1\\tfixture\\n")?;
    std::fs::write(output.with_extension("marker"), BUILD_MARKER)?;
    if let Some(path) = std::env::var_os("DEPTH_MUTATE_SOURCE") {
        std::fs::write(path, "changed\\n")?;
    }
    Ok(())
}
''')
    (root / "src/guard-input.txt").write_text("before\n")

    runner_source = (repo / "scripts/benchmarks/compare-depth.sh").read_text()
    release_command = "cargo test -q --release --locked dump_edges -- --ignored >/dev/null"
    debug_command = "cargo test -q --locked dump_edges -- --ignored >/dev/null"
    assert runner_source.count(release_command) == 1
    runner_source = runner_source.replace(release_command, debug_command, 1)
    (root / "scripts/benchmarks/compare-depth.sh").write_text(runner_source)
    for name in ("output-writer.sh",):
        shutil.copyfile(repo / "scripts/benchmarks" / name, root / "scripts/benchmarks" / name)
    for name in ("check-build-space.sh", "prepare-corpus.sh", "guard-benchmark.sh",
                 "verification-lock.sh", "verification-source.sh", "verification-target.sh"):
        shutil.copyfile(repo / "scripts/utils" / name, root / "scripts/utils" / name)

    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("CARGO_", "RUSTUP_", "GIT_", "XCALYGRAPH_", "BASH_FUNC_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(temporary_directory), XDG_CONFIG_HOME=str(home / "config"),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_NET_OFFLINE="true", RUSTFLAGS="-Dwarnings",
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    subprocess.run(["git", "init", "-q", "-b", "main", str(root)], env=environment, check=True)
    subprocess.run(["git", "-C", str(root), "add", "."], env=environment, check=True)
    subprocess.run(["git", "-C", str(root), "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                    "commit", "-qm", "fixture"], env=environment, check=True)

    runner = root / "scripts/benchmarks/compare-depth.sh"

    def invoke_depth(extra_environment=None):
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        return subprocess.run(["bash", str(runner), "fixture", "1"], cwd=root,
                              env=dict(environment, **(extra_environment or {})),
                              capture_output=True, text=True, timeout=180)

    def run_depth():
        result = invoke_depth()
        assert result.returncode == 0, result.stderr + result.stdout
        evidence = next((line.removeprefix("evidence: ").split("; snapshot: ", 1)[0]
                         for line in (result.stdout + result.stderr).splitlines()
                         if line.startswith("evidence: ")), None)
        assert evidence is not None, result.stdout + result.stderr
        return Path(evidence)

    first_run = run_depth()
    assert (first_run / "edges.marker").read_text() == "BEFORE"

    before = source.stat()
    changed = source.read_text().replace('BUILD_MARKER: &str = "BEFORE"',
                                          'BUILD_MARKER: &str = "AFTER!"')
    assert changed != source.read_text() and len(changed) == len(source.read_text())
    source.write_text(changed)
    os.utime(source, ns=(before.st_atime_ns, before.st_mtime_ns))
    assert source.stat().st_mtime_ns == before.st_mtime_ns
    second_run = run_depth()
    assert (second_run / "edges.marker").read_text() == "AFTER!", \
        "depth runner reused stale dump_edges output after a same-mtime source edit"

    identity = hashlib.sha256(os.fsencode(root.resolve())).hexdigest()
    target = root / "target/.xcalygraph-verification" / identity
    assert (target / ".xcalygraph-verification-owner").read_text() == \
        "xcalygraph-verification-v1\n" + identity + "\n"
    assert (target / ".xcalygraph-verification-source").read_text().splitlines()[-1].startswith("ok ")

    failed = invoke_depth({"DEPTH_MUTATE_SOURCE": str(root / "src/guard-input.txt")})
    assert failed.returncode != 0 and "inputs changed during the gate" in failed.stderr, \
        failed.stderr + failed.stdout
    print("PASS: debug dump_edges rebuilds after preserved-mtime edits and the complete depth run checks its source")
PYTHON
