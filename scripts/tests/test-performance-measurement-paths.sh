#!/usr/bin/env bash
# Exercise real disk-size and rewritten-byte measurements on unusual filesystem paths.
# Usage: test-performance-measurement-paths.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-performance-measurement-paths.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
runner = repo / "scripts/benchmarks/run-benchmarks.sh"
with tempfile.TemporaryDirectory(prefix="performance-measurements.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    shells = list(dict.fromkeys((shutil.which("bash"), "/bin/bash")))
    assert all(shells)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(work), XDG_CONFIG_HOME=str(work / "config"))

    def invoke(shell, function, *arguments):
        return subprocess.run([shell, "-c", 'source "$1"; shift; "$@"', "measurement", str(runner), function,
                               *map(str, arguments)], cwd=work, env=environment, capture_output=True)

    for number, shell in enumerate(shells):
        for name in (f"plain-{number}", f"line-{number}\n123", f"tab-{number}\tand\\slash", f"-dash-{number}"):
            directory = work / name
            directory.mkdir()
            (directory / "data").write_bytes(b"size fixture" * 400)
            expected = subprocess.run(["du", "-sk", "--", name], cwd=work, env=environment,
                                      capture_output=True, check=True).stdout.split(b"\t", 1)[0]
            actual = invoke(shell, "directory_kb", name)
            assert actual.returncode == 0 and actual.stdout.strip() == expected and actual.stdout.count(b"\n") == 1, actual.stderr
        missing = invoke(shell, "directory_kb", f"absent-{number}")
        assert missing.returncode != 0 and not missing.stdout

        index = work / (f"index-{number}\n123")
        index.mkdir()
        for subdirectory in ("left", "right"):
            (index / subdirectory).mkdir()
        stable = index / "stable"
        stable.write_bytes(b"untouched")
        changed = index / "line\n123 999999 suffix"
        changed.write_bytes(b"before")
        resized = index / "tab\tand\\slash"
        resized.write_bytes(b"a")
        (index / "left/same").write_bytes(b"original value")
        (index / "right/same").write_bytes(b"decoy")
        removed = index / "removed"
        removed.write_bytes(b"remove")
        outside = work / f"outside-{number}"
        outside.write_bytes(b"outside preserved")
        (index / "linked").symlink_to(outside)
        snapshot = work / f"snapshot-{number}.json"
        initial = invoke(shell, "measure_index_files", "snapshot", index, snapshot)
        assert initial.returncode == 0, initial.stderr
        original = snapshot.read_bytes()
        before = json.loads(original)
        assert set(before["files"]) == {"stable", "line\n123 999999 suffix", "tab\tand\\slash", "removed",
                                        "left/same", "right/same"}
        duplicate = invoke(shell, "measure_index_files", "snapshot", index, snapshot)
        assert duplicate.returncode != 0 and snapshot.read_bytes() == original

        replacement = work / f"replacement-{number}"
        replacement.write_bytes(b"changed-bytes")
        replacement.replace(changed)
        with resized.open("ab") as stream:
            stream.write(b"more")
        replacement = work / f"nested-replacement-{number}"
        replacement.write_bytes(b"changed nested")
        replacement.replace(index / "left/same")
        removed.unlink()
        created = index / "new\n42 123456 suffix"
        created.write_bytes(b"new file")
        expected_bytes = sum(path.stat().st_size for path in (changed, resized, index / "left/same", created))
        measured = invoke(shell, "measure_index_files", "bytes", index, snapshot)
        assert measured.returncode == 0 and measured.stdout.strip() == str(expected_bytes).encode(), measured.stderr
        after = Path(str(snapshot) + ".after")
        assert after.is_file() and "linked" not in json.loads(after.read_text())["files"]
        after_bytes = after.read_bytes()
        assert stable.read_bytes() == b"untouched" and (index / "right/same").read_bytes() == b"decoy"
        assert outside.read_bytes() == b"outside preserved"
        repeated = invoke(shell, "measure_index_files", "bytes", index, snapshot)
        assert repeated.returncode != 0 and after.read_bytes() == after_bytes

        other = work / f"other-index-{number}"
        other.mkdir()
        wrong_snapshot = work / f"wrong-root-{number}.json"
        shutil.copyfile(snapshot, wrong_snapshot)
        wrong_root = invoke(shell, "measure_index_files", "bytes", other, wrong_snapshot)
        assert wrong_root.returncode != 0 and not wrong_root.stdout and not Path(str(wrong_snapshot) + ".after").exists()
        malformed = work / f"invalid-snapshot-{number}.json"
        malformed.write_text("not JSON\n")
        bad = invoke(shell, "measure_index_files", "bytes", index, malformed)
        assert bad.returncode != 0 and not bad.stdout and not Path(str(malformed) + ".after").exists()
        for position, files in enumerate(({"../escape": [1, 2, 3]}, {"file": [1, -1, 3]}, [])):
            invalid = work / f"invalid-record-{number}-{position}.json"
            invalid.write_text(json.dumps({"root": str(index), "files": files}))
            result = invoke(shell, "measure_index_files", "bytes", index, invalid)
            assert result.returncode != 0 and not result.stdout and not Path(str(invalid) + ".after").exists()
        absent = work / f"absent-snapshot-{number}.json"
        not_found = invoke(shell, "measure_index_files", "snapshot", work / f"not-an-index-{number}", absent)
        assert not_found.returncode != 0 and not absent.exists()
        linked = work / f"linked-index-{number}"
        linked.symlink_to(index, target_is_directory=True)
        unsafe = invoke(shell, "measure_index_files", "snapshot", linked, absent)
        assert unsafe.returncode != 0 and not absent.exists()
        unreadable = index / "unreadable"
        unreadable.mkdir()
        unreadable.chmod(0)
        try:
            if not os.access(unreadable, os.R_OK | os.X_OK):
                inaccessible = invoke(shell, "measure_index_files", "snapshot", index, absent)
                assert inaccessible.returncode != 0 and not absent.exists(), inaccessible.stderr
        finally:
            unreadable.chmod(0o700)
    source = runner.read_text()
    start = source.index("upstream_columns() {\n")
    end = source.index("\n}\n", start) + 3
    function = source[start:end]
    tools, upstream_home, copy = work / "upstream-tools", work / "upstream-home", work / "upstream-copy\n"
    tools.mkdir()
    upstream_home.mkdir()
    copy.mkdir()
    target = copy / "main.rs"
    content = "pub fn alpha() { beta(); }\npub fn beta() {}\n"
    target.write_text(content)
    for name in ("graphify", "python3", "dirname", "env", "git", "rg", "basename", "awk", "rm", "sh", "tee", "sort", "find", "stat"):
        executable = shutil.which(name)
        if executable:
            (tools / name).symlink_to(executable)
    assert shutil.which("du", path=str(tools)) is None
    upstream_env = dict(environment, HOME=str(upstream_home), PATH=str(tools), XDG_CONFIG_HOME=str(upstream_home / "config"))
    code = ('source "$1"; BENCHMARK_HOME="$2"\n' + function +
            '\nif upstream_columns "$3" "$4" $\'\\n// bench\\n\'; then exit 0; else exit $?; fi')
    upstream = subprocess.run([shells[0], "-c", code, "upstream", str(runner), str(upstream_home), str(copy), str(target)],
                              cwd=work, env=upstream_env, capture_output=True, timeout=120)
    assert upstream.returncode != 0 and not upstream.stdout and b"du:" in upstream.stderr, upstream.stderr
    assert (copy / "graphify-out/graph.json").is_file() and target.read_text() == content
    print("PASS: real du and exact inode/size snapshots preserve unusual paths and reject invalid evidence")
PYTHON
