#!/usr/bin/env bash
# Verify license collection and debug packaging with real local Cargo dependencies.
# Usage: scripts/tests/test-release-licenses.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-release-licenses.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="release-licenses.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, target = work / "project", work / "home", work / "target"
    for directory in (root / "src", root / "scripts/build", root / "scripts/utils", home, target):
        directory.mkdir(parents=True)
    script = root / "scripts/build/build-release.sh"
    shutil.copyfile(repo / "scripts/build/build-release.sh", script)
    for name in ("verification-target.sh", "verification-source.sh", "verification-lock.sh", "guard-benchmark.sh",
                 "check-build-space.sh", "cargo-artifact.sh"):
        shutil.copyfile(repo / "scripts/utils" / name, root / "scripts/utils" / name)
    (root / "src/main.rs").write_text("fn main() {}\n")
    (root / "LICENSE").write_text("PROJECT LICENSE\n")
    dependencies = {
        "line_dep": "deps/line\n",
        "tab_dep": "deps/tab\tpart",
        "back_dep": "deps/back\\slash",
        "ordinary_dep": "deps/ordinary",
    }
    manifest = '[package]\nname="xcalygraph"\nversion="0.0.0"\nedition="2024"\n[dependencies]\n'
    for name, relative in dependencies.items():
        directory = root / relative
        (directory / "src").mkdir(parents=True)
        (directory / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion="0.0.0"\nedition="2024"\nlicense="MIT"\n')
        (directory / "src/lib.rs").write_text("pub fn fixture() {}\n")
        manifest += f'{name}={{path={json.dumps(relative)}}}\n'
    (root / "Cargo.toml").write_text(manifest)
    licenses = {
        root / dependencies["line_dep"] / "LICENSE\n": b"LINE ACTUAL\n",
        root / dependencies["line_dep"] / "NOTICE\tactual": b"SECOND ACTUAL\n",
        root / dependencies["tab_dep"] / "COPYING\tactual": b"TAB ACTUAL\n",
        root / dependencies["back_dep"] / "NOTICE\\actual": b"BACKSLASH ACTUAL\n",
        root / dependencies["ordinary_dep"] / "LiCeNsE-ordinary": b"ORDINARY ACTUAL\n",
    }
    for path, content in licenses.items():
        path.write_bytes(content)
    for relative in ("deps/line\\n", "deps/tab\\tpart", "deps/back\\\\slash"):
        decoy = root / relative
        decoy.mkdir()
        (decoy / "LICENSE").write_text("DECOY LICENSE\n")
    secret = root / "secret-license"
    secret.write_text("SYMLINK TARGET\n")
    link = root / dependencies["ordinary_dep"] / "LICENSE-link"
    link.symlink_to("../../secret-license")
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    environment.update(HOME=str(home), CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE="true",
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    subprocess.run(["git", "init", "-q", str(root)], env=environment, check=True)
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, env=environment, capture_output=True, check=True)
    metadata = subprocess.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
                              cwd=root, env=environment, capture_output=True, check=True)
    packages = {package["name"]: package for package in json.loads(metadata.stdout)["packages"]}
    for name, relative in dependencies.items():
        assert packages[name]["manifest_path"] == str(root / relative / "Cargo.toml")

    def collect():
        return subprocess.run(["bash", "-c", 'source "$1"; third_party', "licenses", str(script)],
                              cwd=root, env=environment, capture_output=True)

    def verify(content):
        for expected in licenses.values():
            assert content.count(expected) == 1, (expected, content)
        assert b"---- LICENSE\n\nLINE ACTUAL\n" in content
        assert b"---- NOTICE\tactual\nSECOND ACTUAL\n" in content
        assert b"---- COPYING\tactual\nTAB ACTUAL\n" in content
        assert b"---- NOTICE\\actual\nBACKSLASH ACTUAL\n" in content
        assert b"DECOY LICENSE" not in content and b"SYMLINK TARGET" not in content

    result = collect()
    assert result.returncode == 0, result.stderr
    verify(result.stdout)
    assert link.is_symlink() and os.readlink(link) == "../../secret-license"
    assert secret.read_text() == "SYMLINK TARGET\n"

    lock, saved_lock = root / "Cargo.lock", root / "Cargo.lock.saved"
    lock.rename(saved_lock)
    try:
        missing = collect()
        assert missing.returncode != 0
    finally:
        saved_lock.rename(lock)
    permission_checks = 0
    lock.chmod(0)
    try:
        try:
            lock.read_bytes()
        except PermissionError:
            assert collect().returncode != 0
            permission_checks += 1
    finally:
        lock.chmod(0o644)
    unreadable = root / dependencies["ordinary_dep"] / "LiCeNsE-ordinary"
    unreadable.chmod(0)
    try:
        try:
            unreadable.read_bytes()
        except PermissionError:
            assert collect().returncode != 0
            permission_checks += 1
    finally:
        unreadable.chmod(0o644)
    assert all(path.read_bytes() == content for path, content in licenses.items())

    legal = root / dependencies["line_dep"] / "legal"
    legal.mkdir()
    nested = legal / "custom\n"
    shared = root / "deps/SHARED-LICENSE"
    absolute = root / "ABS-LICENSE"
    declared_files = {nested: b"NESTED DECLARED\n", shared: b"PARENT DECLARED\n", absolute: b"ABSOLUTE DECLARED\n"}
    for path, content in declared_files.items():
        path.write_bytes(content)
    declarations = {
        "line_dep": "legal/custom\n",
        "tab_dep": "./COPYING\tactual",
        "back_dep": "../SHARED-LICENSE",
        "ordinary_dep": str(absolute),
    }
    for name, declaration in declarations.items():
        dependency_manifest = root / dependencies[name] / "Cargo.toml"
        original = dependency_manifest.read_text()
        assert original.count('license="MIT"') == 1
        dependency_manifest.write_text(original.replace('license="MIT"', "license-file=" + json.dumps(declaration)))
    metadata = subprocess.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
                              cwd=root, env=environment, capture_output=True, check=True)
    packages = {package["name"]: package for package in json.loads(metadata.stdout)["packages"]}
    assert all(packages[name]["license_file"] == value for name, value in declarations.items())

    def verify_declared(content):
        verify(content)
        for expected in declared_files.values():
            assert content.count(expected) == 1, (expected, content)
        assert b"---- custom\n\nNESTED DECLARED\n" in content
        assert os.fsencode(root) not in content

    declared = collect()
    assert declared.returncode == 0, declared.stderr
    verify_declared(declared.stdout)
    line_manifest = root / dependencies["line_dep"] / "Cargo.toml"
    original_manifest = line_manifest.read_text()
    named_license = "license-file=" + json.dumps(declarations["line_dep"])
    assert original_manifest.count(named_license) == 1
    line_manifest.write_text(original_manifest.replace(named_license, 'license-file=""'))
    try:
        assert collect().returncode != 0
    finally:
        line_manifest.write_text(original_manifest)
    saved_nested = legal / "saved-custom"
    nested.rename(saved_nested)
    try:
        assert collect().returncode != 0
        nested.symlink_to(saved_nested.name)
        try:
            assert collect().returncode != 0
        finally:
            nested.unlink()
    finally:
        saved_nested.rename(nested)
    saved_legal = legal.with_name("legal-saved")
    legal.rename(saved_legal)
    try:
        legal.symlink_to(saved_legal.name, target_is_directory=True)
        try:
            assert collect().returncode != 0
        finally:
            legal.unlink()
    finally:
        saved_legal.rename(legal)
    nested.chmod(0)
    try:
        try:
            nested.read_bytes()
        except PermissionError:
            assert collect().returncode != 0
            permission_checks += 1
    finally:
        nested.chmod(0o644)
    assert all(path.read_bytes() == content for path, content in declared_files.items())

    # The source guard rejects all project symlinks; test collection above before staging.
    saved_link = work / "saved-link"
    link.rename(saved_link)
    subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
    assert shutil.disk_usage(work).free > 2 * 1024**3 + 128 * 1024**2
    rustc = subprocess.run(["rustc", "-vV"], cwd=root, env=environment, capture_output=True, text=True, check=True)
    triple = next(line.removeprefix("host: ") for line in rustc.stdout.splitlines() if line.startswith("host: "))
    platform = subprocess.run(["bash", "-c", 'source "$1"; platform "$2"', "platform", str(script), triple],
                              cwd=root, env=environment, capture_output=True, text=True, check=True).stdout.strip()
    stage = work / "stage"
    stage.mkdir()
    guard = root / "scripts/utils/guard-benchmark.sh"
    staged = subprocess.run(["bash", str(guard), "bash", "-c", 'source "$1"; shift; stage_releases "$@"',
                             "stage", str(script), str(stage), "0.0.0", "--debug", triple],
                            cwd=root, env=environment, capture_output=True)
    assert staged.returncode == 0, staged.stderr
    name = f"xcalygraph-0.0.0-{platform}"
    bundled = subprocess.run(["tar", "-xOzf", str(stage / (name + ".tar.gz")),
                              name + "/THIRD-PARTY-LICENSES.txt"], capture_output=True, check=True)
    verify_declared(bundled.stdout)
    saved_link.rename(link)
    assert all(path.read_bytes() == content for path, content in licenses.items()) and link.is_symlink()
    print("PASS: exact dependency and license paths, decoys, symlinks, failures and guarded debug archive;"
          f" permission checks: {permission_checks}/3")
PYTHON
