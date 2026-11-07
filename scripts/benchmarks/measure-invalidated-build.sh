#!/usr/bin/env bash
# Build wall, user and sys time after attempted file-cache invalidation, binaries alternating.
# Usage: scripts/benchmarks/measure-invalidated-build.sh [--runs N] ROOT BINARY...
#   Attempt: mmap plus msync(MS_INVALIDATE) on files below ROOT up to 64 MiB, no privilege needed.
#   Successful syscalls do not prove eviction: metadata stays cached and resident pages may remain.
#   Output: one line per run and the medians per binary over valid runs. A run is valid only when
#   every file was opened, mapped, invalidated and unmapped (files above 64 MiB count as skipped);
#   without a valid run for a binary the script exits 1.
set -euo pipefail
python3 -I - "$@" <<'PYTHON'
import ctypes
from collections import Counter
import mmap
import os
import resource
import statistics
import subprocess
import sys
import tempfile
import time

arguments = sys.argv[1:]
runs = 3
if arguments[:1] == ["--runs"]:
    runs = int(arguments[1])
    arguments = arguments[2:]
if len(arguments) < 2:
    sys.exit("usage: measure-invalidated-build.sh [--runs N] ROOT BINARY...")
root, binaries = arguments[0], arguments[1:]
libc = ctypes.CDLL(None, use_errno=True)
MS_INVALIDATE = 2

libc.mmap.restype = ctypes.c_void_p
libc.mmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_long]
libc.msync.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int]
libc.msync.restype = ctypes.c_int
libc.munmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
libc.munmap.restype = ctypes.c_int

def evict():
    start = time.perf_counter()
    counts = dict(mapped=0, map_failed=0, invalidate_ok=0, invalidate_failed=0, unmap_failed=0, open_failed=0, skipped_large=0)
    errors = Counter()
    for directory, _, files in os.walk(root):
        for name in files:
            path = os.path.join(directory, name)
            try:
                if os.path.islink(path):
                    continue
                size = os.path.getsize(path)
                if size > 64 << 20:
                    counts["skipped_large"] += 1
                    continue
                if size == 0:
                    continue
                descriptor = os.open(path, os.O_RDONLY)
            except OSError as error:
                counts["open_failed"] += 1
                errors[error.errno] += 1
                continue
            try:
                address = libc.mmap(None, size, mmap.PROT_READ, mmap.MAP_SHARED, descriptor, 0)
                if address in (None, ctypes.c_void_p(-1).value):
                    counts["map_failed"] += 1
                    errors[ctypes.get_errno()] += 1
                    continue
                counts["mapped"] += 1
                try:
                    if libc.msync(address, size, MS_INVALIDATE) == 0:
                        counts["invalidate_ok"] += 1
                    else:
                        counts["invalidate_failed"] += 1
                        errors[ctypes.get_errno()] += 1
                finally:
                    if libc.munmap(address, size) != 0:
                        counts["unmap_failed"] += 1
                        errors[ctypes.get_errno()] += 1
            finally:
                os.close(descriptor)
    return counts, errors, time.perf_counter() - start

results = {binary: [] for binary in binaries}
print("cache_state unproven; metadata stays cached; successful invalidation calls do not prove page eviction", flush=True)
for run in range(runs):
    for binary in binaries:
        counts, errors, seconds = evict()
        with tempfile.TemporaryDirectory(prefix="invalidated-index.", dir=os.environ.get("INVALIDATED_INDEX_BASE")) as index:
            before = resource.getrusage(resource.RUSAGE_CHILDREN)
            start = time.perf_counter()
            subprocess.run([binary, "index", "-C", root], check=True, stdout=subprocess.DEVNULL,
                           env=dict(os.environ, XCALYGRAPH_INDEX_DIR=index))
            wall = time.perf_counter() - start
            after = resource.getrusage(resource.RUSAGE_CHILDREN)
        row = (wall, after.ru_utime - before.ru_utime, after.ru_stime - before.ru_stime)
        valid = all(counts[key] == 0 for key in ("map_failed", "invalidate_failed", "unmap_failed", "open_failed", "skipped_large"))
        if valid:
            results[binary].append(row)
        accounting = "\t".join(f"{key} {value}" for key, value in counts.items())
        failures = ",".join(f"{code}:{count}" for code, count in sorted(errors.items()))
        print(f"{os.path.basename(binary)}\trun {run}\t{accounting}\terrno={failures or 'none'}\tinvalidation_s {seconds:.3f}\twall {row[0]:.1f}\tuser {row[1]:.1f}\tsys {row[2]:.1f}\tvalid {'yes' if valid else 'no'}", flush=True)
status = 0
for binary, rows in results.items():
    if not rows:
        print(f"{os.path.basename(binary)}\tmedian\tn/a: no valid run")
        status = 1
        continue
    print(f"{os.path.basename(binary)}\tmedian\twall {statistics.median(r[0] for r in rows):.1f}\tuser {statistics.median(r[1] for r in rows):.1f}\tsys {statistics.median(r[2] for r in rows):.1f}\tvalid_runs {len(rows)}")
sys.exit(status)
PYTHON
