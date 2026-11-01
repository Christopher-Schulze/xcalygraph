#!/usr/bin/env bash
# Exercise publication scan status handling with real Git and grep failures.
# Usage: scripts/tests/test-publication-scans.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-publication-scans.sh" >&2; exit 2; }
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
with tempfile.TemporaryDirectory(prefix="publication-scans.", dir=repo / ".cache") as temporary:
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
        patterns = audit.parent / "private-patterns.txt"
        patterns.write_text("# fixture\n")
        subprocess.run(["git", "init", "-q", "-b", "main", str(root)], env=environment, check=True)
        return root, audit, patterns

    def commit(root, files, message="fixture"):
        for name, content in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        subprocess.run(["git", "add", "--", *files], cwd=root, env=environment, check=True)
        subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                        "commit", "-qm", message], cwd=root, env=environment, check=True)

    def audit(root, script, revision="HEAD"):
        return subprocess.run(["bash", str(script), revision], cwd=root, env=environment,
                              capture_output=True, text=True)

    root, script, patterns = fixture("patterns")
    commit(root, {"public.txt": "ordinary content\n"})
    assert audit(root, script).returncode == 0
    patterns.write_text("unique-fixture-pattern\n")
    assert audit(root, script).returncode == 0
    bad_pattern = "unique-fixture-pattern["
    patterns.write_text(bad_pattern + "\n")
    invalid = audit(root, script)
    assert invalid.returncode != 0 and "private pattern validation" in invalid.stderr, invalid.stdout + invalid.stderr
    assert bad_pattern not in invalid.stdout + invalid.stderr
    patterns.write_bytes(b"unique-fixture-pattern\0suffix\n")
    binary_pattern = audit(root, script)
    assert binary_pattern.returncode != 0 and "contains NUL" in binary_pattern.stderr, binary_pattern.stdout + binary_pattern.stderr
    patterns.write_text("unique-fixture-pattern\n")
    patterns.chmod(0)
    try:
        probe = subprocess.run(["grep", "-Ev", r"^[[:space:]]*(#|$)", str(patterns)],
                               env=environment, capture_output=True)
        unreadable = audit(root, script)
        if probe.returncode > 1:
            assert unreadable.returncode != 0 and "clean" not in unreadable.stdout
        else:
            assert unreadable.returncode == 0, unreadable.stderr
    finally:
        patterns.chmod(0o600)

    root, script, patterns = fixture("missing-blob")
    patterns.write_text("unique-fixture-pattern\n")
    object_format = subprocess.run(["git", "rev-parse", "--show-object-format"], cwd=root, env=environment,
                                   capture_output=True, text=True, check=True).stdout.strip()
    assert object_format in ("sha1", "sha256"), object_format
    absent = b"f" * (40 if object_format == "sha1" else 64)
    tree = subprocess.run(["git", "mktree", "--missing", "-z"], cwd=root, env=environment,
                          input=b"100644 blob " + absent + b"\tpublic.txt\0",
                          capture_output=True, check=True).stdout.decode().strip()
    revision = subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                               "commit-tree", tree, "-m", "fixture"], cwd=root, env=environment,
                              capture_output=True, text=True, check=True).stdout.strip()
    missing = audit(root, script, revision)
    assert missing.returncode != 0 and "private content scan" in missing.stderr, missing.stdout + missing.stderr

    root, _, _ = fixture("nested")
    nested = root / "nested/scripts/audits/check-publication.sh"
    nested.parent.mkdir(parents=True)
    shutil.copyfile(repo / "scripts/audits/check-publication.sh", nested)
    (nested.parent / "private-patterns.txt").write_text("unique-fixture-pattern\n")
    commit(root, {"outside.txt": "unique-fixture-pattern\n", "nested/allowed.txt": "ordinary\n"})
    outside = audit(root, nested)
    assert outside.returncode != 0 and "private pattern in tree" in outside.stderr, outside.stdout + outside.stderr

    root, script, patterns = fixture("message")
    patterns.write_text("unique-fixture-pattern\n")
    commit(root, {"public.txt": "ordinary\n"}, "unique-fixture-pattern")
    message = audit(root, script)
    assert message.returncode != 0 and "private pattern in commit message" in message.stderr
    print("PASS: real pattern, Git and grep failures, full-tree content and valid no-match cases")
PYTHON
