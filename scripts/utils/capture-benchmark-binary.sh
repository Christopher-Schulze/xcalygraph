#!/usr/bin/env bash
# Build and copy a benchmark executable while the checkout's verification lock is held.
# Usage: capture-benchmark-binary.sh --debug|--release NEW_DESTINATION
set -euo pipefail
if (( $# != 2 )) || [[ "$1" != --debug && "$1" != --release ]]; then
  echo "usage: capture-benchmark-binary.sh --debug|--release NEW_DESTINATION" >&2
  exit 2
fi
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if ! bash "${REPO_ROOT}/scripts/utils/verification-lock.sh"; then
  echo "capture-benchmark-binary.sh: verification lock is required" >&2
  exit 1
fi
profile="$1"
destination="$2"
build_args=()
[[ "${profile}" == --release ]] && build_args+=(--release)
cd "${REPO_ROOT}"
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
binary="$(RUSTFLAGS="-Dwarnings" cargo build "${build_args[@]}" --locked --bin xcalygraph --message-format=json -q \
  | bash "${REPO_ROOT}/scripts/utils/cargo-artifact.sh")"
python3 -I - "${binary}" "${destination}" "${profile#--}" \
  "${REPO_ROOT}/skills/xcalygraph/SKILL.md" "${CARGO_TARGET_DIR}/.xcalygraph-verification-source" <<'PYTHON'
import hashlib
import json
import os
from pathlib import Path
import stat
import sys

source, destination, skill, journal = map(Path, (sys.argv[1], sys.argv[2], sys.argv[4], sys.argv[5]))
profile = sys.argv[3]
with source.open("rb") as original:
    before = os.fstat(original.fileno())
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("Cargo executable is not a regular file")
    descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o500)
    digest = hashlib.sha256()
    with os.fdopen(descriptor, "wb") as copied:
        for block in iter(lambda: original.read(1024 * 1024), b""):
            digest.update(block)
            copied.write(block)
        os.fchmod(copied.fileno(), 0o500)
    after = os.fstat(original.fileno())
    if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
        raise ValueError("Cargo executable changed during capture")
copied = destination.lstat()
if not stat.S_ISREG(copied.st_mode) or copied.st_nlink != 1:
    raise ValueError("captured executable ownership is unverified")
with destination.open("rb") as stream:
    copied_hash = hashlib.sha256()
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        copied_hash.update(block)
    copied_digest = copied_hash.hexdigest()
if copied_digest != digest.hexdigest():
    raise ValueError("captured executable differs from Cargo's executable")
line = journal.read_bytes().splitlines()[-1]
if not line.startswith(b"start "):
    raise ValueError("verification source identity is unavailable")
identity = json.loads(line[6:])
print(json.dumps({"binary_sha256": copied_digest, "skill_sha256": hashlib.sha256(skill.read_bytes()).hexdigest(),
                  "source": identity, "profile": profile,
                  "binary_stat": [copied.st_dev, copied.st_ino, copied.st_size, copied.st_mtime_ns,
                                  copied.st_ctime_ns, stat.S_IMODE(copied.st_mode), copied.st_nlink]}, sort_keys=True))
PYTHON
