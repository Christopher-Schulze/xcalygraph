#!/usr/bin/env bash
# Exercise artifact selection against real Cargo builds in an isolated project.
# Usage: scripts/tests/test-cargo-artifact.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-cargo-artifact.sh" >&2; exit 2; }
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
with tempfile.TemporaryDirectory(prefix="cargo-artifact.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, home, tmp = (work / name for name in ("project", "home", "tmp"))
    for directory in (root / "src", root / "target/debug", root / "scripts/utils", home, tmp):
        directory.mkdir(parents=True)
    (root / "Cargo.toml").write_text('[package]\nname = "xcalygraph"\nversion = "0.0.0"\nedition = "2024"\n')
    (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "xcalygraph"\nversion = "0.0.0"\n')
    (root / "src/main.rs").write_text('fn main() { println!("CURRENT"); }\n'
                                      '#[test] #[ignore] fn measure_query_compute() { println!("MEASURED"); }\n')
    old_binary = root / "target/debug/xcalygraph"
    old_binary.write_text('#!/usr/bin/env bash\nprintf "STALE\\n"\n')
    old_binary.chmod(0o755)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), XDG_CONFIG_HOME=str(home / "config"),
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       CARGO_NET_OFFLINE="true")

    def build(env, *arguments):
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        assert shutil.disk_usage(root).free > 2 * 1024**3 + 128 * 1024**2
        return subprocess.run(["cargo", "build", "--locked", "--bin", "xcalygraph", "--message-format=json",
                               *arguments], cwd=root, env=env, capture_output=True, check=True).stdout

    def output(binary):
        return subprocess.run([str(binary)], cwd=root, env=environment, capture_output=True, text=True, check=True).stdout.strip()

    redirected = dict(environment, CARGO_TARGET_DIR=str(work / "target with spaces\nand 'quotes'"))
    messages = build(redirected)
    actual = Path(redirected["CARGO_TARGET_DIR"]) / "debug/xcalygraph"
    assert output(actual) == "CURRENT" and output(old_binary) == "STALE"
    subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
    assert shutil.disk_usage(root).free > 2 * 1024**3 + 128 * 1024**2
    measured = subprocess.run(["cargo", "test", "-v", "--locked", "--bin", "xcalygraph",
                               "measure_query_compute", "--", "--ignored", "--nocapture"],
                              cwd=root, env=redirected, capture_output=True, text=True, check=True)
    assert "MEASURED" in measured.stdout
    assert not any("Running `" in line and " measure_query_compute " in line for line in measured.stderr.splitlines())
    parser = root / "scripts/utils/cargo-artifact.sh"
    shutil.copyfile(repo / "scripts/utils/cargo-artifact.sh", parser)

    def select(data):
        return subprocess.run(["bash", str(parser)], cwd=root, env=environment, input=data, capture_output=True)

    def selected(data):
        result = select(data)
        assert result.returncode == 0, result.stderr.decode()
        return Path(os.fsdecode(result.stdout.rstrip(b"\n")))

    assert selected(messages) == actual and output(selected(messages)) == "CURRENT"
    records = messages.splitlines(keepends=True)
    artifacts = [line for line in records if json.loads(line).get("reason") == "compiler-artifact"
                 and json.loads(line).get("executable")]
    assert len(artifacts) == 1
    assert select(messages + artifacts[0]).returncode != 0
    assert select(b"".join(line for line in records if line not in artifacts)).returncode != 0
    wrong = json.loads(artifacts[0])
    wrong["manifest_path"] = str(work / "other/Cargo.toml")
    assert select(json.dumps(wrong).encode() + b"\n").returncode != 0

    subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
    assert shutil.disk_usage(root).free > 2 * 1024**3 + 128 * 1024**2
    measured_json = subprocess.run(["cargo", "test", "-v", "--locked", "--bin", "xcalygraph",
                                    "--message-format=json", "measure_query_compute", "--", "--ignored", "--nocapture"],
                                   cwd=root, env=redirected, capture_output=True, check=True)
    test_path_file = work / "test-binary.path"
    measured_selection = subprocess.run(["bash", "-c", 'exec bash "$1" --test --passthrough 3>"$2"',
                                         "artifact-test", str(parser), str(test_path_file)], cwd=root,
                                        env=environment, input=measured_json.stdout, capture_output=True)
    assert measured_selection.returncode == 0, measured_selection.stderr.decode()
    assert b"MEASURED" in measured_selection.stdout and b'"reason"' not in measured_selection.stdout
    selected_test = Path(os.fsdecode(test_path_file.read_bytes()[:-1]))
    assert selected_test.is_file() and selected_test.parent == actual.parent / "deps"

    relative = dict(environment, CARGO_TARGET_DIR="relative-target")
    assert selected(build(relative)) == root / "relative-target/debug/xcalygraph"
    (root / ".cargo").mkdir()
    (root / ".cargo/config.toml").write_text('[build]\ntarget-dir = "configured target"\n')
    assert selected(build(environment)) == root / "configured target/debug/xcalygraph"
    host = subprocess.run(["rustc", "-vV"], cwd=root, env=environment, capture_output=True, text=True, check=True)
    triple = next(line.removeprefix("host: ") for line in host.stdout.splitlines() if line.startswith("host: "))
    assert selected(build(relative, "--target", triple)) == root / "relative-target" / triple / "debug/xcalygraph"
    assert output(old_binary) == "STALE"
    print("PASS: real Cargo absolute, relative, configured and target-triple artifacts; ambiguity rejected")
PYTHON
