#!/usr/bin/env bash
# Fail when extraction cost grows faster than linearly with file size; the gate runs this so a
# per-reference or per-import rescan cannot pass unnoticed (the instruction guard sees only its
# blessed corpus). Per language one generated file at size N and 4N: linear work retires about
# 4x the instructions, a quadratic pass about 16x; above MAX_RATIO the check fails.
# Usage: scripts/benchmarks/check-scaling.sh BINARY
# Where /usr/bin/time reports no retired instructions (Linux), it says so and passes.
set -euo pipefail

(( $# == 1 )) && [[ -x "$1" ]] || { echo "usage: check-scaling.sh BINARY" >&2; exit 2; }
readonly binary="$1"
readonly MAX_RATIO=6
# GNU time (Linux) rejects -l; BSD time (macOS) reports retired instructions with it.
if ! /usr/bin/time -l true >/dev/null 2>&1; then
  echo "check-scaling.sh: /usr/bin/time -l is unavailable; scaling unchecked" >&2
  exit 0
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/xcalygraph-scaling.XXXXXXXX")"
readonly work
trap 'rm -rf -- "${work}"' EXIT

# One file per language with SCALE units of imports, functions, locals and calls.
python3 -I - "${work}" <<'PYTHON'
import pathlib
import sys

work = pathlib.Path(sys.argv[1])

def javascript(n):
    lines = [f"const m{i} = require('./dep{i}');" for i in range(n)]
    lines += [f"function f{i}(a, b) {{ const x = a + b * {i}; if (x > 2) {{ return m{i % n}.run(x); }} return f{max(i - 1, 0)}(x, b); }}" for i in range(n * 10)]
    return "\n".join(lines + ["module.exports = f0;"])

def typescript(n):
    lines = [f"import {{ dep{i} }} from './dep{i}';" for i in range(n)]
    lines += [f"export class C{i} {{ run(x: number): number {{ const y = dep{i % n}(x); return new C{max(i - 1, 0)}().run(y); }} }}" for i in range(n * 10)]
    return "\n".join(lines)

def python(n):
    lines = [f"import dep{i}" for i in range(n)]
    lines += [f"def f{i}(a, b=None):\n    x = dep{i % n}.run(a)\n    return f{max(i - 1, 0)}(x, b)" for i in range(n * 10)]
    return "\n".join(lines)

def rust(n):
    lines = [f"use crate::dep{i}::Item{i};" for i in range(n)]
    lines += [f"pub fn f{i}(a: u32) -> u32 {{ let x = Item{i % n}::run(a); f{max(i - 1, 0)}(x) }}" for i in range(n * 10)]
    return "\n".join(lines)

def go(n):
    lines = ["package app", "import ("] + [f'\tdep{i} "example.com/dep{i}"' for i in range(n)] + [")"]
    lines += [f"func F{i}(a int) int {{ x := dep{i % n}.Run(a); return F{max(i - 1, 0)}(x) }}" for i in range(n * 10)]
    return "\n".join(lines)

def lua(n):
    lines = [f"local m{i} = require('dep{i}')" for i in range(n)]
    lines += [f"local function f{i}(a)\n  local x = m{i % n}.run(a)\n  if x > 2 then x = x - 1 end\n  return f{max(i - 1, 0)}(x)\nend" for i in range(n * 10)]
    return "\n".join(lines + ["return { f0 = f0 }"])

for name, extension, generate in [
    ("javascript", "js", javascript), ("typescript", "ts", typescript), ("python", "py", python),
    ("rust", "rs", rust), ("go", "go", go), ("lua", "lua", lua),
]:
    for scale, size in (("small", 50), ("large", 200)):
        root = work / f"{name}-{scale}"
        root.mkdir()
        (root / f"main.{extension}").write_text(generate(size) + "\n")
PYTHON

# Retired instructions of one fresh build, empty when the platform does not report them.
measure() {
  rm -rf -- "${work}/index"
  XCALYGRAPH_INDEX_DIR="${work}/index" /usr/bin/time -l "${binary}" index -C "$1" 2>&1 >/dev/null |
    awk '/instructions retired/ { print $1 }'
}

failed=0
for language in javascript typescript python rust go lua; do
  small="$(measure "${work}/${language}-small")"
  large="$(measure "${work}/${language}-large")"
  if [[ -z "${small}" || -z "${large}" ]]; then
    echo "check-scaling.sh: no retired-instruction count on this platform; scaling unchecked" >&2
    exit 0
  fi
  if (( large > small * MAX_RATIO )); then
    # One retry of both sizes: a single run can pay for cold pages.
    small="$(measure "${work}/${language}-small")"
    large="$(measure "${work}/${language}-large")"
  fi
  if (( large > small * MAX_RATIO )); then
    echo "check-scaling.sh: ${language} extraction grows superlinearly: 4x input retires $(( large * 10 / small / 10 )).$(( large * 10 / small % 10 ))x instructions (limit ${MAX_RATIO}x)" >&2
    failed=1
  fi
done
exit "${failed}"
