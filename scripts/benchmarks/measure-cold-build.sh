#!/usr/bin/env bash
# Build wall, user and sys time with file contents evicted from the page cache, binaries alternating.
# Usage: scripts/benchmarks/measure-cold-build.sh [--runs N] ROOT BINARY...
#   Eviction: mmap plus msync(MS_INVALIDATE) on every regular file below ROOT up to 64 MiB, no
#   privilege needed. Limit: directory metadata stays cached, and files mapped elsewhere stay resident.
#   Output: one line per run and the medians per binary.
set -euo pipefail
python3 -I - "$@" <<'PYTHON'
import ctypes
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
    sys.exit("usage: measure-cold-build.sh [--runs N] ROOT BINARY...")
root, binaries = arguments[0], arguments[1:]
libc = ctypes.CDLL(None, use_errno=True)
MS_INVALIDATE = 2

libc.mmap.restype = ctypes.c_void_p
libc.mmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_long]
libc.msync.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int]
libc.munmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t]

def evict():
    start = time.perf_counter()
    count = 0
    for directory, _, files in os.walk(root):
        for name in files:
            path = os.path.join(directory, name)
            try:
                if os.path.islink(path):
                    continue
                size = os.path.getsize(path)
                if not 0 < size <= 64 << 20:
                    continue
                descriptor = os.open(path, os.O_RDONLY)
            except OSError:
                continue
            address = libc.mmap(None, size, mmap.PROT_READ, mmap.MAP_SHARED, descriptor, 0)
            if address not in (None, ctypes.c_void_p(-1).value):
                libc.msync(address, size, MS_INVALIDATE)
                libc.munmap(address, size)
                count += 1
            os.close(descriptor)
    return count, time.perf_counter() - start

results = {binary: [] for binary in binaries}
for run in range(runs):
    for binary in binaries:
        count, seconds = evict()
        with tempfile.TemporaryDirectory(prefix="cold-index.", dir=os.environ.get("COLD_INDEX_BASE")) as index:
            before = resource.getrusage(resource.RUSAGE_CHILDREN)
            start = time.perf_counter()
            subprocess.run([binary, "index", "-C", root], check=True, stdout=subprocess.DEVNULL,
                           env=dict(os.environ, XCALYGRAPH_INDEX_DIR=index))
            wall = time.perf_counter() - start
            after = resource.getrusage(resource.RUSAGE_CHILDREN)
        row = (wall, after.ru_utime - before.ru_utime, after.ru_stime - before.ru_stime)
        results[binary].append(row)
        print(f"{os.path.basename(binary)}\trun {run}\tevicted {count} files in {seconds:.0f} s\twall {row[0]:.1f}\tuser {row[1]:.1f}\tsys {row[2]:.1f}", flush=True)
for binary, rows in results.items():
    print(f"{os.path.basename(binary)}\tmedian\twall {statistics.median(r[0] for r in rows):.1f}\tuser {statistics.median(r[1] for r in rows):.1f}\tsys {statistics.median(r[2] for r in rows):.1f}")
PYTHON
