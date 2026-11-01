#!/usr/bin/env bash
# Exercise real disk checks and Cargo target discovery without starting a build.
# Usage: scripts/tests/test-build-space.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-build-space.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="build-space.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, tmp, tools = (work / name for name in ("project", "home", "tmp", "tools"))
    for directory in (root, home, tmp, tools):
        directory.mkdir()
    shell = shutil.which("bash")
    assert shell
    for name in ("bash", "dirname", "awk", "graphify", "python3", "df", "du", "mkdir", "mktemp", "tee", "env"):
        executable = shutil.which(name)
        assert executable, name
        (tools / name).write_text("#!" + shell + "\nexec " + shlex.quote(executable) + ' "$@"\n')
        (tools / name).chmod(0o755)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), XDG_CONFIG_HOME=str(home / "config"), LC_ALL="C")
    restricted = dict(environment, PATH=str(tools))
    assert shutil.which("cargo", path=restricted["PATH"]) is None
    for relative in ("scripts/benchmarks/compare-depth.sh", "scripts/utils/check-build-space.sh"):
        source = repo / relative
        destination = root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        if source.exists():
            shutil.copyfile(source, destination)
    benchmark = root / "scripts/benchmarks/compare-depth.sh"
    (root / "research/corpora/fixture").mkdir(parents=True)
    failed = subprocess.run([shell, str(benchmark), "fixture"], cwd=root, env=restricted,
                            capture_output=True, text=True)
    assert failed.returncode != 0 and "Filesystem" in failed.stderr, failed.stderr
    assert not (root / ".cache").exists()

    (root / "Cargo.toml").write_text('[package]\nname = "xcalygraph"\nversion = "0.0.0"\nedition = "2024"\n')
    (root / "src").mkdir()
    (root / "src/main.rs").write_text('fn main() { println!("fixture"); }\n')
    (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "xcalygraph"\nversion = "0.0.0"\n')
    (root / ".cargo").mkdir()
    (root / ".cargo/config.toml").write_text('[build]\ntarget-dir = "configured-target"\n')
    target = root / "configured-target"
    target.mkdir()
    sentinel = target / "unrelated"
    sentinel.write_text("retain\n")
    real = dict(environment, RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")), CARGO_NET_OFFLINE="true")
    real.pop("CARGO_TARGET_DIR", None)
    check = root / "scripts/utils/check-build-space.sh"
    normal = subprocess.run([shell, str(check)], cwd=root, env=real, capture_output=True, text=True)
    assert normal.returncode == 0, normal.stderr
    bad_build = work / "build-is-a-file"
    bad_build.write_text("preserve\n")
    build_environment = dict(real, CARGO_BUILD_BUILD_DIR=str(bad_build))
    invalid_build = subprocess.run([shell, str(check)], cwd=root, env=build_environment, capture_output=True, text=True)
    assert invalid_build.returncode != 0 and "build" in invalid_build.stderr and "not a directory" in invalid_build.stderr, invalid_build.stderr
    assert bad_build.read_text() == "preserve\n" and sentinel.read_text() == "retain\n"
    template_environment = dict(real, CARGO_BUILD_BUILD_DIR=str(work / "build-{workspace-path-hash}"))
    template = subprocess.run([shell, str(check)], cwd=root, env=template_environment, capture_output=True, text=True)
    assert template.returncode == 0, template.stderr
    metadata = subprocess.run(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
                              cwd=root, env=template_environment, capture_output=True, text=True, check=True)
    assert "{workspace-path-hash}" not in json.loads(metadata.stdout)["build_directory"]
    opaque_build = work / "opaque-build"
    (opaque_build / "opaque").mkdir(parents=True)
    opaque = opaque_build / "opaque"
    opaque.chmod(0)
    try:
        if not os.access(opaque, os.R_OK | os.X_OK):
            opaque_environment = dict(real, CARGO_BUILD_BUILD_DIR=str(opaque_build))
            unreadable = subprocess.run([shell, str(check)], cwd=root, env=opaque_environment, capture_output=True, text=True)
            assert unreadable.returncode != 0 and "cannot measure build directory" in unreadable.stderr, unreadable.stderr
    finally:
        opaque.chmod(0o700)
    split_target, split_build = work / "split-target", work / "split-build with newline\n"
    for directory in (split_target, split_build):
        directory.mkdir()
        (directory / "CACHEDIR.TAG").write_text("Signature: 8a477f597d28d172789f06886806bc55\n")
        (directory / "unrelated").write_text("keep\n")
    split_environment = dict(real, CARGO_TARGET_DIR=str(split_target), CARGO_BUILD_BUILD_DIR=str(split_build))
    split_check = subprocess.run([shell, str(check)], cwd=root, env=split_environment, capture_output=True, text=True)
    assert split_check.returncode == 0, split_check.stderr
    subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
    assert shutil.disk_usage(root).free > 2 * 1024**3 + 128 * 1024**2
    subprocess.run(["cargo", "build", "--locked", "--bin", "xcalygraph"],
                   cwd=root, env=split_environment, capture_output=True, text=True, check=True)
    assert (split_target / "debug/xcalygraph").is_file() and (split_build / "debug").is_dir()
    intermediate_artifacts = list((split_build / "debug/deps").glob("xcalygraph-*"))
    assert intermediate_artifacts
    split_forced = subprocess.run([shell, str(check), str(shutil.disk_usage(split_target).free // 1024 + 8 * 1024 * 1024)],
                                  cwd=root, env=split_environment, capture_output=True, text=True)
    assert split_forced.returncode != 0 and "build aborted" in split_forced.stderr, split_forced.stderr
    assert "could not safely clean" not in split_forced.stderr, split_forced.stderr
    assert not (split_target / "debug/xcalygraph").exists()
    assert all(not path.exists() for path in intermediate_artifacts)
    assert all((directory / "unrelated").read_text() == "keep\n" for directory in (split_target, split_build))
    available_kib = shutil.disk_usage(target).free // 1024
    forced = subprocess.run([shell, str(check), str(available_kib + 8 * 1024 * 1024)],
                            cwd=root, env=real, capture_output=True, text=True)
    assert forced.returncode != 0 and "could not safely clean" in forced.stderr and str(target) in forced.stderr, forced.stderr
    assert sentinel.read_text() == "retain\n"
    (target / "CACHEDIR.TAG").write_text("Signature: 8a477f597d28d172789f06886806bc55\n")
    forced = subprocess.run([shell, str(check), str(available_kib + 8 * 1024 * 1024)],
                            cwd=root, env=real, capture_output=True, text=True)
    assert forced.returncode != 0 and "build aborted" in forced.stderr and str(target) in forced.stderr, forced.stderr
    assert "could not safely clean" not in forced.stderr, forced.stderr
    assert sentinel.read_text() == "retain\n"
    relative_target = root / "relative-target"
    relative_target.mkdir()
    relative_sentinel = relative_target / "unrelated"
    relative_sentinel.write_text("preserve\n")
    relative_environment = dict(real, CARGO_TARGET_DIR="relative-target")
    relative = subprocess.run([shell, str(check), str(available_kib + 8 * 1024 * 1024)],
                              cwd=root, env=relative_environment, capture_output=True, text=True)
    assert relative.returncode != 0 and str(relative_target) in relative.stderr, relative.stderr
    assert relative_sentinel.read_text() == "preserve\n" and sentinel.read_text() == "retain\n"
    newline_target = root / "target-with-newline\n"
    newline_target.mkdir()
    (newline_target / "unrelated").write_text("keep\n")
    newline_environment = dict(real, CARGO_TARGET_DIR=str(newline_target))
    newline = subprocess.run([shell, str(check), str(available_kib + 8 * 1024 * 1024)],
                             cwd=root, env=newline_environment, capture_output=True, text=True)
    assert newline.returncode != 0 and f"for {newline_target}; cleaning" in newline.stderr, newline.stderr
    assert (newline_target / "unrelated").read_text() == "keep\n"
    bad_target = work / "target-is-a-file"
    bad_target.write_text("keep\n")
    bad_environment = dict(real, CARGO_TARGET_DIR=str(bad_target))
    bad = subprocess.run([shell, str(check)], cwd=root, env=bad_environment, capture_output=True, text=True)
    assert bad.returncode != 0 and "not a directory" in bad.stderr, bad.stderr
    assert bad_target.read_text() == "keep\n" and sentinel.read_text() == "retain\n"
    print("PASS: real preflight, split build directories, unreadable paths, reserve failure and safe cleanup")
PYTHON
