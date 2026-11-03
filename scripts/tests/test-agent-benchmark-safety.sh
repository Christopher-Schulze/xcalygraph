#!/usr/bin/env bash
# Exercise real benchmark planning and setup without invoking an agent.
# Usage: scripts/tests/test-agent-benchmark-safety.sh [BINARY]
set -euo pipefail
(( $# <= 1 )) || { echo "usage: test-agent-benchmark-safety.sh [BINARY]" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" "${1:-}" <<'PYTHON'
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
target = Path(os.environ.get("CARGO_TARGET_DIR", repo / "target"))
if not target.is_absolute():
    target = repo / target
binary = Path(sys.argv[2]).resolve(strict=True) if sys.argv[2] else (target / "debug/xcalygraph").resolve(strict=True)
with tempfile.TemporaryDirectory(prefix="agent-benchmark-safety.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    home, tmp = work / "home", work / "tmp"
    home.mkdir()
    tmp.mkdir()
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "CODEX_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), XDG_CONFIG_HOME=str(home / "config"))

    def project(name):
        root = work / name
        runner = root / "scripts/benchmarks/run-agent-benchmark.sh"
        runner.parent.mkdir(parents=True)
        shutil.copyfile(repo / "scripts/benchmarks/run-agent-benchmark.sh", runner)
        questions = root / "scripts/benchmarks/questions/xcalygraph.tsv"
        questions.parent.mkdir()
        questions.write_text("question\texpected\nFind Create\tmain.go::Create\n")
        return root, runner

    root, runner = project("plan")
    plan = subprocess.run(["bash", str(runner), "--runs", "1", "xcalygraph"],
                          env=environment, capture_output=True, text=True)
    assert plan.returncode == 0 and "codex exec" in plan.stdout, plan.stderr
    assert not (root / ".cache").exists(), "planning wrote a cache"
    selection = subprocess.run(["bash", "-c", 'source "$1" xcalygraph >/dev/null; select_corpora missing',
                                "selection", str(runner)], env=environment, capture_output=True, text=True)
    assert selection.returncode != 0 and "no runnable corpora" in selection.stderr
    assert not (root / ".cache").exists(), "invalid selection wrote a cache"

    root, runner = project("preserve")
    base = root / ".cache/agent-benchmark"
    old_home = base / "home-none"
    old_home.mkdir(parents=True)
    sentinel = old_home / "sentinel"
    sentinel.write_text("keep prior home\n")
    old_results = base / "results.tsv"
    old_results.write_text("keep prior results\n")
    setup = r'''
set -euo pipefail
source "$1" xcalygraph >/dev/null
first="$(create_run)"
second="$(create_run)"
home1="$(prepare_home none "$first")"
home2="$(prepare_home none "$first")"
printf '%s\0' "$first" "$second" "$home1" "$home2"
'''
    prepared = subprocess.run(["bash", "-c", setup, "setup", str(runner)],
                              env=environment, capture_output=True)
    assert prepared.returncode == 0, prepared.stderr
    values = prepared.stdout.split(b"\0")
    assert len(values) == 5 and values[-1] == b"", prepared.stdout
    first, second, home1, home2 = (Path(os.fsdecode(value)) for value in values[:-1])
    assert first != second and first.parent == second.parent == base
    assert home1 != home2 and home1.parent == home2.parent == first
    assert (home1 / ".codex").is_dir() and (home2 / ".codex").is_dir()
    assert (first / "results.tsv").read_text().startswith("corpus\tarm\trun\t")
    assert (second / "results.tsv").read_text().startswith("corpus\tarm\trun\t")
    assert sentinel.read_text() == "keep prior home\n" and old_results.read_text() == "keep prior results\n"

    root, runner = project("binding")
    run_dir, binary_dir, foreign = (work / name for name in ("binding-run", "binding-bin", "foreign-bin"))
    for directory in (run_dir, binary_dir, foreign):
        directory.mkdir()
    foreign_git = work / "foreign-git"
    foreign_git.mkdir()
    (foreign_git / "decoy.txt").write_text("foreign agent workspace\n")
    for command in (["git", "init", "-q", "-b", "main", str(foreign_git)],
                    ["git", "-C", str(foreign_git), "add", "."],
                    ["git", "-C", str(foreign_git), "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                     "commit", "-qm", "foreign fixture"]):
        subprocess.run(command, env=environment, check=True)
    foreign_index = work / "foreign-index"
    foreign_index.mkdir()
    (foreign_index / "sentinel").write_text("preserve\n")
    foreign_git_config = work / "foreign-gitconfig"
    foreign_git_config.write_text("[core]\n\texcludesFile = /dev/null\n")
    captured = binary_dir / "xcalygraph"
    shutil.copyfile(binary, captured)
    captured.chmod(0o500)
    foreign_binary = shutil.which("false")
    assert foreign_binary is not None
    (foreign / "xcalygraph").symlink_to(foreign_binary)
    info = captured.lstat()
    identity = work / "binding-identity.json"
    identity.write_text(json.dumps({"binary_sha256": hashlib.sha256(captured.read_bytes()).hexdigest(),
                                    "skill_sha256": hashlib.sha256((repo / "skills/xcalygraph/SKILL.md").read_bytes()).hexdigest(),
                                    "binary_stat": [info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns,
                                                    info.st_ctime_ns, stat.S_IMODE(info.st_mode), info.st_nlink]}))
    foreign_claude_config = work / "foreign-config"
    foreign_claude_config.mkdir()
    startup = work / "shell-startup"
    startup.write_text('xcalygraph() { return 77; }\n')
    bind = r'''
set -euo pipefail
source "$1" xcalygraph >/dev/null
home="$(prepare_home xcalygraph "$2" "$3")"
verify_arm "$home" "$3" "$4" --content
xcalygraph() { return 77; }
export -f xcalygraph
export BASH_ENV="$5"
invoke_agent xcalygraph "$home" "$3" bash -c 'printf "%s\0" "$CODEX_HOME" "$(command -v xcalygraph)" "${GIT_DIR-}" "${GIT_WORK_TREE-}" "${GIT_INDEX_FILE-}" "${GIT_CONFIG_GLOBAL-}" "${GIT_CONFIG_NOSYSTEM-}" "${XCALYGRAPH_INDEX_DIR-}"; xcalygraph version >&2'
printf '%s\0' "$home"
'''
    bound_env = dict(environment, PATH=str(foreign) + os.pathsep + environment["PATH"],
                     CODEX_HOME=str(foreign_claude_config), GIT_DIR=str(foreign_git / ".git"),
                     GIT_WORK_TREE=str(foreign_git), GIT_INDEX_FILE=str(foreign_index / "index"),
                     GIT_CONFIG_GLOBAL=str(foreign_git_config), GIT_CONFIG_NOSYSTEM="0",
                     XCALYGRAPH_INDEX_DIR=str(foreign_index))
    bound = subprocess.run(["bash", "-c", bind, "binding", str(runner), str(run_dir), str(captured),
                            str(identity), str(startup)], env=bound_env, capture_output=True)
    assert bound.returncode == 0, bound.stderr
    values = bound.stdout.split(b"\0")
    assert len(values) == 10 and values[-1] == b"", bound.stdout
    config, selected = (Path(os.fsdecode(value)) for value in values[:2])
    installed_home = Path(os.fsdecode(values[8]))
    assert selected == captured and b"xcalygraph " in bound.stderr, bound.stderr
    assert config == installed_home / ".codex"
    assert values[2:5] == [b"", b"", b""] and values[5:7] == [b"/dev/null", b"1"] and values[7] == b"", bound.stdout
    assert (foreign_index / "sentinel").read_text() == "preserve\n"
    failed = subprocess.run(["bash", "-c", 'source "$1" xcalygraph >/dev/null; invoke_agent xcalygraph "$2" "$3" false',
                             "failed", str(runner), str(installed_home), str(captured)],
                            env=bound_env, capture_output=True, text=True)
    assert failed.returncode != 0
    graphify = subprocess.run(["bash", "-c", 'source "$1" xcalygraph >/dev/null; prepare_home graphify "$2"',
                               "graphify", str(runner), str(run_dir)], env=bound_env, capture_output=True, text=True)
    assert graphify.returncode == 0, graphify.stderr
    graphify_home = Path(graphify.stdout.removesuffix("\n"))
    assert (graphify_home / ".codex/skills/graphify/SKILL.md").is_file() and not list(foreign_claude_config.iterdir())
    refused = subprocess.run(["bash", "-c", 'source "$1" xcalygraph >/dev/null; invoke_agent xcalygraph "$2" "$3" true',
                              "refuse", str(runner), str(installed_home), str(work / "bad:path/xcalygraph")],
                             env=environment, capture_output=True, text=True)
    assert refused.returncode != 0 and "cannot be represented in PATH" in refused.stderr
    skill = installed_home / ".agents/skills/xcalygraph/SKILL.md"
    assert skill.read_bytes() == (repo / "skills/xcalygraph/SKILL.md").read_bytes()
    verify = 'source "$1" xcalygraph >/dev/null; verify_arm "$2" "$3" "$4"'
    skill.write_text("changed skill\n")
    altered = subprocess.run(["bash", "-c", verify, "verify", str(runner), str(installed_home), str(captured),
                              str(identity)], env=environment, capture_output=True, text=True)
    assert altered.returncode != 0 and "installed skill changed" in altered.stderr
    skill.write_bytes((repo / "skills/xcalygraph/SKILL.md").read_bytes())
    captured.chmod(0o700)
    with captured.open("ab") as stream:
        stream.write(b"changed")
    altered = subprocess.run(["bash", "-c", verify, "verify", str(runner), str(installed_home), str(captured),
                              str(identity)], env=environment, capture_output=True, text=True)
    assert altered.returncode != 0 and "captured binary changed" in altered.stderr

    root, _ = project("capture")
    (root / "src").mkdir()
    (root / "skills/xcalygraph").mkdir(parents=True)
    (root / "skills/xcalygraph/SKILL.md").write_text("fixture skill\n")
    (root / "Cargo.toml").write_text('[package]\nname="xcalygraph"\nversion="0.0.0"\nedition="2024"\n')
    (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "xcalygraph"\nversion = "0.0.0"\n')
    source = root / "src/main.rs"
    source.write_text('const SKILL: &str = include_str!("../skills/xcalygraph/SKILL.md");\n'
                      'fn main() { println!("FIRST {}", SKILL.len()); }\n')
    for item in ("verification-target.sh", "verification-source.sh", "verification-lock.sh", "guard-benchmark.sh",
                 "check-build-space.sh", "cargo-artifact.sh", "capture-benchmark-binary.sh"):
        dest = root / "scripts/utils" / item
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(repo / "scripts/utils" / item, dest)
    capture_env = dict(environment, CARGO_TARGET_DIR=str(work / "capture-target"), CARGO_NET_OFFLINE="true",
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    subprocess.run(["git", "init", "-q", str(root)], env=capture_env, check=True)
    guard = root / "scripts/utils/guard-benchmark.sh"
    capture = root / "scripts/utils/capture-benchmark-binary.sh"

    def capture_binary(destination):
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        assert shutil.disk_usage(work).free > 2 * 1024**3 + 128 * 1024**2
        return subprocess.run(["bash", str(guard), "bash", str(capture), "--debug", str(destination)],
                              cwd=root, env=capture_env, capture_output=True, text=True)

    first_binary, second_binary = work / "first-binary", work / "second-binary"
    first = capture_binary(first_binary)
    assert first.returncode == 0, first.stderr
    first_identity = json.loads(first.stdout)
    assert first_identity["binary_sha256"] == hashlib.sha256(first_binary.read_bytes()).hexdigest()
    assert first_identity["skill_sha256"] == hashlib.sha256((root / "skills/xcalygraph/SKILL.md").read_bytes()).hexdigest()
    assert first_identity["profile"] == "debug" and len(first_identity["source"]["source"]) == 64
    assert subprocess.check_output([str(first_binary)], text=True).startswith("FIRST ")
    before = source.stat()
    source.write_text(source.read_text().replace('"FIRST ', '"OTHER '))
    os.utime(source, ns=(before.st_atime_ns, before.st_mtime_ns))
    second = capture_binary(second_binary)
    assert second.returncode == 0, second.stderr
    assert subprocess.check_output([str(second_binary)], text=True).startswith("OTHER ")
    assert json.loads(second.stdout)["source"]["source"] != first_identity["source"]["source"]
    preserved = hashlib.sha256(first_binary.read_bytes()).digest()
    conflict = capture_binary(first_binary)
    assert conflict.returncode != 0 and hashlib.sha256(first_binary.read_bytes()).digest() == preserved
    unguarded = subprocess.run(["bash", str(capture), "--debug", str(work / "unguarded")], cwd=root,
                               env=capture_env, capture_output=True, text=True)
    assert unguarded.returncode != 0 and "verification lock is required" in unguarded.stderr

    for linked_part in ("cache", "category"):
        root, runner = project("linked-" + linked_part)
        foreign = work / ("foreign-" + linked_part)
        foreign.mkdir()
        if linked_part == "cache":
            (root / ".cache").symlink_to(foreign, target_is_directory=True)
        else:
            (root / ".cache").mkdir()
            (root / ".cache/agent-benchmark").symlink_to(foreign, target_is_directory=True)
        rejected = subprocess.run(["bash", "-c", 'source "$1" xcalygraph >/dev/null; create_run',
                                   "setup", str(runner)], env=environment, capture_output=True, text=True)
        assert rejected.returncode != 0 and "symlink" in rejected.stderr
        assert not list(foreign.iterdir())

    # Outcome scoring from real `codex exec --json` shapes: every status is reachable and failable.
    root, runner = project("scoring")
    def trace(name, answer, commands=(), error=False):
        path = work / (name + ".jsonl")
        events = [{"type": "thread.started", "thread_id": "t"}, {"type": "turn.started"}]
        events += [{"type": "item.completed", "item": {"id": f"c{n}", "type": "command_execution",
                    "command": "/bin/bash -lc " + json.dumps(command), "exit_code": 0, "status": "completed"}}
                   for n, command in enumerate(commands)]
        events.append({"type": "item.completed", "item": {"id": "m", "type": "agent_message", "text": answer}})
        events.append({"type": "turn.failed", "error": {"message": answer}} if error else
                      {"type": "turn.completed", "usage": {"input_tokens": 100, "cached_input_tokens": 80,
                                                           "output_tokens": 10, "reasoning_output_tokens": 5}})
        path.write_text("\n".join(json.dumps(event) for event in events) + "\n")
        return path
    cases = [("correct", "none", "1", trace("c", "src/app.ts::compose")),
             ("wrong_symbol", "none", "1", trace("w", "src/app.ts::dispatch")),
             ("miss", "none", "0", trace("m", "src/other.ts::compose")),
             ("error", "none", "1", trace("e", "Not logged in", error=True)),
             ("leak", "none", "1", trace("l", "src/app.ts::compose", ["xcalygraph find compose"])),
             ("leak", "xcalygraph", "1", trace("g", "src/app.ts::compose", ["graphify query compose"])),
             ("correct", "xcalygraph", "1", trace("x", "src/app.ts::compose", ["xcalygraph find compose"]))]
    for status, arm, matched, path in cases:
        scored = subprocess.run(["bash", "-c", 'source "$1" xcalygraph >/dev/null; evaluate_trace "$2" "$3" "$4" "$5" 1200',
                                 "score", str(runner), str(path), arm, matched, "src/app.ts::compose"],
                                env=environment, capture_output=True, text=True)
        assert scored.returncode == 0, scored.stderr
        fields = scored.stdout.rstrip("\n").split("\t")
        tools = sum(1 for line in path.read_text().splitlines() if "command_execution" in line)
        expected_tokens = "0" if status == "error" else "115"
        assert fields[0] == status and fields[1:5] == [expected_tokens, "1", str(tools), "1200"], (status, fields)
    # PATH isolation: the no-tool arm cannot reach xcalygraph or graphify even when the caller can.
    tools = work / "tools"
    tools.mkdir()
    for name in ("xcalygraph", "graphify"):
        (tools / name).symlink_to(shutil.which("true"))
    isolated = subprocess.run(["bash", "-c", 'source "$1" xcalygraph >/dev/null; invoke_agent none "$2" "" bash -c \'command -v xcalygraph graphify || true\'',
                               "isolate", str(runner), str(home)],
                              env=dict(environment, PATH=str(tools) + os.pathsep + environment["PATH"]),
                              capture_output=True, text=True)
    assert isolated.returncode == 0 and isolated.stdout == "", (isolated.stdout, isolated.stderr)
    print("PASS: planning, guarded debug capture, binary/skill binding, Git/xcalygraph/shell override isolation, previous evidence, linked-cache refusal, outcome scoring and PATH isolation")
PYTHON
