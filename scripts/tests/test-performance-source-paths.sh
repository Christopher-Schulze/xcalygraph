#!/usr/bin/env bash
# Exercise exact source-file selection in the real performance benchmark with a debug binary.
# Usage: test-performance-source-paths.sh DEBUG_BINARY
set -euo pipefail
(( $# == 1 )) || { echo "usage: test-performance-source-paths.sh DEBUG_BINARY" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" "$1" <<'PYTHON'
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
binary = Path(sys.argv[2]).resolve(strict=True)
runner_source = repo / "scripts/benchmarks/run-benchmarks.sh"
with tempfile.TemporaryDirectory(prefix="performance-source-paths.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    home, direct = work / "home", work / "direct"
    home.mkdir()
    direct.mkdir()
    odd = "strange\\name\tpart\nline.rs"
    content = b"pub fn unusual() {}\n"
    (direct / odd).write_bytes(content)
    (direct / "decoy.txt").write_bytes(b"preserve\n")
    shell = shutil.which("bash")
    assert shell
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    environment.update(HOME=str(home), XDG_CONFIG_HOME=str(home / "config"), CARGO_NET_OFFLINE="true",
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")

    def select(directory, record, env=environment):
        return subprocess.run([shell, "-c", 'source "$1"; select_source_path "$2" "$3"',
                               "selection", str(runner_source), str(directory), str(record)],
                              cwd=work, env=env, capture_output=True)

    record = work / "direct.path"
    result = select(direct, record)
    assert result.returncode == 0 and record.read_bytes() == os.fsencode(str(direct / odd)) + b"\0", result.stderr
    conflict = select(direct, record)
    assert conflict.returncode != 0 and record.read_bytes() == os.fsencode(str(direct / odd)) + b"\0"
    ordinary = work / "ordinary"
    ordinary.mkdir()
    (ordinary / "linked.rs").symlink_to(direct / odd)
    (ordinary / "module.go").write_bytes(b"package module\n")
    ordinary_record = work / "ordinary.path"
    ordinary_result = select(ordinary, ordinary_record)
    assert ordinary_result.returncode == 0 and ordinary_record.read_bytes() == os.fsencode(str(ordinary / "module.go")) + b"\0"
    empty = work / "empty"
    empty.mkdir()
    empty_record = work / "empty.path"
    empty_result = select(empty, empty_record)
    assert empty_result.returncode == 0 and empty_record.read_bytes() == b""
    missing = select(work / "missing", work / "missing.path")
    assert missing.returncode != 0
    tools = work / "without-find"
    tools.mkdir()
    dirname = shutil.which("dirname")
    assert dirname
    (tools / "dirname").symlink_to(dirname)
    no_find = select(direct, work / "no-find.path", dict(environment, PATH=str(tools)))
    assert no_find.returncode != 0 and b"find:" in no_find.stderr

    project, invocation, target = work / "project\n", work / "caller", work / "target"
    for directory in (project / "src", project / "scripts/benchmarks/questions/private", project / "scripts/utils",
                      invocation, target):
        directory.mkdir(parents=True)
    (project / "Cargo.toml").write_text('[package]\nname="xcalygraph"\nversion="0.0.0"\nedition="2024"\n')
    (project / "src/main.rs").write_text('fn main() { println!("fixture"); }\n')
    (project / ".gitignore").write_text('.cache/\n')
    for item in ("read-private-corpus.sh", "guard-benchmark.sh", "verification-target.sh", "verification-source.sh",
                 "verification-lock.sh", "prepare-corpus.sh", "check-build-space.sh"):
        shutil.copyfile(repo / "scripts/utils" / item, project / "scripts/utils" / item)
    environment["CARGO_TARGET_DIR"] = str(target)
    subprocess.run(["git", "init", "-q", str(project)], env=environment, check=True)
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=project, env=environment, capture_output=True, check=True)
    corpus = invocation / "source-repo"
    corpus.mkdir()
    subprocess.run(["git", "init", "-q", str(corpus)], env=environment, check=True)
    (corpus / odd).write_bytes(content)
    (corpus / "decoy.txt").write_bytes(b"preserve\n")
    subprocess.run(["git", "-C", str(corpus), "add", "--", odd], env=environment, check=True)
    (project / "scripts/benchmarks/questions/private/corpora.tsv").write_text("weird\tsource-repo\n")
    prepared = subprocess.run(["bash", str(project / "scripts/utils/prepare-corpus.sh"), str(corpus),
                               str(project / ".cache/corpus-snapshots")], env=environment, capture_output=True, check=True)
    assert prepared.stdout.endswith(b"\n")
    snapshot = Path(os.fsdecode(prepared.stdout[:-1]))
    tracked = str((snapshot / odd).relative_to(project))
    subprocess.run(["git", "-C", str(project), "add", "-f", "--", tracked], env=environment, check=True)
    legacy = subprocess.run(["git", "-C", str(snapshot), "ls-files", "--", "*.rs"], env=environment,
                            capture_output=True, check=True).stdout
    exact = subprocess.run(["git", "-C", str(snapshot), "ls-files", "-z", "--", "*.rs"], env=environment,
                           capture_output=True, check=True).stdout
    assert exact == os.fsencode(odd) + b"\0" and legacy != os.fsencode(odd) + b"\n"

    source = runner_source.read_text()
    build = ('BINARY="$(cargo build --release --locked --bin xcalygraph --message-format=json -q \\\n'
             '  | bash "${REPO_ROOT}/scripts/utils/cargo-artifact.sh")"')
    cleanup = '    rm -rf "${copy}" "${copy_index}"'
    assert source.count(build) == 1 and source.count(cleanup) == 1
    measured_runner = project / "scripts/benchmarks/run-benchmarks.sh"
    measured_runner.write_text(source.replace(build, 'BINARY="${PERFORMANCE_TEST_BINARY}"', 1)
                               .replace(cleanup, '    : # Retain the exclusively test-owned copy for inspection.', 1))
    environment["PERFORMANCE_TEST_BINARY"] = str(binary)
    measured = subprocess.run([shell, str(measured_runner), "weird"], cwd=invocation, env=environment,
                              capture_output=True, timeout=180)
    assert measured.returncode == 0, measured.stderr + measured.stdout
    start = measured.stdout.rfind(b"evidence: ")
    end = measured.stdout.find(b" (identity.tsv and measurement artifacts)", start)
    assert start >= 0 and end >= 0, measured.stdout
    run = Path(os.fsdecode(measured.stdout[start + len(b"evidence: "):end]))
    copied = run / "copy-1"
    assert (run / "source-1.path").read_bytes() == os.fsencode(str(copied / odd)) + b"\0"
    edited = (copied / odd).read_bytes()
    assert edited.startswith(content) and len(edited) > len(content)
    assert (copied / "decoy.txt").read_bytes() == b"preserve\n"
    assert {path.relative_to(copied) for path in copied.rglob("*") if path.is_file()} == {Path(odd), Path("decoy.txt")}
    assert (corpus / odd).read_bytes() == content and (corpus / "decoy.txt").read_bytes() == b"preserve\n"
    print("PASS: real Git and debug benchmark preserve the exact unusual source path and neighboring data")
PYTHON
