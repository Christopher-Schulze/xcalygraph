#!/usr/bin/env bash
# Exercise publication scans of binary and symlink blobs in real Git commits.
# Usage: scripts/tests/test-publication-blobs.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-publication-blobs.sh" >&2; exit 2; }
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
with tempfile.TemporaryDirectory(prefix="publication-blobs.", dir=repo / ".cache") as temporary:
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
        (audit.parent / "private-patterns.txt").write_text("unique-fixture-pattern\n")
        subprocess.run(["git", "init", "-q", "-b", "main", str(root)], env=environment, check=True)
        return root, audit

    def commit(root, *paths):
        subprocess.run(["git", "add", "--", *paths], cwd=root, env=environment, check=True)
        subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                        "commit", "-qm", "fixture"], cwd=root, env=environment, check=True)

    def audit(root, script, revision="HEAD"):
        return subprocess.run(["bash", str(script), revision], cwd=root, env=environment,
                              capture_output=True, text=True)

    root, script = fixture("allowed")
    (root / "text.txt").write_text("ordinary text\n")
    (root / "asset.bin").write_bytes(b"\x00ordinary\xff\x00")
    (root / "link-one").symlink_to("../safe-target")
    (root / "link-two").symlink_to("../safe-target")
    commit(root, "text.txt", "asset.bin", "link-one", "link-two")
    result = audit(root, script)
    assert result.returncode == 0 and "1 commit(s) clean" in result.stdout, result.stderr
    assert (root / "link-one").is_symlink() and (root / "link-two").is_symlink()

    root, script = fixture("binary-private")
    (root / "asset.bin").write_bytes(b"\x00UNIQUE-FIXTURE-PATTERN\xff\x00")
    commit(root, "asset.bin")
    result = audit(root, script)
    assert result.returncode != 0 and "private pattern in tree" in result.stderr, result.stdout + result.stderr
    assert "unique-fixture-pattern" not in (result.stdout + result.stderr).lower()

    root, script = fixture("binary-home")
    (root / "asset.bin").write_bytes(b"\x00/" + b"Users/fixture/private\xff\x00")
    commit(root, "asset.bin")
    result = audit(root, script)
    assert result.returncode != 0 and "private pattern in tree" in result.stderr, result.stdout + result.stderr
    assert "/" + "Users/fixture" not in result.stdout + result.stderr

    root, script = fixture("link-private")
    target = "prefix/UNIQUE-FIXTURE-PATTERN\nsuffix"
    (root / "odd\nlink").symlink_to(target)
    (root / "duplicate").symlink_to(target)
    commit(root, "odd\nlink", "duplicate")
    tree = subprocess.run(["git", "ls-tree", "-r", "-z", "HEAD"], cwd=root, env=environment,
                          capture_output=True, check=True).stdout
    objects = [entry.split(b"\t", 1)[0].split()[2] for entry in tree.split(b"\0") if entry]
    assert len(objects) == 2 and objects[0] == objects[1]
    result = audit(root, script)
    assert result.returncode != 0 and "private pattern in tree" in result.stderr, result.stdout + result.stderr
    assert "unique-fixture-pattern" not in (result.stdout + result.stderr).lower()
    assert os.readlink(root / "odd\nlink") == target and os.readlink(root / "duplicate") == target

    root, script = fixture("link-home")
    target = "/" + "Users/fixture/private"
    (root / "link").symlink_to(target)
    commit(root, "link")
    result = audit(root, script)
    assert result.returncode != 0 and "private pattern in tree" in result.stderr, result.stdout + result.stderr
    assert target not in result.stdout + result.stderr and os.readlink(root / "link") == target

    root, script = fixture("binary-link")
    oid = subprocess.run(["git", "hash-object", "-w", "--stdin"], cwd=root, env=environment,
                         input=b"\x00UNIQUE-FIXTURE-PATTERN\xff\x00",
                         capture_output=True, check=True).stdout.strip()
    tree = subprocess.run(["git", "mktree", "-z"], cwd=root, env=environment,
                          input=b"120000 blob " + oid + b"\tlink\0",
                          capture_output=True, check=True).stdout.decode().strip()
    revision = subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                               "commit-tree", tree, "-m", "fixture"], cwd=root, env=environment,
                              capture_output=True, text=True, check=True).stdout.strip()
    result = audit(root, script, revision)
    assert result.returncode != 0 and "private pattern in tree" in result.stderr, result.stdout + result.stderr
    assert "unique-fixture-pattern" not in (result.stdout + result.stderr).lower()

    root, script = fixture("missing-link")
    object_format = subprocess.run(["git", "rev-parse", "--show-object-format"], cwd=root, env=environment,
                                   capture_output=True, text=True, check=True).stdout.strip()
    assert object_format in ("sha1", "sha256"), object_format
    absent = b"f" * (40 if object_format == "sha1" else 64)
    tree = subprocess.run(["git", "mktree", "--missing", "-z"], cwd=root, env=environment,
                          input=b"120000 blob " + absent + b"\tmissing\nlink\0",
                          capture_output=True, check=True).stdout.decode().strip()
    revision = subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                               "commit-tree", tree, "-m", "fixture"], cwd=root, env=environment,
                              capture_output=True, text=True, check=True).stdout.strip()
    missing = audit(root, script, revision)
    assert missing.returncode != 0 and "clean" not in missing.stdout, missing.stdout + missing.stderr
    print("PASS: real binary, symlink, duplicate, safe and missing-blob publication checks")
PYTHON
