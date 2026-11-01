#!/usr/bin/env bash
# Select an owned Cargo target unique to this checkout for verification.
# Usage: scripts/utils/verification-target.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: verification-target.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
cd "${REPO_ROOT}"
cargo metadata --locked --no-deps --format-version 1 | python3 -I -c '
import hashlib
import json
import os
from pathlib import Path
import stat
import sys

try:
    root = Path(sys.argv[1]).resolve()
    identity = hashlib.sha256(os.fsencode(root)).hexdigest()
    base = Path(json.load(sys.stdin)["target_directory"]).resolve()
    category = ".xcalygraph-verification"
    if base.name == identity and base.parent.name == category:
        namespace, target = base.parent, base
    else:
        namespace, target = base / category, base / category / identity
    if namespace.is_symlink():
        raise ValueError("verification target namespace is a symlink")
    namespace.mkdir(parents=True, exist_ok=True)
    try:
        target.mkdir()
        created = True
    except FileExistsError:
        created = False
    if target.is_symlink() or not target.is_dir():
        raise ValueError("verification target ownership is unverified")
    marker = target / ".xcalygraph-verification-owner"
    expected = ("xcalygraph-verification-v1\n" + identity + "\n").encode()
    if created:
        with (target / "CACHEDIR.TAG").open("xb") as stream:
            stream.write(b"Signature: 8a477f597d28d172789f06886806bc55\n")
        with marker.open("xb") as stream:
            stream.write(expected)
    else:
        try:
            info = marker.lstat()
        except FileNotFoundError:
            raise ValueError("verification target ownership is unverified") from None
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or marker.read_bytes() != expected:
            raise ValueError("verification target ownership is unverified")
    print(target)
except (OSError, ValueError, KeyError, TypeError) as error:
    print("verification-target.sh: " + str(error), file=sys.stderr)
    sys.exit(1)
' "${REPO_ROOT}"
