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
import json
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
    odd = "strange name é\\part\tline\n.rs"
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
        return subprocess.run([shell, "-c", 'source "$1"; enumerate_source_paths "$2" "$3"',
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
    (corpus / ".xcalygraphignore").write_text("aaa-ignored.rs\n")
    (corpus / "aaa-ignored.rs").write_bytes(b"pub fn ignored() {}\n")
    (corpus / "testdata").mkdir()
    (corpus / "testdata/fixture.rs").write_bytes(b"pub fn fixture() {}\n")
    (corpus / "id_rsa.rs").write_bytes(b"pub fn sensitive() {}\n")
    (corpus / "large.rs").write_bytes(b"pub fn large() {}\n" + b" " * (8 * 1024 * 1024))
    (corpus / odd).write_bytes(content)
    (corpus / "decoy.txt").write_bytes(b"preserve\n")
    subprocess.run(["git", "-C", str(corpus), "add", "--", odd], env=environment, check=True)
    ignored_corpus = invocation / "ignored-repo"
    ignored_corpus.mkdir()
    subprocess.run(["git", "init", "-q", str(ignored_corpus)], env=environment, check=True)
    (ignored_corpus / ".xcalygraphignore").write_text("only-ignored.rs\n")
    (ignored_corpus / "only-ignored.rs").write_bytes(b"pub fn ignored_only() {}\n")
    path_only_corpus = invocation / "path-only-repo"
    (path_only_corpus / "testdata").mkdir(parents=True)
    subprocess.run(["git", "init", "-q", str(path_only_corpus)], env=environment, check=True)
    (path_only_corpus / "testdata/fixture.rs").write_bytes(b"pub fn fixture() {}\n")
    (path_only_corpus / "id_rsa.rs").write_bytes(b"pub fn sensitive() {}\n")
    (path_only_corpus / "large.rs").write_bytes(b"pub fn large() {}\n" + b" " * (8 * 1024 * 1024))
    (project / "scripts/benchmarks/questions/private/corpora.tsv").write_text(
        "weird\tsource-repo\nignored-only\tignored-repo\npath-only\tpath-only-repo\n")
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
    ignored_prepared = subprocess.run(["bash", str(project / "scripts/utils/prepare-corpus.sh"), str(ignored_corpus),
                                       str(project / ".cache/corpus-snapshots")], env=environment,
                                      capture_output=True, check=True)
    assert ignored_prepared.stdout.endswith(b"\n")
    ignored_snapshot = Path(os.fsdecode(ignored_prepared.stdout[:-1]))
    path_only_prepared = subprocess.run(["bash", str(project / "scripts/utils/prepare-corpus.sh"),
                                         str(path_only_corpus), str(project / ".cache/corpus-snapshots")],
                                        env=environment, capture_output=True, check=True)
    assert path_only_prepared.stdout.endswith(b"\n")
    path_only_snapshot = Path(os.fsdecode(path_only_prepared.stdout[:-1]))

    source = runner_source.read_text()
    build = ('BINARY="$(cargo build --release --locked --bin xcalygraph --message-format=json -q \\\n'
             '  | bash "${REPO_ROOT}/scripts/utils/cargo-artifact.sh")"')
    cleanup = '    rm -rf "${copy}" "${copy_index}"'
    assert source.count(build) == 1 and source.count(cleanup) == 1
    measured_runner = project / "scripts/benchmarks/run-benchmarks.sh"
    measured_runner.write_text(source.replace(build, 'BINARY="${PERFORMANCE_TEST_BINARY}"', 1)
                               .replace(cleanup, '    : # Retain the exclusively test-owned copy for inspection.', 1))
    environment["PERFORMANCE_TEST_BINARY"] = str(binary)
    measured = subprocess.run([shell, str(measured_runner), "weird", "ignored-only", "path-only"], cwd=invocation, env=environment,
                              capture_output=True, timeout=180)
    assert measured.returncode == 0, measured.stderr + measured.stdout
    start = measured.stdout.rfind(b"evidence: ")
    end = measured.stdout.find(b" (identity.tsv and measurement artifacts)", start)
    assert start >= 0 and end >= 0, measured.stdout
    run = Path(os.fsdecode(measured.stdout[start + len(b"evidence: "):end]))
    copied = run / "copy-1"
    ignored_copy = run / "copy-2"
    path_only_copy = run / "copy-3"
    assert (run / "source-1.path").read_bytes() == os.fsencode(str(copied / odd)) + b"\0"
    candidates = (run / "source-1.candidates").read_bytes().split(b"\0")
    assert all(os.fsencode(str(copied / path)) in candidates for path in
               ("aaa-ignored.rs", "testdata/fixture.rs", "id_rsa.rs", "large.rs", odd))
    assert (run / "probe-1.txt").read_text().strip() == "files 6 changed 1 deleted 0"
    assert (run / "source-2.path").read_bytes() == b""
    assert (run / "source-2.candidates").read_bytes() == os.fsencode(str(ignored_copy / "only-ignored.rs")) + b"\0"
    assert (run / "source-3.path").read_bytes() == b""
    path_only_candidates = (run / "source-3.candidates").read_bytes().split(b"\0")
    assert all(os.fsencode(str(path_only_copy / path)) in path_only_candidates
               for path in ("testdata/fixture.rs", "id_rsa.rs", "large.rs"))
    assert measured.stderr.count(b"no indexed source with extracted symbols; refresh not measured") == 2
    rows = [line.split() for line in measured.stdout.splitlines()
            if line.startswith((b"weird ", b"ignored-only ", b"path-only "))]
    assert len(rows) == 3 and rows[0][0] == b"weird" and rows[0][5] != b"-"
    assert rows[1][0] == b"ignored-only" and rows[1][5] == b"-" and rows[1][7] == b"-", (rows, measured.stderr)
    assert rows[2][0] == b"path-only" and rows[2][5] == b"-" and rows[2][7] == b"-", (rows, measured.stderr)
    edited = (copied / odd).read_bytes()
    assert edited.startswith(content) and len(edited) > len(content)
    assert (copied / "decoy.txt").read_bytes() == b"preserve\n"
    assert (copied / "aaa-ignored.rs").read_bytes() == b"pub fn ignored() {}\n"
    assert (copied / "testdata/fixture.rs").read_bytes() == b"pub fn fixture() {}\n"
    assert (copied / "id_rsa.rs").read_bytes() == b"pub fn sensitive() {}\n"
    assert len((copied / "large.rs").read_bytes()) == 8 * 1024 * 1024 + len(b"pub fn large() {}\n")
    for path in ("testdata/fixture.rs", "id_rsa.rs", "large.rs"):
        query = subprocess.run([str(binary), "show", "--json", "--read-only", "-b", "65536", "-d", "0",
                                "-C", str(copied), "--index-dir", str(run / "index-copy-1"), "--", path],
                               env=environment, capture_output=True)
        records = [json.loads(line) for line in query.stdout.splitlines()]
        assert query.returncode == 0 and records[0].get("t") == "file"
        assert not any(record.get("t") == "sym" for record in records), (path, records)
    assert (ignored_copy / "only-ignored.rs").read_bytes() == b"pub fn ignored_only() {}\n"
    assert (path_only_copy / "testdata/fixture.rs").read_bytes() == b"pub fn fixture() {}\n"
    assert (path_only_copy / "id_rsa.rs").read_bytes() == b"pub fn sensitive() {}\n"
    assert len((path_only_copy / "large.rs").read_bytes()) == 8 * 1024 * 1024 + len(b"pub fn large() {}\n")
    expected = {Path(odd), Path("decoy.txt"), Path("aaa-ignored.rs"), Path("testdata/fixture.rs"),
                Path("id_rsa.rs"), Path("large.rs"), Path(".xcalygraphignore")}
    assert {path.relative_to(copied) for path in copied.rglob("*") if path.is_file()} == expected
    assert (corpus / odd).read_bytes() == content and (corpus / "decoy.txt").read_bytes() == b"preserve\n"
    assert (ignored_corpus / "only-ignored.rs").read_bytes() == b"pub fn ignored_only() {}\n"
    assert (path_only_corpus / "id_rsa.rs").read_bytes() == b"pub fn sensitive() {}\n"
    assert (ignored_snapshot / "only-ignored.rs").is_file()
    assert (path_only_snapshot / "large.rs").is_file()
    print("PASS: real debug benchmark measures an extracted change and skips ignored-only and path-only corpora")
PYTHON
