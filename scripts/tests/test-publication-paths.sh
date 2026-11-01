#!/usr/bin/env bash
# Exercise the publication audit against real commit trees with unusual path names.
# Usage: scripts/tests/test-publication-paths.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-publication-paths.sh" >&2; exit 2; }
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
with tempfile.TemporaryDirectory(prefix="publication-paths.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    home, tmp = work / "home", work / "tmp"
    home.mkdir()
    tmp.mkdir()
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")

    def fixture(name):
        root = work / name
        audit = root / "scripts/audits/check-publication.sh"
        audit.parent.mkdir(parents=True)
        shutil.copyfile(repo / "scripts/audits/check-publication.sh", audit)
        (audit.parent / "private-patterns.txt").write_text("# fixture\n")
        subprocess.run(["git", "init", "-q", "-b", "main", str(root)], env=environment, check=True)
        return root, audit

    def commit(root, paths, message="fixture"):
        for name in paths:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            if not path.exists():
                path.write_text("ordinary fixture content\n")
        subprocess.run(["git", "add", "--", *paths], cwd=root, env=environment, check=True)
        subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                        "commit", "-qm", message], cwd=root, env=environment, check=True)

    def audit(root, script, revision="HEAD"):
        return subprocess.run(["bash", str(script), revision], cwd=root, env=environment,
                              capture_output=True, text=True)

    protected = ("AGENTS.md", "CLAUDE.md", "docs/decisions.md", "docs/tasks.md",
                 "docs/tasks/hidden\nname.md", "research/über\tname.txt",
                 "scripts/benchmarks/questions/private/back\\slash.txt", "scripts/audits/private-patterns.txt")
    for number, name in enumerate(protected):
        root, script = fixture(f"protected-{number}")
        commit(root, [name])
        result = audit(root, script)
        assert result.returncode != 0 and "local-only file tracked" in result.stderr, (name, result.stdout, result.stderr)
        if "\n" in name:
            assert "\\n" in result.stdout and name not in result.stdout, result.stdout

    for number, name in enumerate(("docs/tasks", "research", "scripts/benchmarks/questions/private")):
        root, script = fixture(f"protected-link-{number}")
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        (path.parent / "fixture-target").write_text("preserve\n")
        path.symlink_to("fixture-target")
        commit(root, [name])
        result = audit(root, script)
        assert result.returncode != 0 and "local-only file tracked" in result.stderr, (name, result.stdout, result.stderr)
        assert path.is_symlink() and (path.parent / "fixture-target").read_text() == "preserve\n"

    for number, name in enumerate(("docs/tasks", "research", "scripts/benchmarks/questions/private")):
        root, script = fixture(f"protected-empty-{number}")
        tree = subprocess.run(["git", "mktree", "-z"], cwd=root, env=environment, input=b"",
                              capture_output=True, check=True).stdout.strip()
        for part in reversed(name.split("/")):
            tree = subprocess.run(["git", "mktree", "-z"], cwd=root, env=environment,
                                  input=b"040000 tree " + tree + b"\t" + part.encode() + b"\0",
                                  capture_output=True, check=True).stdout.strip()
        revision = subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                                   "commit-tree", os.fsdecode(tree), "-m", "fixture"], cwd=root, env=environment,
                                  capture_output=True, check=True).stdout.decode().strip()
        result = audit(root, script, revision)
        assert result.returncode != 0 and "local-only file tracked" in result.stderr, (name, result.stdout, result.stderr)

    root, script = fixture("allowed")
    commit(root, ["src/allowed\nname.rs", "researcher/data.txt", "docs/tasks-other/data.txt",
                  "scripts/benchmarks/questions/private-other/data.txt"])
    result = audit(root, script)
    assert result.returncode == 0 and "1 commit(s) clean" in result.stdout, result.stderr
    invalid = audit(root, script, "missing-revision")
    assert invalid.returncode != 0, invalid.stdout + invalid.stderr

    nested_root, _ = fixture("nested-tree")
    nested_script = nested_root / "nested/scripts/audits/check-publication.sh"
    nested_script.parent.mkdir(parents=True)
    shutil.copyfile(repo / "scripts/audits/check-publication.sh", nested_script)
    (nested_script.parent / "private-patterns.txt").write_text("# fixture\n")
    commit(nested_root, ["research/outside.txt", "nested/allowed.txt"])
    nested = audit(nested_root, nested_script)
    assert nested.returncode != 0 and "local-only file tracked" in nested.stderr, nested.stdout + nested.stderr

    object_format = subprocess.run(["git", "rev-parse", "--show-object-format"], cwd=root, env=environment,
                                   capture_output=True, text=True, check=True).stdout.strip()
    assert object_format in ("sha1", "sha256"), object_format
    missing_tree = "f" * (40 if object_format == "sha1" else 64)
    broken = (f"tree {missing_tree}\nauthor Fixture <fixture@example.invalid> 1 +0000\n"
              "committer Fixture <fixture@example.invalid> 1 +0000\n\nmissing tree\n").encode()
    object_id = subprocess.run(["git", "hash-object", "-t", "commit", "-w", "--stdin"], cwd=root,
                               env=environment, input=broken, capture_output=True, check=True).stdout.decode().strip()
    assert subprocess.run(["git", "rev-list", object_id], cwd=root, env=environment, capture_output=True).returncode == 0
    missing = audit(root, script, object_id)
    assert missing.returncode != 0, missing.stdout + missing.stderr

    root, script = fixture("raw-byte")
    blob = subprocess.run(["git", "hash-object", "-w", "--stdin"], cwd=root, env=environment,
                          input=b"fixture\n", capture_output=True, check=True).stdout.strip()
    subtree = subprocess.run(["git", "mktree", "-z"], cwd=root, env=environment,
                             input=b"100644 blob " + blob + b"\tinvalid-\xff.txt\0",
                             capture_output=True, check=True).stdout.strip()
    tree = subprocess.run(["git", "mktree", "-z"], cwd=root, env=environment,
                          input=b"040000 tree " + subtree + b"\tresearch\0",
                          capture_output=True, check=True).stdout.strip()
    raw_commit = subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                                 "commit-tree", os.fsdecode(tree), "-m", "fixture"], cwd=root, env=environment,
                                capture_output=True, check=True).stdout.decode().strip()
    listed = subprocess.run(["git", "ls-tree", "-r", "--name-only", "-z", raw_commit], cwd=root,
                            env=environment, capture_output=True, check=True).stdout
    assert listed == b"research/invalid-\xff.txt\0", listed
    raw_result = subprocess.run(["bash", str(script), raw_commit], cwd=root, env=environment, capture_output=True)
    assert raw_result.returncode != 0 and b"local-only file tracked" in raw_result.stderr, raw_result.stderr
    assert b"\xff" not in raw_result.stdout, raw_result.stdout

    root, script = fixture("content")
    (script.parent / "private-patterns.txt").write_text("fixture-secret-pattern\n")
    commit(root, ["src/content.txt"])
    (root / "src/content.txt").write_text("fixture-secret-pattern\n")
    subprocess.run(["git", "add", "src/content.txt"], cwd=root, env=environment, check=True)
    subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                    "commit", "-qm", "content"], cwd=root, env=environment, check=True)
    assert audit(root, script).returncode != 0
    root, script = fixture("message")
    (script.parent / "private-patterns.txt").write_text("fixture-secret-pattern\n")
    commit(root, ["src/content.txt"], "fixture-secret-pattern")
    assert audit(root, script).returncode != 0
    root, script = fixture("home-path")
    commit(root, ["src/content.txt"])
    (root / "src/content.txt").write_text("/" + "Users/fixture\n")
    subprocess.run(["git", "add", "src/content.txt"], cwd=root, env=environment, check=True)
    subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                    "commit", "-qm", "home path"], cwd=root, env=environment, check=True)
    assert audit(root, script).returncode != 0
    root, script = fixture("trailer")
    commit(root, ["src/content.txt"], "fixture\n\nCo-authored-by: Example <fixture@example.invalid>")
    assert audit(root, script).returncode != 0
    print("PASS: exact local-only paths, clean trees, Git failures and existing privacy checks")
PYTHON
