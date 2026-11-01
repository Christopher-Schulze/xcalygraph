#!/usr/bin/env bash
# Check that this process inherited the held lock for this checkout's owned target.
# Usage: scripts/utils/verification-lock.sh
set -euo pipefail
(( $# == 0 )) || exit 2
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import fcntl
import hashlib
import os
from pathlib import Path
import stat
import sys

try:
    root = Path(sys.argv[1]).resolve()
    identity = hashlib.sha256(os.fsencode(root)).hexdigest()
    target = Path(os.environ["CARGO_TARGET_DIR"])
    descriptor = int(os.environ["XCALYGRAPH_VERIFICATION_FD"])
    if (descriptor < 100 or not target.is_absolute() or target.name != identity
            or target.parent.name != ".xcalygraph-verification" or target.is_symlink()
            or target.parent.is_symlink() or Path(os.environ["CARGO_BUILD_BUILD_DIR"]) != target):
        raise ValueError("invalid verification target")
    marker = target / ".xcalygraph-verification-owner"
    info = marker.lstat()
    expected = ("xcalygraph-verification-v1\n" + identity + "\n").encode()
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or marker.read_bytes() != expected:
        raise ValueError("invalid verification ownership")
    state = target / ".xcalygraph-verification-source"
    info = state.lstat()
    held = os.fstat(descriptor)
    if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
            or (info.st_dev, info.st_ino) != (held.st_dev, held.st_ino)):
        raise ValueError("invalid inherited lock")
    probe = os.open(state, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        try:
            fcntl.flock(probe, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            pass
        else:
            fcntl.flock(probe, fcntl.LOCK_UN)
            raise ValueError("verification lock is not held")
    finally:
        os.close(probe)
except (KeyError, ValueError, OSError):
    sys.exit(1)
PYTHON
