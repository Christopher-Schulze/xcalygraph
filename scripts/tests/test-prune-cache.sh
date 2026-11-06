#!/usr/bin/env bash
# Exercise cache pruning only in an exclusively created fixture.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

repo = Path(sys.argv[1])
base = repo / ".cache"
if base.is_symlink():
    raise SystemExit("test cache must not be a symlink")
base.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix="prune-regression.", dir=base) as temporary:
    work = Path(temporary)
    home = work / "home"
    temporary_directory = work / "temporary"
    home.mkdir()
    temporary_directory.mkdir()
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), TMPDIR=str(temporary_directory), XDG_CONFIG_HOME=str(home / "config"),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    root = work / "project"
    script = root / "scripts/utils/prune-cache.sh"
    script.parent.mkdir(parents=True)
    shutil.copyfile(repo / "scripts/utils/prune-cache.sh", script)
    cache = root / ".cache"
    cache.mkdir()
    marker = ".xcalygraph-disposable"

    def artifact(relative, content=b"xcalygraph-cache-v1\n"):
        path = cache / relative
        path.mkdir(parents=True)
        if content is not None:
            (path / marker).write_bytes(content)
        return path

    disposable = [artifact("benchmarks/run.old"), artifact("depth/run.old"),
                  artifact("evaluation/sealed.old")]
    direct_keep = artifact("evaluation/sealed.direct")
    (direct_keep / ".keep").write_text("protected")
    nested_keep = artifact("benchmarks/query.nested")
    (nested_keep / "evidence").mkdir()
    (nested_keep / "evidence/.keep").write_text("protected")
    dangling_keep = artifact("depth/run.dangling")
    (dangling_keep / ".keep").symlink_to(dangling_keep / "missing")
    invalid_marker = artifact("benchmarks/run.invalid", b"not an ownership marker\n")
    marker_link = artifact("evaluation/run.marker-link", None)
    (marker_link / marker).symlink_to(disposable[0] / marker)
    incomplete = artifact("evaluation/sealed.incomplete", None)
    foreign = artifact("foreign", None)
    foreign_bytes = os.urandom(1024 * 1024)
    (foreign / "keep").write_bytes(foreign_bytes)
    foreign_benchmark = artifact("benchmarks/foreign", None)
    unknown_marked = artifact("benchmarks/unrecognized")
    snapshot = artifact("corpus-snapshots/retained", None)
    outside = root / "outside"
    outside.mkdir()
    (outside / "keep").write_text("foreign")
    (disposable[0] / "outside-link").symlink_to(outside, target_is_directory=True)
    foreign_link = cache / "foreign-link"
    foreign_link.symlink_to(outside, target_is_directory=True)
    candidate_link = cache / "benchmarks/run.link"
    candidate_link.symlink_to(outside, target_is_directory=True)
    fresh = artifact("benchmarks/run.fresh")
    protected = [direct_keep, nested_keep, dangling_keep, invalid_marker, marker_link,
                 incomplete, foreign, foreign_benchmark, unknown_marked, snapshot]
    past = time.time() - 48 * 60 * 60
    for path in disposable + protected:
        os.utime(path, (past, past))
    for path in (foreign_link, candidate_link):
        os.utime(path, (past, past), follow_symlinks=False)

    def run(age):
        return subprocess.run(["bash", str(script), age], env=environment, text=True, capture_output=True)

    result = run("24")
    assert result.returncode == 0, result.stderr
    assert "removed 3 entries" in result.stdout, result.stdout
    assert all(not path.exists() for path in disposable), result.stdout
    assert all(path.is_dir() for path in protected), result.stdout
    assert foreign_link.is_symlink() and candidate_link.is_symlink()
    assert (outside / "keep").read_text() == "foreign"
    assert (foreign / "keep").read_bytes() == foreign_bytes
    assert (direct_keep / ".keep").read_text() == "protected"
    assert (nested_keep / "evidence/.keep").read_text() == "protected"
    assert fresh.is_dir()
    actual_kib = int(subprocess.check_output(["du", "-sk", str(cache)]).split()[0])
    assert re.search(r"\.cache is (\d+) MiB", result.stdout)
    assert int(re.search(r"\.cache is (\d+) MiB", result.stdout).group(1)) == actual_kib // 1024

    unreadable = artifact("benchmarks/run.unreadable")
    opaque = unreadable / "opaque"
    opaque.mkdir()
    opaque.chmod(0)
    inaccessible = not os.access(opaque, os.R_OK | os.X_OK)
    os.utime(unreadable, (past, past))
    try:
        if inaccessible:
            result = run("24")
            assert result.returncode != 0 and "cannot inspect" in result.stderr
            assert "cannot measure cache size" in result.stderr and unreadable.is_dir()
    finally:
        if opaque.exists():
            opaque.chmod(0o700)
    (unreadable / ".keep").write_text("protected")
    protected.append(unreadable)
    assert run("08").returncode == 0
    assert fresh.is_dir()
    invalid = run("9" * 80)
    assert invalid.returncode == 2, invalid.stderr
    assert fresh.is_dir() and foreign.is_dir()
    extra = subprocess.run(["bash", str(script), "24", "0"], env=environment, text=True, capture_output=True)
    assert extra.returncode == 2
    all_runs = run("0")
    assert all_runs.returncode == 0 and "removed 1 entries" in all_runs.stdout
    assert not fresh.exists()
    assert all(path.is_dir() for path in protected)
    assert foreign_link.is_symlink() and candidate_link.is_symlink()

    category_run = artifact("benchmarks/run.category-keep")
    category_keep = cache / "benchmarks/.keep"
    category_keep.symlink_to(cache / "missing")
    assert run("0").returncode == 0 and category_run.is_dir()
    category_keep.unlink()
    root_run = artifact("evaluation/run.root-keep")
    root_keep = cache / ".keep"
    root_keep.write_text("protect the cache")
    assert run("0").returncode == 0 and category_run.is_dir() and root_run.is_dir()
    root_keep.unlink()
    assert run("0").returncode == 0 and not category_run.exists() and not root_run.exists()

    # Work directories: age, .keep and open TASK references protect them; symlinks stay.
    (root / "docs/tasks").mkdir(parents=True)
    (root / "docs/tasks/123-open.md").write_text("evidence in .cache/task123-named.abc\n")
    old_work = artifact("task900-old.abc", None)
    (old_work / "log").write_text("old")
    old_fixture = artifact("some-test-4711-0", None)
    named_work = artifact("task123-named.abc", None)
    kept_work = artifact("task901-kept.abc", None)
    (kept_work / ".keep").write_text("")
    fresh_work = artifact("task902-fresh.abc", None)
    (fresh_work / "log").write_text("running")
    linked_work = cache / "task903-link"
    linked_work.symlink_to(outside, target_is_directory=True)
    for path in (old_work, old_work / "log", old_fixture, named_work, kept_work, kept_work / ".keep"):
        os.utime(path, (past, past))
    result = run("24")
    assert result.returncode == 0, result.stderr
    assert not old_work.exists() and not old_fixture.exists(), result.stdout
    assert named_work.is_dir() and kept_work.is_dir() and fresh_work.is_dir(), result.stdout
    assert linked_work.is_symlink() and (outside / "keep").read_text() == "foreign"

    # Anything above the limit fails the gate, snapshots included.
    limit_script = root / "scripts/utils/limit-prune.sh"
    limit_script.write_text(script.read_text().replace("LIMIT_KIB=$((23 * 1024 * 1024 / 10))", "LIMIT_KIB=2048"))
    ok = subprocess.run(["bash", str(limit_script), "24"], env=environment, text=True, capture_output=True)
    assert ok.returncode == 0, ok.stderr
    for bulky in (foreign, snapshot):
        (bulky / "bulk.bin").write_bytes(b"0" * (4 * 1024 * 1024))
        over = subprocess.run(["bash", str(limit_script), "24"], env=environment, text=True, capture_output=True)
        assert over.returncode == 1 and "exceeds 2.3 GiB" in over.stderr, over.stderr
        (bulky / "bulk.bin").unlink()
    limit_script.unlink()

    linked_project = Path(temporary) / "linked-project"
    linked_script = linked_project / "scripts/utils/prune-cache.sh"
    linked_script.parent.mkdir(parents=True)
    shutil.copyfile(script, linked_script)
    (linked_project / ".cache").symlink_to(outside, target_is_directory=True)
    result = subprocess.run(["bash", str(linked_script), "0"], env=environment, text=True, capture_output=True)
    assert result.returncode != 0 and "symlink" in result.stderr
    assert (outside / "keep").read_text() == "foreign"
    (linked_project / ".cache").unlink()
    (linked_project / ".cache").symlink_to(linked_project / "missing", target_is_directory=True)
    result = subprocess.run(["bash", str(linked_script), "0"], env=environment, text=True, capture_output=True)
    assert result.returncode != 0 and "symlink" in result.stderr
    (linked_project / ".cache").unlink()
    (linked_project / ".cache").write_text("foreign cache file")
    result = subprocess.run(["bash", str(linked_script), "0"], env=environment, text=True, capture_output=True)
    assert result.returncode != 0 and "not a directory" in result.stderr
    assert (linked_project / ".cache").read_text() == "foreign cache file"

    # Only completed, aged runs can lose bulk data; interrupted work stays intact.
    compact_project = work / "compact"
    compact_script = compact_project / "scripts/utils/prune-cache.sh"
    compact_script.parent.mkdir(parents=True)
    shutil.copyfile(repo / "scripts/utils/prune-cache.sh", compact_script)
    runs = compact_project / ".cache/evaluation"
    for age, name in enumerate(("sealed.new", "sealed.mid", "sealed.old", "sealed.pinned")):
        run = runs / name
        (run / "corpus").mkdir(parents=True)
        (run / "corpus/source.txt").write_text("bulk")
        (run / "report.json").write_text("{}")
        (run / "index.bin").write_bytes(b"0" * (9 * 1024 * 1024))
        (run / ".keep").write_text("")
        (run / marker).write_bytes(b"xcalygraph-cache-v1\n")
        stamp = time.time() - (60 * (age + 1) if age < 2 else 3600 * 48)
        os.utime(run, (stamp, stamp))
    (runs / "sealed.pinned/.keep-full").write_text("")
    os.utime(runs / "sealed.pinned", (time.time() - 3600 * 9, time.time() - 3600 * 9))
    for name, completion, age_seconds in (
        ("sealed.incomplete", None, 3600 * 72),
        ("sealed.invalid", b"not completed\n", 3600 * 72),
        ("sealed.fresh", b"xcalygraph-cache-v1\n", 300),
    ):
        run = runs / name
        (run / "corpus").mkdir(parents=True)
        (run / "corpus/source.txt").write_text("live data")
        if completion is not None:
            (run / marker).write_bytes(completion)
        stamp = time.time() - age_seconds
        os.utime(run, (stamp, stamp))
    result = subprocess.run(["bash", str(compact_script)], env=environment, text=True, capture_output=True)
    assert result.returncode == 0, result.stderr
    for name in ("sealed.new", "sealed.mid", "sealed.pinned"):
        assert (runs / name / "index.bin").is_file() and (runs / name / "corpus").is_dir(), name
    old = runs / "sealed.old"
    assert (old / "report.json").read_text() == "{}" and (old / ".keep").exists() and (old / ".compacted").exists()
    assert not (old / "index.bin").exists() and not (old / "corpus").exists()
    for name in ("sealed.incomplete", "sealed.invalid", "sealed.fresh"):
        run = runs / name
        assert (run / "corpus/source.txt").read_text() == "live data", name
        assert not (run / ".compacted").exists(), name
    result = subprocess.run(["bash", str(compact_script), "0"], env=environment, text=True, capture_output=True)
    assert result.returncode == 0, result.stderr
    assert not (runs / "sealed.fresh/corpus").exists()
    for name in ("sealed.incomplete", "sealed.invalid"):
        assert (runs / name / "corpus/source.txt").read_text() == "live data", name
    print("PASS: age bounds, marked ownership, protected evidence, links, foreign data, size reporting and run compaction")
PYTHON
