#!/usr/bin/env bash
# Exercise the instruction guard with a stand-in binary in an exclusively created fixture.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly GUARD="${REPO_ROOT}/scripts/benchmarks/check-instructions.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/xcalygraph-guard-test.XXXXXXXX")"
trap 'rm -rf -- "${work}"' EXIT
mkdir "${work}/root"
printf '#!/bin/sh\nexit 0\n' > "${work}/binary"
chmod +x "${work}/binary"
export XCALYGRAPH_INSTRUCTION_BASELINE="${work}/baseline.tsv"

# No baseline: passes silently.
bash "${GUARD}" "${work}/binary"
/usr/bin/time -l true 2>&1 | grep -q 'instructions retired' || { echo "PASS: instruction guard (no counter)"; exit 0; }

bash "${GUARD}" --bless "${work}/binary" "${work}/root"
blessed="$(cut -f2 "${XCALYGRAPH_INSTRUCTION_BASELINE}")"
(( blessed > 0 )) || { echo "no blessed count" >&2; exit 1; }
printf '%s\t%s\n' "${work}/root" "$(( blessed * 100 ))" > "${XCALYGRAPH_INSTRUCTION_BASELINE}"
bash "${GUARD}" "${work}/binary"

# A count above the limit fails with the corpus name.
printf '%s\t%s\n' "${work}/root" 1 > "${XCALYGRAPH_INSTRUCTION_BASELINE}"
if bash "${GUARD}" "${work}/binary" 2> "${work}/stderr"; then
  echo "regression passed the guard" >&2
  exit 1
fi
grep -q 'more instructions than the baseline on root' "${work}/stderr"

# A vanished root is skipped; a usage error is an error.
printf '%s\t%s\n' "${work}/missing" 1 > "${XCALYGRAPH_INSTRUCTION_BASELINE}"
bash "${GUARD}" "${work}/binary"
if bash "${GUARD}" "${work}/absent-binary" 2>/dev/null; then
  echo "missing binary passed" >&2
  exit 1
fi
echo "PASS: instruction guard"
