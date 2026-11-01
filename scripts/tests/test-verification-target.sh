#!/usr/bin/env bash
# Verify checkout-specific Cargo targets using real isolated builds.
# Usage: scripts/tests/test-verification-target.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-verification-target.sh" >&2; exit 2; }
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
with tempfile.TemporaryDirectory(prefix="verification-target.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    home, tmp, shared = (work / name for name in ("home", "tmp", "shared"))
    for directory in (home, tmp, shared):
        directory.mkdir()
    foreign = shared / "unrelated"
    foreign.write_text("keep\n")
    foreign_build = work / "shared-intermediates"
    foreign_build.mkdir()
    foreign_build_sentinel = foreign_build / "sentinel"
    foreign_build_sentinel.write_text("preserve\n")
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), XDG_CONFIG_HOME=str(home / "config"),
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       CARGO_TARGET_DIR=str(shared), CARGO_BUILD_BUILD_DIR=str(foreign_build), CARGO_NET_OFFLINE="true")

    def project(name):
        root = work / name
        (root / "src").mkdir(parents=True)
        (root / "Cargo.toml").write_text('[package]\nname = "checkout-fixture"\nversion = "0.0.0"\nedition = "2024"\n')
        (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "checkout-fixture"\nversion = "0.0.0"\n')
        (root / "src/main.rs").write_text(f'fn main() {{ println!("{name}"); }}\n')
        helper = root / "scripts/utils/verification-target.sh"
        helper.parent.mkdir(parents=True)
        source = repo / "scripts/utils/verification-target.sh"
        if source.exists():
            shutil.copyfile(source, helper)
        return root, helper

    def resolve(root, helper, env=environment):
        return subprocess.run(["bash", str(helper)], cwd=root, env=env, capture_output=True, text=True)

    first, first_helper = project("first")
    second, second_helper = project("second")
    first_result = resolve(first, first_helper)
    second_result = resolve(second, second_helper)
    assert first_result.returncode == second_result.returncode == 0, first_result.stderr + second_result.stderr
    first_target, second_target = Path(first_result.stdout.strip()), Path(second_result.stdout.strip())
    assert first_target != second_target and first_target.is_dir() and second_target.is_dir()
    assert (first_target / "CACHEDIR.TAG").is_file() and (second_target / "CACHEDIR.TAG").is_file()
    assert resolve(first, first_helper).stdout.strip() == str(first_target)
    derived_environment = dict(environment, CARGO_TARGET_DIR=str(first_target))
    assert resolve(first, first_helper, derived_environment).stdout.strip() == str(first_target)
    alias = work / "alias"
    alias.symlink_to(first, target_is_directory=True)
    assert resolve(alias, alias / "scripts/utils/verification-target.sh").stdout.strip() == str(first_target)

    for iteration, (root, target, expected) in enumerate(((first, first_target, "first"),
                                                         (second, second_target, "second"),
                                                         (first, first_target, "first"))):
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        assert shutil.disk_usage(root).free > 2 * 1024**3 + 128 * 1024**2
        build_environment = dict(environment, CARGO_TARGET_DIR=str(target), CARGO_BUILD_BUILD_DIR=str(target))
        built = subprocess.run(["cargo", "build", "--locked", "--bin", "checkout-fixture"],
                               cwd=root, env=build_environment, capture_output=True, text=True, check=True)
        if iteration == 2:
            assert "Compiling checkout-fixture" not in built.stderr
        result = subprocess.run([str(target / "debug/checkout-fixture")], env=build_environment,
                                capture_output=True, text=True, check=True)
        assert result.stdout.strip() == expected

    blocked, blocked_helper = project("blocked")
    digest = hashlib.sha256(os.fsencode(blocked.resolve())).hexdigest()
    occupied = shared / ".xcalygraph-verification" / digest
    occupied.mkdir()
    sentinel = occupied / "sentinel"
    sentinel.write_text("preserve\n")
    rejected = resolve(blocked, blocked_helper)
    assert rejected.returncode != 0 and "ownership" in rejected.stderr, rejected.stderr
    assert sentinel.read_text() == "preserve\n" and foreign.read_text() == "keep\n"
    linked_base = work / "linked-base"
    linked_base.mkdir()
    (linked_base / ".xcalygraph-verification").symlink_to(shared, target_is_directory=True)
    rejected_link = resolve(first, first_helper, dict(environment, CARGO_TARGET_DIR=str(linked_base)))
    assert rejected_link.returncode != 0 and "symlink" in rejected_link.stderr, rejected_link.stderr
    assert foreign.read_text() == "keep\n" and foreign_build_sentinel.read_text() == "preserve\n"
    assert list(foreign_build.iterdir()) == [foreign_build_sentinel]
    print("PASS: real cross-checkout builds, reusable targets and preservation of unowned entries")
PYTHON
