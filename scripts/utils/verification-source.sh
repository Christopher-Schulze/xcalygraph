#!/usr/bin/env bash
# Serialize an owned verification target and invalidate stale project artifacts by content.
# Usage: scripts/utils/verification-source.sh COMMAND [ARG...]
set -euo pipefail
(( $# > 0 )) || { echo "usage: verification-source.sh COMMAND [ARG...]" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"
python3 -I - "${REPO_ROOT}" "$@" <<'PYTHON'
import fcntl
import hashlib
import os
from pathlib import Path
import stat
import subprocess
import sys

root = Path(sys.argv[1]).resolve()
identity = hashlib.sha256(os.fsencode(root)).hexdigest()
target = Path(os.environ["CARGO_TARGET_DIR"])
if (target.name != identity or target.parent.name != ".xcalygraph-verification"
        or target.is_symlink() or target.parent.is_symlink()
        or Path(os.environ["CARGO_BUILD_BUILD_DIR"]) != target):
    raise SystemExit("verification-source.sh: target ownership or build directory is invalid")
marker = target / ".xcalygraph-verification-owner"
expected = ("xcalygraph-verification-v1\n" + identity + "\n").encode()
info = marker.lstat()
if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or marker.read_bytes() != expected:
    raise SystemExit("verification-source.sh: target ownership is unverified")

def walk_error(error):
    raise error

def fingerprint():
    result = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
                            cwd=root, check=True, stdout=subprocess.PIPE)
    paths = {Path(os.fsdecode(path)) for path in result.stdout.split(b"\0") if path}
    for name in ("src", "tests", "skills", ".cargo"):
        directory = root / name
        if directory.is_symlink():
            raise ValueError(f"source directory is a symlink: {name}")
        if not directory.exists():
            continue
        for parent, directories, files in os.walk(directory, followlinks=False, onerror=walk_error):
            for entry in directories:
                if (Path(parent) / entry).is_symlink():
                    raise ValueError(f"source directory is a symlink: {Path(parent) / entry}")
            paths.update((Path(parent) / entry).relative_to(root) for entry in files)
    if (root / "build.rs").exists():
        paths.add(Path("build.rs"))
    digest = hashlib.sha256()
    for relative in sorted(paths, key=os.fsencode):
        path = root / relative
        try:
            before = path.lstat()
        except FileNotFoundError:
            continue
        if not stat.S_ISREG(before.st_mode):
            raise ValueError(f"verification input is not a regular file: {relative}")
        content = path.read_bytes()
        after = path.lstat()
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
                after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            raise ValueError(f"verification input changed while hashing: {relative}")
        name = os.fsencode(relative)
        digest.update(len(name).to_bytes(8, "big"))
        digest.update(name)
        digest.update(stat.S_IMODE(before.st_mode).to_bytes(4, "big"))
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
    return digest.hexdigest().encode()

def append(stream, record):
    stream.seek(0, os.SEEK_END)
    stream.write(record + b"\n")
    stream.flush()
    os.fsync(stream.fileno())

state_path = target / ".xcalygraph-verification-source"
flags = os.O_RDWR | getattr(os, "O_NOFOLLOW", 0)
try:
    descriptor = os.open(state_path, flags | os.O_CREAT | os.O_EXCL, 0o600)
    created = True
except FileExistsError:
    try:
        descriptor = os.open(state_path, flags)
    except OSError as error:
        raise SystemExit("verification-source.sh: state ownership is unverified: " + str(error)) from error
    created = False
with os.fdopen(descriptor, "r+b") as state:
    info = os.fstat(state.fileno())
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise SystemExit("verification-source.sh: state ownership is unverified")
    fcntl.flock(state.fileno(), fcntl.LOCK_EX)
    header = b"xcalygraph-verification-source-v1\n"
    if created:
        append(state, header.rstrip(b"\n"))
    state.seek(0)
    data = state.read()
    if not data.startswith(header):
        raise SystemExit("verification-source.sh: state ownership is unverified")
    try:
        current = fingerprint()
        lines = data.splitlines() if data.endswith(b"\n") else []
        reusable = len(lines) >= 3 and lines[-2:] == [b"start " + current, b"ok " + current]
        if not data.endswith(b"\n"):
            append(state, b"")
        append(state, b"start " + current)
        if not reusable:
            print("verification-source.sh: invalidating project artifacts", file=sys.stderr, flush=True)
            subprocess.run(["cargo", "clean", "--locked", "--package", "xcalygraph"],
                           cwd=root, check=True, pass_fds=(state.fileno(),))
        result = subprocess.run(sys.argv[2:], cwd=root, pass_fds=(state.fileno(),))
        if result.returncode != 0:
            raise SystemExit(result.returncode if result.returncode > 0 else 128 - result.returncode)
        if fingerprint() != current:
            raise ValueError("verification inputs changed during the gate")
        append(state, b"ok " + current)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print("verification-source.sh: " + str(error), file=sys.stderr)
        raise SystemExit(1) from error
PYTHON
