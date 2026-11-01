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
import json
import os
from collections import Counter, defaultdict
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

def normalized(name):
    return name.replace("-", "_").casefold()

def tree_paths(directory):
    if directory.is_symlink():
        raise ValueError(f"source directory is a symlink: {directory}")
    if not directory.exists():
        return set()
    paths = set()
    for parent, directories, files in os.walk(directory, followlinks=False, onerror=walk_error):
        for entry in directories[:]:
            child = Path(parent) / entry
            if child.is_symlink():
                raise ValueError(f"source directory is a symlink: {child}")
            if entry == ".git" or child == target:
                directories.remove(entry)
        paths.update((Path(parent) / entry).relative_to(root) for entry in files)
    return paths

def hash_paths(paths):
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
    return digest.hexdigest()

def snapshot():
    result = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
                            cwd=root, check=True, stdout=subprocess.PIPE)
    paths = {Path(os.fsdecode(path)) for path in result.stdout.split(b"\0") if path}
    for name in ("src", "tests", "skills", ".cargo"):
        paths.update(tree_paths(root / name))
    if (root / "build.rs").exists():
        paths.add(Path("build.rs"))
    metadata = subprocess.run(["cargo", "metadata", "--locked", "--format-version", "1"],
                              cwd=root, check=True, stdout=subprocess.PIPE)
    graph = json.loads(metadata.stdout)["packages"]
    counts = Counter(normalized(package["name"]) for package in graph)
    targets = defaultdict(set)
    for package in graph:
        for item in package["targets"]:
            if "custom-build" not in item["kind"]:
                targets[normalized(item["name"])].add(package["id"])
    packages = {}
    root_package = None
    for package in graph:
        if package["source"] is not None:
            continue
        if counts[normalized(package["name"])] != 1:
            raise ValueError(f"ambiguous local Cargo package: {package['name']}")
        for item in package["targets"]:
            if "custom-build" not in item["kind"] and len(targets[normalized(item["name"])]) != 1:
                raise ValueError(f"ambiguous local Cargo target: {item['name']}")
        manifest = Path(package["manifest_path"]).resolve(strict=True)
        if manifest == root / "Cargo.toml":
            root_package = package["name"]
            continue
        if not manifest.is_relative_to(root):
            raise ValueError(f"local Cargo package is outside the repository: {manifest}")
        packages[package["name"]] = hash_paths(tree_paths(manifest.parent))
    if root_package is None:
        raise ValueError("Cargo metadata did not identify the root package")
    return {"source": hash_paths(paths), "root": root_package, "packages": packages}

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
        current = snapshot()
        encoded = json.dumps(current, sort_keys=True, separators=(",", ":")).encode()
        lines = data.splitlines() if data.endswith(b"\n") else []
        previous = None
        if len(lines) >= 3 and lines[-1].startswith(b"ok ") and lines[-2] == b"start " + lines[-1][3:]:
            try:
                previous = json.loads(lines[-1][3:])
                if (not isinstance(previous, dict) or set(previous) != {"source", "root", "packages"}
                        or previous["root"] != current["root"] or not isinstance(previous["source"], str)
                        or not isinstance(previous["packages"], dict)
                        or not all(isinstance(key, str) and isinstance(value, str)
                                   for key, value in previous["packages"].items())):
                    previous = None
            except (json.JSONDecodeError, UnicodeDecodeError):
                pass
        reusable = previous == current
        if not data.endswith(b"\n"):
            append(state, b"")
        append(state, b"start " + encoded)
        if not reusable:
            print("verification-source.sh: invalidating project artifacts", file=sys.stderr, flush=True)
            old_packages = previous["packages"] if previous is not None else {}
            changed = [package for package, digest in current["packages"].items()
                       if old_packages.get(package) != digest]
            arguments = ["--package", current["root"]]
            for package in changed:
                arguments.extend(("--package", package))
            subprocess.run(["cargo", "clean", "--locked", *arguments],
                           cwd=root, check=True, pass_fds=(state.fileno(),))
        result = subprocess.run(sys.argv[2:], cwd=root, pass_fds=(state.fileno(),))
        if result.returncode != 0:
            raise SystemExit(result.returncode if result.returncode > 0 else 128 - result.returncode)
        if snapshot() != current:
            raise ValueError("verification inputs changed during the gate")
        append(state, b"ok " + encoded)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print("verification-source.sh: " + str(error), file=sys.stderr)
        raise SystemExit(1) from error
PYTHON
