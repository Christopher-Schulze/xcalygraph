#!/usr/bin/env bash
# Measure cache-hot, single-thread copying of the derived index's symbol section.
# Usage: measure-symbol-copy.sh DERIVED
set -euo pipefail

if (( $# != 1 )); then
  echo "usage: measure-symbol-copy.sh DERIVED" >&2
  exit 2
fi

python3 -I - "$1" <<'PYTHON'
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import struct
import sys
import time

data = Path(sys.argv[1]).read_bytes()
if (len(data) < 384 or data[:8] != b"XCALYGR\x00"
        or struct.unpack_from("<III", data, 8) != (7, 4, 20)
        or struct.unpack_from("<Q", data, 24)[0] != len(data) - 64):
    raise ValueError("unexpected derived index format")
offset, size = struct.unpack_from("<QQ", data, 64 + 3 * 16)
if not size or offset + size > len(data):
    raise ValueError("invalid symbol section")
source = (ctypes.c_ubyte * size).from_buffer_copy(data[offset:offset + size])
target = (ctypes.c_ubyte * size)()
for _ in range(10):
    ctypes.memmove(target, source, size)
timings = []
for _ in range(5):
    started = time.perf_counter_ns()
    for _ in range(300):
        ctypes.memmove(target, source, size)
    timings.append((time.perf_counter_ns() - started) / 300)
if bytes(source) != bytes(target):
    raise ValueError("symbol copy mismatch")
median = statistics.median(timings)
print(json.dumps({"architecture": platform.machine(), "load": os.getloadavg(),
                  "derived_sha256": hashlib.sha256(data).hexdigest(), "symbol_bytes": size,
                  "iterations_per_trial": 300, "trials": 5, "median_ns_per_copy": median,
                  "logical_gb_per_second": size / median,
                  "limitations": "One repeatedly cached section is read and written on one thread; this is not a strict query lower bound or sustained RAM bandwidth."},
                 sort_keys=True))
PYTHON
