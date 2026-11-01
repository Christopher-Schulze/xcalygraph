#!/usr/bin/env bash
# Exercise real performance-benchmark selection without release builds or agents.
# Usage: test-performance-benchmark-corpora.sh DEBUG_BINARY
set -euo pipefail
(( $# == 1 )) || { echo "usage: test-performance-benchmark-corpora.sh DEBUG_BINARY" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" "$1" <<'PYTHON'
import hashlib
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
with tempfile.TemporaryDirectory(prefix="performance-corpora.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root, caller, home = work / "preflight", work / "caller\n\n", work / "home"
    for directory in (root / "scripts/benchmarks/questions/private", root / "scripts/utils", caller / "relative", home):
        directory.mkdir(parents=True)
    runner = root / "scripts/benchmarks/run-benchmarks.sh"
    shutil.copyfile(runner_source, runner)
    shutil.copyfile(repo / "scripts/utils/read-private-corpus.sh", root / "scripts/utils/read-private-corpus.sh")
    listing = root / "scripts/benchmarks/questions/private/corpora.tsv"
    shells = list(dict.fromkeys((shutil.which("bash"), "/bin/bash")))
    assert all(shells)
    for number, shell in enumerate(shells):
        tools = work / f"tools-{number}"
        tools.mkdir()
        (tools / "bash").symlink_to(shell)
        for name in ("python3", "dirname", "basename"):
            executable = shutil.which(name)
            assert executable, name
            (tools / name).symlink_to(executable)
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_"))
                       and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
        environment.update(HOME=str(home), PATH=str(tools), XDG_CONFIG_HOME=str(home / "config"))
        assert shutil.which("cargo", path=environment["PATH"]) is None

        def invoke(*args, trace=False):
            command = [shell]
            if trace:
                command.append("-x")
            return subprocess.run([*command, str(runner), *args], cwd=caller, env=environment, capture_output=True)

        listing.write_text("fixture\trelative\n")
        relative = invoke("fixture", trace=True)
        assignment = subprocess.run([shell, "-x", "-c", 'path="$1"', "trace", str((caller / "relative").resolve())],
                                    env=environment, capture_output=True, check=True).stderr
        assert b"hyperfine missing" in relative.stderr and assignment in relative.stderr, relative.stderr
        default = invoke()
        assert b"hyperfine missing" in default.stderr and b"unbound variable" not in default.stderr, default.stderr
        listing.write_text("last\trelative")
        last = invoke("last")
        assert b"hyperfine missing" in last.stderr, last.stderr
        listing.write_text("# comment\nfirst\trelative\nliteral\\name\trelative")
        listed = subprocess.run([shell, str(root / "scripts/utils/read-private-corpus.sh"), "list", str(listing)],
                                env=environment, capture_output=True)
        assert listed.returncode == 0 and listed.stdout == b"first\trelative\nliteral\\name\trelative\n", listed.stderr
        for data in (b"bad\trelative\textra\n", b"bad\trelative\x00\n", b"bad\trelative\nbad\trelative\n"):
            listing.write_bytes(data)
            invalid_list = subprocess.run([shell, str(root / "scripts/utils/read-private-corpus.sh"), "list", str(listing)],
                                          env=environment, capture_output=True)
            assert invalid_list.returncode != 0 and not invalid_list.stdout, invalid_list.stderr
            rejected = invoke("xcalygraph")
            assert rejected.returncode != 0 and b"read-private-corpus.sh: invalid" in rejected.stderr, rejected.stderr
            assert b"hyperfine missing" not in rejected.stderr
        listing.write_text("xcalygraph\trelative\n")
        duplicate = invoke("xcalygraph")
        assert duplicate.returncode != 0 and b"duplicate corpus" in duplicate.stderr, duplicate.stderr
        listing.write_text("alpha/../../victim\trelative\na/b\trelative\na-b\trelative\n")
        special = invoke("alpha/../../victim", "a/b", "a-b")
        assert b"hyperfine missing" in special.stderr, special.stderr
        listing.write_bytes(b"")
        public = root / "research/corpora/bad\n"
        public.mkdir(parents=True)
        invalid_public = invoke()
        assert invalid_public.returncode != 0 and b"invalid corpus name" in invalid_public.stderr, invalid_public.stderr
        public.rmdir()
        assert not (root / ".cache").exists()

    def fixture(name, include_build_space):
        project, invocation, target = work / name, work / (name + "-caller"), work / (name + "-target")
        for directory in (project / "src", project / "scripts/benchmarks/questions/private",
                          project / "scripts/utils", invocation, target):
            directory.mkdir(parents=True)
        (project / "Cargo.toml").write_text('[package]\nname="xcalygraph"\nversion="0.0.0"\nedition="2024"\n')
        (project / "src/main.rs").write_text('fn main() { println!("fixture"); }\n')
        (project / ".gitignore").write_text('.cache/\n')
        utilities = ["read-private-corpus.sh", "guard-benchmark.sh", "verification-target.sh",
                     "verification-source.sh", "verification-lock.sh", "prepare-corpus.sh"]
        if include_build_space:
            utilities.append("check-build-space.sh")
        for utility in utilities:
            shutil.copyfile(repo / "scripts/utils" / utility, project / "scripts/utils" / utility)
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                       and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
        environment.update(HOME=str(home), CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE="true",
                           RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                           CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                           GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        subprocess.run(["git", "init", "-q", str(project)], env=environment, check=True)
        subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=project, env=environment,
                       capture_output=True, check=True)
        return project, invocation, environment

    guarded, invocation, environment = fixture("guarded", False)
    (invocation / "relative").mkdir()
    (guarded / "scripts/benchmarks/questions/private/corpora.tsv").write_text("fixture\trelative\n")
    guarded_runner = guarded / "scripts/benchmarks/run-benchmarks.sh"
    shutil.copyfile(runner_source, guarded_runner)
    canary = guarded / "scripts/utils/check-build-space.sh"
    canary.write_text('#!/bin/sh\nprintf "observed-selection=%s\\n" "${XCALYGRAPH_BENCHMARK_SELECTION_SHA256:-missing}" >&2\nexit 99\n')
    private_entry = "fixture\t" + str((invocation / "relative").resolve())
    for command, entries in ((("bash", str(guarded_runner), "fixture"), [private_entry]),
                             (("/bin/bash", str(guarded_runner)), ["xcalygraph\t" + str(guarded.resolve()), private_entry])):
        result = subprocess.run(command, cwd=invocation, env=environment, capture_output=True, timeout=90)
        assert result.returncode != 0 and b"verification-source.sh" in result.stderr, result.stderr
        digest = hashlib.sha256(b"".join(os.fsencode(entry) + b"\0" for entry in entries)).hexdigest()
        assert ("observed-selection=" + digest).encode() in result.stderr, result.stderr
        assert b"missing corpus" not in result.stderr, result.stderr
    assert not (guarded / "relative").exists()
    changed_env = dict(environment, XCALYGRAPH_BENCHMARK_SELECTION_SHA256="0" * 64)
    guarded_command = ["bash", str(guarded / "scripts/utils/guard-benchmark.sh"), "bash", "-c",
                       'cd -- "$1"; shift; exec "$@"', "selection", str(invocation),
                       "bash", str(guarded_runner), "fixture"]
    changed = subprocess.run(guarded_command, cwd=guarded, env=changed_env, capture_output=True, timeout=90)
    assert changed.returncode != 0 and b"corpus selection changed" in changed.stderr, changed.stderr
    assert b"observed-selection=" not in changed.stderr

    project, invocation, environment = fixture("measurement\n\n", True)
    for name, content in (("alpha", "pub fn alpha() {}\n"), ("beta", "pub fn beta() {}\n")):
        (invocation / name).mkdir()
        (invocation / name / "main.rs").write_text(content)
    (project / "scripts/benchmarks/questions/private/corpora.tsv").write_text(
        "alpha\talpha\nalpha/../../victim\tbeta\na/b\talpha\na-b\tbeta\n")
    victim = project / ".cache/benchmarks/victim"
    victim.mkdir(parents=True)
    (victim / "sentinel").write_text("preserve\n")
    source = runner_source.read_text()
    build = ('BINARY="$(cargo build --release --locked --bin xcalygraph --message-format=json -q \\\n'
             '  | bash "${REPO_ROOT}/scripts/utils/cargo-artifact.sh")"')
    assert source.count(build) == 1
    measured_runner = project / "scripts/benchmarks/run-benchmarks.sh"
    measured_runner.write_text(source.replace(build, 'BINARY="${PERFORMANCE_TEST_BINARY}"', 1))
    environment["PERFORMANCE_TEST_BINARY"] = str(binary)
    result = subprocess.run(["bash", str(measured_runner), "alpha", "alpha/../../victim", "a/b", "a-b"], cwd=invocation,
                            env=environment, capture_output=True, timeout=180)
    assert result.returncode == 0, result.stderr + result.stdout
    assert (victim / "sentinel").read_text() == "preserve\n"
    evidence = result.stdout.rfind(b"evidence: ")
    end = result.stdout.find(b" (identity.tsv and measurement artifacts)", evidence)
    assert evidence >= 0 and end >= 0, result.stdout
    identity = Path(os.fsdecode(result.stdout[evidence + len(b"evidence: "):end])) / "identity.tsv"
    records = [line.split("\t") for line in identity.read_text().splitlines() if line.startswith("corpus\t")]
    assert len(records) == 4 and all(len(record) == 5 for record in records), records
    actual = {json.loads(record[2]): (Path(json.loads(record[3])), Path(json.loads(record[4]))) for record in records}
    assert set(actual) == {"alpha", "alpha/../../victim", "a/b", "a-b"}
    for name, directory, content in (("alpha", "alpha", "pub fn alpha() {}\n"),
                                     ("alpha/../../victim", "beta", "pub fn beta() {}\n"),
                                     ("a/b", "alpha", "pub fn alpha() {}\n"),
                                     ("a-b", "beta", "pub fn beta() {}\n")):
        input_root, snapshot = actual[name]
        assert input_root == invocation / directory
        assert (snapshot / "main.rs").read_text() == content
    assert len(list(identity.parent.glob("*.full.time"))) == 4
    upstream = subprocess.run(["bash", str(measured_runner), "--upstream", "alpha"], cwd=invocation,
                              env=environment, capture_output=True, timeout=180)
    assert upstream.returncode == 0, upstream.stderr + upstream.stdout
    rows = [line.split() for line in upstream.stdout.splitlines() if line.startswith(b"alpha ")]
    assert len(rows) == 1 and len(rows[0]) == 12, upstream.stdout
    assert all(float(value) >= 0 for value in rows[0][1:]), rows
    assert (victim / "sentinel").read_text() == "preserve\n"
    assert (invocation / "alpha/main.rs").read_text() == "pub fn alpha() {}\n"
    print("PASS: real corpus preflight, guard re-entry and debug-binary measurements preserve selection and evidence")
PYTHON
