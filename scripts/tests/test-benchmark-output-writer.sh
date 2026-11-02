#!/usr/bin/env bash
# Exercise benchmark output completion under system and current Bash without release builds or agents.
# Usage: scripts/tests/test-benchmark-output-writer.sh DEBUG_BINARY
set -euo pipefail
(( $# == 1 )) || { echo "usage: test-benchmark-output-writer.sh DEBUG_BINARY" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" "$1" <<'PYTHON'
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
binary = Path(sys.argv[2]).resolve(strict=True)
writer_source = repo / "scripts/benchmarks/output-writer.sh"
benchmark_scripts = ("run-benchmarks.sh", "run-evaluation.sh", "compare-depth.sh")
for name in benchmark_scripts:
    source = (repo / "scripts/benchmarks" / name).read_text()
    assert source.count('source "${REPO_ROOT}/scripts/benchmarks/output-writer.sh"') == 1, name
    assert source.count('start_output_writer "${WORK}/results.txt"') == 1, name
    assert source.count("finish_output_writer") == 1, name
    assert ">(tee " not in source and 'wait "${tee_pid}"' not in source, name

current_bash = shutil.which("bash")
assert current_bash, "bash is required"
shells = []
for shell in (current_bash, "/bin/bash"):
    resolved = str(Path(shell).resolve(strict=True))
    if resolved not in [item[0] for item in shells]:
        version = subprocess.run([resolved, "--version"], capture_output=True, text=True, check=True).stdout
        match = re.search(r"version (\d+\.\d+\.\d+)", version)
        assert match, version
        shells.append((resolved, match.group(1)))

for command in ("python3", "git", "rg", "jq", "graphify", "tee"):
    assert shutil.which(command), f"{command} is required"
real_tee = shutil.which("tee")
with tempfile.TemporaryDirectory(prefix="benchmark-output-writer.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    home = work / "home"
    home.mkdir()
    base_environment = {key: value for key, value in os.environ.items()
                        if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                        and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER",
                                        "RUSTC_WORKSPACE_WRAPPER")}
    base_environment.update(HOME=str(home), XDG_CONFIG_HOME=str(home / "config"), LC_ALL="C",
                            GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")

    delayed_tee = work / "delayed-tee"
    delayed_tee.mkdir()
    wrapper = delayed_tee / "tee"
    wrapper.write_text(
        "#!/bin/sh\n"
        "sleep 0.4\n"
        "if [ -e \"$WRITER_MARKER\" ]; then\n"
        "  printf marker-present > \"$WRITER_OBSERVATION\"\n"
        "else\n"
        "  printf marker-absent > \"$WRITER_OBSERVATION\"\n"
        "fi\n"
        f"{shlex.quote(real_tee)} \"$@\"\n"
        "tee_status=$?\n"
        ": > \"$WRITER_FINISHED\"\n"
        "if [ \"${WRITER_EXIT_STATUS:-0}\" -ne 0 ]; then exit \"$WRITER_EXIT_STATUS\"; fi\n"
        "exit \"$tee_status\"\n"
    )
    wrapper.chmod(0o755)

    runner = work / "writer-fixture.sh"
    runner.write_text(
        "#!/usr/bin/env bash\n"
        "set -euo pipefail\n"
        "source \"$1\"\n"
        "work=\"$2\"\n"
        "marker=\"$3\"\n"
        "mode=\"$4\"\n"
        "if [[ \"${mode}\" == existing ]]; then\n"
        "  if start_output_writer \"${work}/results.txt\"; then exit 97; else status=$?; fi\n"
        "  [[ \"${status}\" == 1 ]] || exit \"${status}\"\n"
        "  exit 0\n"
        "fi\n"
        "start_output_writer \"${work}/results.txt\"\n"
        "printf 'writer payload\\n'\n"
        "if finish_output_writer; then\n"
        "  [[ \"${mode}\" != failure ]] || exit 98\n"
        "  ( set -o noclobber; printf 'xcalygraph-cache-v1\\n' > \"${marker}\" )\n"
        "else\n"
        "  writer_status=$?\n"
        "  [[ \"${mode}\" == failure ]] || exit \"${writer_status}\"\n"
        "  exit \"${writer_status}\"\n"
        "fi\n"
    )

    def environment_for(shell, name, use_wrapper=False):
        tools = work / f"tools-{name}"
        tools.mkdir(exist_ok=True)
        bash_link = tools / "bash"
        if not bash_link.exists():
            bash_link.symlink_to(shell)
        tee_link = tools / "tee"
        if use_wrapper and not tee_link.exists():
            tee_link.symlink_to(wrapper)
        environment = dict(base_environment)
        environment["PATH"] = str(tools) + os.pathsep + os.environ["PATH"]
        return environment

    for number, (shell, version) in enumerate(shells):
        shell_environment = environment_for(shell, f"writer-{number}", use_wrapper=True)
        existing_case = work / f"existing-{number}"
        existing_case.mkdir()
        existing_output = existing_case / "results.txt"
        existing_output.write_bytes(b"pre-existing evidence\n")
        existing = subprocess.run(
            [shell, str(runner), str(writer_source), str(existing_case),
             str(existing_case / ".xcalygraph-disposable"), "existing"],
            env=shell_environment, capture_output=True)
        assert existing.returncode == 0, (version, existing.returncode, existing.stderr)
        assert existing_output.read_bytes() == b"pre-existing evidence\n", version
        assert not (existing_case / ".xcalygraph-disposable").exists(), version
        assert not (existing_case / "results.txt.fifo").exists(), version

        for mode, writer_status in (("success", "0"), ("failure", "23")):
            case = work / f"{mode}-{number}"
            case.mkdir()
            marker = case / ".xcalygraph-disposable"
            observation = case / "writer-observation"
            finished = case / "writer-finished"
            environment = dict(shell_environment, WRITER_MARKER=str(marker),
                               WRITER_OBSERVATION=str(observation), WRITER_FINISHED=str(finished),
                               WRITER_EXIT_STATUS=writer_status)
            result = subprocess.run([shell, str(runner), str(writer_source), str(case), str(marker), mode],
                                    env=environment, capture_output=True)
            assert result.stdout == b"writer payload\n", (version, mode, result.stdout, result.stderr)
            assert (case / "results.txt").read_bytes() == result.stdout, (version, mode)
            assert observation.read_bytes() == b"marker-absent", (version, mode)
            assert finished.is_file(), (version, mode)
            if mode == "success":
                assert result.returncode == 0, (version, result.returncode, result.stderr)
                assert marker.read_bytes() == b"xcalygraph-cache-v1\n", version
            else:
                assert result.returncode == 23, (version, result.returncode, result.stderr)
                assert not marker.exists(), version

        original_helper = writer_source.read_text()
        wait_line = '  wait "${OUTPUT_WRITER_PID}" || writer_status=$?\n'
        assert original_helper.count(wait_line) == 1
        mutant = work / f"output-writer-no-wait-{number}.sh"
        mutant.write_text(original_helper.replace(wait_line, "  : # mutation: skip the writer wait\n", 1))
        mutant_case = work / f"mutation-{number}"
        mutant_case.mkdir()
        marker = mutant_case / ".xcalygraph-disposable"
        observation = mutant_case / "writer-observation"
        finished = mutant_case / "writer-finished"
        environment = dict(shell_environment, WRITER_MARKER=str(marker), WRITER_OBSERVATION=str(observation),
                           WRITER_FINISHED=str(finished), WRITER_EXIT_STATUS="0")
        mutation = subprocess.run([shell, str(runner), str(mutant), str(mutant_case), str(marker), "success"],
                                  env=environment, capture_output=True)
        assert mutation.returncode == 0, (version, mutation.returncode, mutation.stderr)
        assert (mutant_case / "results.txt").read_bytes() == b"writer payload\n", version
        assert marker.read_bytes() == b"xcalygraph-cache-v1\n", version
        assert observation.read_bytes() == b"marker-present", version
        assert finished.is_file(), version

        legacy_case = work / f"legacy-{number}"
        legacy_case.mkdir()
        legacy = work / f"legacy-{number}.sh"
        legacy.write_text(
            "#!/usr/bin/env bash\n"
            "set -euo pipefail\n"
            "work=\"$1\"\n"
            "exec 3>&1\n"
            "exec > >(tee \"${work}/results.txt\")\n"
            "tee_pid=$!\n"
            "printf 'legacy payload\\n'\n"
            "exec >&3 3>&-\n"
            "wait \"${tee_pid}\"\n"
            "( set -o noclobber; printf 'xcalygraph-cache-v1\\n' > \"${work}/.xcalygraph-disposable\" )\n"
        )
        legacy_result = subprocess.run([shell, str(legacy), str(legacy_case)],
                                       env=base_environment, capture_output=True)
        assert legacy_result.stdout == b"legacy payload\n", (version, legacy_result.stdout)
        assert (legacy_case / "results.txt").read_bytes() == legacy_result.stdout, version
        if version.startswith("3.2."):
            assert legacy_result.returncode != 0 and not (legacy_case / ".xcalygraph-disposable").exists(), \
                (version, legacy_result.returncode, legacy_result.stderr)
        else:
            assert legacy_result.returncode == 0 and (legacy_case / ".xcalygraph-disposable").is_file(), \
                (version, legacy_result.returncode, legacy_result.stderr)

    def tools_for_entrypoint(shell, label):
        return environment_for(shell, f"entry-{label}")

    def verify_evidence(result, expression, label):
        assert result.returncode == 0, (label, result.returncode, result.stderr, result.stdout)
        match = re.search(expression, result.stdout)
        assert match, (label, result.stdout)
        evidence = Path(os.fsdecode(match.group(1)))
        assert (evidence / "results.txt").read_bytes() == result.stdout, label
        assert (evidence / ".xcalygraph-disposable").read_bytes() == b"xcalygraph-cache-v1\n", label
        assert not (evidence / "results.txt.fifo").exists(), label
        return evidence

    evaluation_source = (repo / "scripts/benchmarks/run-evaluation.sh").read_text()
    old_build = (
        'build_binary() {\n'
        '  bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"\n'
        '  RUSTFLAGS="-Dwarnings" cargo build --locked --bin xcalygraph --message-format=json -q \\\n'
        '    | bash "${REPO_ROOT}/scripts/utils/cargo-artifact.sh"\n'
        '}\n'
    )
    assert evaluation_source.count(old_build) == 1
    evaluation_source = evaluation_source.replace(
        old_build, 'build_binary() { printf \'%s\\n\' "${EVALUATION_TEST_BINARY}"; }\n', 1)

    for number, (shell, version) in enumerate(shells):
        project = work / f"evaluation-{number}"
        questions = project / "scripts/benchmarks/questions/writer-set"
        (project / "scripts/benchmarks").mkdir(parents=True)
        questions.mkdir(parents=True)
        (project / "src").mkdir()
        (project / ".gitignore").write_text(".cache/\n")
        (project / "src/main.rs").write_text('pub fn answer() -> &\'static str { "ok" }\n')
        (questions / "xcalygraph.tsv").write_text("question\texpected\nfind answer\tsrc/main.rs::answer\n")
        (project / "scripts/benchmarks/run-evaluation.sh").write_text(evaluation_source)
        shutil.copyfile(writer_source, project / "scripts/benchmarks/output-writer.sh")
        utilities = project / "scripts/utils"
        utilities.mkdir()
        shutil.copyfile(repo / "scripts/utils/prepare-corpus.sh", utilities / "prepare-corpus.sh")
        (utilities / "verification-lock.sh").write_text("#!/bin/sh\nexit 0\n")
        environment = tools_for_entrypoint(shell, f"evaluation-{number}")
        environment["EVALUATION_TEST_BINARY"] = str(binary)
        run = subprocess.run([shell, str(project / "scripts/benchmarks/run-evaluation.sh"),
                              "--no-upstream", "--set", "writer-set", "xcalygraph"],
                             cwd=project, env=environment, capture_output=True, timeout=120)
        verify_evidence(run, rb"evidence: ([^\s]+) \(identity\.tsv, misses-<corpus>\.txt, upstream-errors\.log\)",
                        f"run-evaluation Bash {version}")

    original_depth = (repo / "scripts/benchmarks/compare-depth.sh").read_text()
    cargo_command = (
        '  env CARGO_HOME="${CARGO_HOME:-${HOME}/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-${HOME}/.rustup}" \\\n'
        '    HOME="${BENCHMARK_HOME}" XCALYGRAPH_DUMP_ROOT="${copy}" XCALYGRAPH_DUMP_FILE="${ours}" \\\n'
        '    cargo test -q --release --locked dump_edges -- --ignored >/dev/null'
    )
    assert original_depth.count(cargo_command) == 1
    depth_source = original_depth.replace(cargo_command, '  : > "${ours}"', 1)

    for number, (shell, version) in enumerate(shells):
        project = work / f"depth-{number}"
        (project / "research/corpora/fixture").mkdir(parents=True)
        (project / "scripts/benchmarks").mkdir(parents=True)
        utilities = project / "scripts/utils"
        utilities.mkdir(parents=True)
        (project / ".gitignore").write_text(".cache/\n")
        (project / "research/corpora/fixture/example.py").write_text("def answer():\n    return 42\n")
        (project / "scripts/benchmarks/compare-depth.sh").write_text(depth_source)
        shutil.copyfile(writer_source, project / "scripts/benchmarks/output-writer.sh")
        shutil.copyfile(repo / "scripts/utils/prepare-corpus.sh", utilities / "prepare-corpus.sh")
        (utilities / "check-build-space.sh").write_text("#!/bin/sh\nexit 0\n")
        environment = tools_for_entrypoint(shell, f"depth-{number}")
        run = subprocess.run([shell, str(project / "scripts/benchmarks/compare-depth.sh"), "fixture", "1"],
                             cwd=project, env=environment, capture_output=True, timeout=120)
        verify_evidence(run, rb"evidence: ([^\s;]+); snapshot:", f"compare-depth Bash {version}")

    print("PASS: real tee completion, writer failure, no-wait mutation, Bash 3.2 regression, and all three entrypoints")
PYTHON
