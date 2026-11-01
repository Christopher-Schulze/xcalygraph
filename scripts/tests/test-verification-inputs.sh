#!/usr/bin/env bash
# Verify that hidden build-script inputs remain protected by the source guard.
# Usage: scripts/tests/test-verification-inputs.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-verification-inputs.sh" >&2; exit 2; }
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
with tempfile.TemporaryDirectory(prefix="verification-inputs.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, targets = (work / name for name in ("project", "home", "targets"))
    for directory in (root / "src", root / "scripts/utils", home, targets):
        directory.mkdir(parents=True)
    (root / "Cargo.toml").write_text('[package]\nname="xcalygraph"\nversion="0.0.0"\nedition="2024"\n')
    (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "xcalygraph"\nversion = "0.0.0"\n')
    (root / "build.rs").write_text(
        'use std::{env, error::Error, fs, path::PathBuf};\n'
        'fn main() -> Result<(), Box<dyn Error>> {\n'
        '    let text = fs::read_to_string("README.md")?;\n'
        '    fs::write(PathBuf::from(env::var("OUT_DIR")?).join("value.txt"), text)?;\n'
        '    println!("cargo:rerun-if-changed=build.rs");\n'
        '    Ok(())\n}\n')
    (root / "src/main.rs").write_text(
        'fn main() { print!("{}", include_str!(concat!(env!("OUT_DIR"), "/value.txt"))); }\n')
    document = root / "README.md"
    document.write_text("FIRST")
    for name in ("verification-target.sh", "verification-source.sh"):
        shutil.copyfile(repo / "scripts/utils" / name, root / "scripts/utils" / name)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    environment.update(HOME=str(home), TMPDIR=str(work), XDG_CONFIG_HOME=str(home / "config"),
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       CARGO_TARGET_DIR=str(targets), CARGO_NET_OFFLINE="true",
                       GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1")
    subprocess.run(["git", "init", "-q", str(root)], env=environment, check=True)
    selected = subprocess.run(["bash", str(root / "scripts/utils/verification-target.sh")], cwd=root,
                              env=environment, capture_output=True, text=True, check=True)
    target = Path(selected.stdout.strip())
    environment.update(CARGO_TARGET_DIR=str(target), CARGO_BUILD_BUILD_DIR=str(target))
    helper = root / "scripts/utils/verification-source.sh"

    def build(guarded=True):
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        assert shutil.disk_usage(root).free > 2 * 1024**3 + 128 * 1024**2
        command = ["cargo", "build", "--locked", "-v"]
        if guarded:
            command = ["bash", str(helper), *command]
        result = subprocess.run(command, cwd=root, env=environment, capture_output=True, text=True)
        assert result.returncode == 0, result.stderr
        return result

    def output():
        return subprocess.run([str(target / "debug/xcalygraph")], cwd=root, env=environment,
                              capture_output=True, text=True, check=True).stdout

    def replace_document(text):
        before = document.stat()
        assert len(document.read_bytes()) == len(text.encode())
        document.write_text(text)
        os.utime(document, ns=(before.st_atime_ns, before.st_mtime_ns))

    build()
    assert output() == "FIRST"
    depfiles = list((target / "debug/deps").glob("xcalygraph-*.d"))
    assert depfiles and all("README.md" not in path.read_text() for path in depfiles)
    replace_document("OTHER")
    stale = build(guarded=False)
    assert output() == "FIRST", stale.stderr
    build()
    assert output() == "OTHER"
    replace_document("FIRST")
    build()
    assert output() == "FIRST"
    print("PASS: hidden build-script input stays current despite missing Cargo dep-info")
PYTHON
