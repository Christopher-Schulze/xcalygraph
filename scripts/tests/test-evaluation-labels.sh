#!/usr/bin/env bash
# Exercise the real sealed evaluator, binary and Graphify on an exclusive temporary git corpus.
# Usage: scripts/tests/test-evaluation-labels.sh [BINARY]
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
(( $# <= 1 )) || { echo "usage: test-evaluation-labels.sh [BINARY]" >&2; exit 2; }
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "$REPO_ROOT" "${1:-}" <<'PYTHON'
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
target = Path(os.environ.get("CARGO_TARGET_DIR", repo / "target"))
if not target.is_absolute():
    target = repo / target
binary = Path(sys.argv[2]).resolve(strict=True) if sys.argv[2] else (target / "debug/xcalygraph").resolve(strict=True)
with tempfile.TemporaryDirectory(prefix="evaluation-labels.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root = work / "repo"
    home = work / "home"
    root.mkdir()
    home.mkdir()
    temporary_directory = work / "temporary"
    temporary_directory.mkdir()
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(temporary_directory), XDG_CONFIG_HOME=str(home / "config"),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    for relative in ("scripts/utils/prepare-corpus.sh", "scripts/benchmarks/evaluate-sealed.sh"):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(repo / relative, path)
        path.chmod(0o755)
    (root / ".gitignore").write_text(".cache/\n")
    (root / "main.go").write_text('package sample\n// Create returns a sample star.\nfunc Create() string { return "star" }\n\nfunc Other() {}\n')
    (root / "right.go").write_text("package sample\nfunc Delete() {}\n")
    for command in (["git", "init", "-q", "-b", "main", str(root)],
                    ["git", "-C", str(root), "add", "."],
                    ["git", "-C", str(root), "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                     "commit", "-qm", "fixture"]):
        subprocess.run(command, env=environment, check=True)
    helper = root / "scripts/utils/prepare-corpus.sh"
    result = subprocess.run(["bash", str(helper), str(root), str(root / ".cache/original"), "--graphify"],
                            env=environment, capture_output=True, text=True, check=True)
    source = Path(result.stdout.strip())
    metadata = json.loads((source.parent / "snapshot.json").read_text())
    # Force the real pinned-commit restore path without removing or mutating any evidence.
    frozen = root / ".cache/corpus-snapshots" / metadata["source_sha256"] / source.parent.name / "source"
    assert not frozen.exists()
    cases_dir = root / ".cache/cases"
    cases_dir.mkdir()

    def locator(path, line, name):
        data = (source / path).read_bytes()
        lines = data.decode().splitlines(keepends=True)
        end = min(line + 11, len(lines))
        return {"path": path, "line": line, "name": name,
                "file_sha256": hashlib.sha256(data).hexdigest(), "evidence_end": end,
                "evidence_sha256": hashlib.sha256("".join(lines[line-1:end]).encode()).hexdigest()}

    create = locator("main.go", 3, "Create")
    other = locator("main.go", 5, "Other")
    delete = locator("right.go", 2, "Delete")
    group = lambda loc: {"alternatives": [loc]}
    # Real answers to Create must not earn a definition hit for Other in the same file,
    # or a complete result for the two required source locations.
    cases = [
        {"id": "correct", "family": "symbol", "question": "Create", "requires": [group(create)]},
        {"id": "wrong-definition", "family": "behavior", "question": "Create", "requires": [group(other)]},
        {"id": "partial", "family": "multi", "question": "Create", "requires": [group(create), group(delete)]},
        {"id": "relationship", "family": "relationship", "question": "Create", "requires": [group(create), group(delete)]},
        {"id": "negative", "family": "negative", "question": "CreateAbsent", "requires": [],
         "absence": {"identifier": "CreateAbsent", "source_sha256": metadata["source_sha256"]}},
    ]
    case_data = "".join(json.dumps(case) + "\n" for case in cases).encode()
    (cases_dir / "fixture.jsonl").write_bytes(case_data)
    commit = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], env=environment,
                            capture_output=True, text=True, check=True).stdout.strip()
    manifest = {"schema": 1, "budgets": [200, 600], "exposure": "isolated real-tool regression",
                "corpora": [{"name": "xcalygraph", "commit": commit,
                             "snapshot": str(frozen.relative_to(root)),
                             "source_sha256": metadata["source_sha256"], "upstream": metadata["upstream"],
                             "cases": "fixture.jsonl", "cases_sha256": hashlib.sha256(case_data).hexdigest(), "count": 5}]}
    manifest_path = cases_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest))
    runner = root / "scripts/benchmarks/evaluate-sealed.sh"
    arguments = ["bash", str(runner), str(manifest_path), "final", str(binary)]
    run = subprocess.run(arguments, env=environment, capture_output=True, text=True)
    assert run.returncode == 0, run.stderr + run.stdout
    evidence = Path(run.stdout.splitlines()[-1].removeprefix("evidence: "))
    assert (evidence / ".xcalygraph-disposable").read_bytes() == b"xcalygraph-cache-v1\n"
    report = json.loads((evidence / "report.json").read_text())
    public = json.loads((evidence / "public-report.json").read_text())
    assert public["manifest_sha256"] == hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    assert public["binary_sha256"] == hashlib.sha256(binary.read_bytes()).hexdigest()
    assert public["corpora"] == report["corpora"] and str(root) not in json.dumps(public)
    assert report["exposure"]["xcalygraph"]["first_evaluation"]
    for arm in ("find-text", "find-json"):
        scores = [json.loads(line) for line in (evidence / "xcalygraph" / (arm + "-600.scores.jsonl")).read_text().splitlines()]
        correct, wrong, partial, relationship, negative = scores
        assert correct["complete"] and correct["complete5"], scores
        assert wrong["file5"] and not wrong["complete"], scores
        assert partial["covered"] == 1 and partial["required"] == 2 and not partial["complete"], scores
        assert relationship["required"] == 2 and not relationship["complete"], scores
        assert negative["false_positive"], scores
    metrics = report["corpora"]["xcalygraph"]["metrics"]
    assert metrics["find-text-600"]["multi_total"] == 1
    assert metrics["find-text-600"]["multi_location_total"] == 2
    finds = [json.loads(line) for line in (evidence / "xcalygraph/find-text-600.scores.jsonl").read_text().splitlines()]
    contexts = [json.loads(line) for line in (evidence / "xcalygraph/ctx-text-600.scores.jsonl").read_text().splitlines()]
    workflow = [json.loads(line) for line in (evidence / "xcalygraph/workflow-text-600.scores.jsonl").read_text().splitlines()]
    assert finds[0] != contexts[0] and finds[2] != contexts[2]
    assert workflow == [context if find["required"] > 1 else find for find, context in zip(finds, contexts)]
    assert metrics["workflow-text-600"]["multi_location_total"] == 2
    paired = report["corpora"]["xcalygraph"]["paired_complete"]["workflow-text-600-vs-graphify-legacy-2000"]
    assert sum(paired.values()) == 4
    existing = set((root / ".cache/evaluation").glob("sealed.*"))
    repeat = subprocess.run(arguments, env=environment, capture_output=True, text=True)
    assert repeat.returncode != 0 and "already evaluated" in repeat.stderr
    failed, = set((root / ".cache/evaluation").glob("sealed.*")) - existing
    assert not (failed / ".xcalygraph-disposable").exists()
    assert len(list((root / ".cache/evaluation").glob("sealed.*/report.json"))) == 1
    validation = subprocess.run(arguments[:3] + ["validate", arguments[4]], env=environment,
                                capture_output=True, text=True)
    assert validation.returncode == 0, validation.stderr + validation.stdout
    validated = Path(validation.stdout.strip().rsplit("evidence: ", 1)[-1])
    assert (validated / ".xcalygraph-disposable").read_bytes() == b"xcalygraph-cache-v1\n"
    assert not (root / ".cache/evaluation/exposure/.xcalygraph-disposable").exists()
    focused = subprocess.run(arguments[:3] + ["development", arguments[4], "--budget", "600",
                                              "--reuse-graphify", str(evidence)],
                             env=environment, capture_output=True, text=True)
    assert focused.returncode == 0, focused.stderr + focused.stdout
    focused_evidence = Path(focused.stdout.splitlines()[-1].removeprefix("evidence: "))
    assert (focused_evidence / ".xcalygraph-disposable").read_bytes() == b"xcalygraph-cache-v1\n"
    identity = json.loads((focused_evidence / "identity.json").read_text())
    focused_report = json.loads((focused_evidence / "report.json").read_text())
    focused_public = json.loads((focused_evidence / "public-report.json").read_text())
    assert identity["budgets"] == [600]
    assert identity["manifest_sha256"] == hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    assert not focused_report["exposure"]["xcalygraph"]["first_evaluation"]
    assert focused_public["prior_exposure"]["xcalygraph"] and focused_public["graphify_baseline_reused"]
    metrics = focused_report["corpora"]["xcalygraph"]["metrics"]
    assert "find-text-600" in metrics and "find-text-200" not in metrics
    assert "graphify-legacy-2000" in metrics
    baseline = evidence / "xcalygraph/correct/graphify-legacy.stdout"
    reused = focused_evidence / "xcalygraph/correct/graphify-legacy.stdout"
    observation = json.loads(reused.with_suffix(".json").read_text())
    assert reused.read_bytes() == baseline.read_bytes()
    assert observation["reused_from"] == str(baseline.with_suffix(""))
    assert observation["stdout_sha256"] == hashlib.sha256(reused.read_bytes()).hexdigest()
    invalid = subprocess.run(arguments[:3] + ["development", arguments[4], "--budget", "300"],
                             env=environment, capture_output=True, text=True)
    assert invalid.returncode != 0 and "budget" in invalid.stderr
    bad_baseline = work / "bad-baseline"
    bad_baseline.mkdir()
    (bad_baseline / "identity.json").write_text(json.dumps({"manifest_sha256": "wrong"}))
    bad_reuse = subprocess.run(arguments[:3] + ["development", arguments[4], "--reuse-graphify", str(bad_baseline)],
                               env=environment, capture_output=True, text=True)
    assert bad_reuse.returncode != 0 and "manifest" in bad_reuse.stderr
    final_subset = subprocess.run(arguments + ["--budget", "600"],
                                  env=environment, capture_output=True, text=True)
    assert final_subset.returncode != 0 and "development" in final_subset.stderr
    original_output = reused.read_bytes()
    assert original_output
    reused.write_bytes(bytes([original_output[0] ^ 1]) + original_output[1:])
    corrupted = subprocess.run(arguments[:3] + ["development", arguments[4], "--budget", "600",
                                                "--reuse-graphify", str(focused_evidence)],
                               env=environment, capture_output=True, text=True)
    assert corrupted.returncode != 0 and "checksum mismatch" in corrupted.stderr
    linked = work / "linked-repo"
    linked_runner = linked / "scripts/benchmarks/evaluate-sealed.sh"
    linked_runner.parent.mkdir(parents=True)
    shutil.copyfile(runner, linked_runner)
    foreign = work / "foreign-cache"
    foreign.mkdir()
    (linked / ".cache").symlink_to(foreign, target_is_directory=True)
    rejected = subprocess.run(["bash", str(linked_runner), str(manifest_path), "validate", str(binary)],
                              env=environment, capture_output=True, text=True)
    assert rejected.returncode != 0 and "symlink" in rejected.stderr
    assert not list(foreign.iterdir())
    print("PASS: evaluator scoring, exposure, baseline reuse, completed-run markers and linked-cache refusal")
PYTHON
