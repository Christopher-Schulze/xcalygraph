#!/usr/bin/env bash
# Exercise the instruction guard with a stand-in binary in an exclusively created fixture.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly GUARD="${REPO_ROOT}/scripts/benchmarks/check-instructions.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/xcalygraph-guard-test.XXXXXXXX")"
trap 'rm -rf -- "${work}"' EXIT
mkdir "${work}/root-one" "${work}/root-two"
export XCALYGRAPH_GUARD_HOME_LOG="${work}/home.log"
# shellcheck disable=SC2016  # the stand-in binary expands its own variables
printf '%s\n' '#!/bin/sh' 'printf "%s\n" "$HOME" >> "$XCALYGRAPH_GUARD_HOME_LOG"' 'exit 0' > "${work}/binary"
chmod +x "${work}/binary"
export XCALYGRAPH_INSTRUCTION_BASELINE="${work}/baseline.tsv"

# No baseline: report that no measurement was made.
bash "${GUARD}" "${work}/binary" 2> "${work}/stderr"
grep -q 'no local baseline; instructions unchecked' "${work}/stderr"
/usr/bin/time -l true 2>&1 | grep 'instructions retired' >/dev/null || { echo "PASS: instruction guard (no counter)"; exit 0; }

bash "${GUARD}" --bless "${work}/binary" "${work}/root-one" "${work}/root-two"
blessed_one="$(sed -n '1s/^[^[:space:]]*[[:space:]]//p' "${XCALYGRAPH_INSTRUCTION_BASELINE}")"
blessed_two="$(sed -n '2s/^[^[:space:]]*[[:space:]]//p' "${XCALYGRAPH_INSTRUCTION_BASELINE}")"
(( blessed_one > 0 && blessed_two > 0 )) || { echo "no blessed counts" >&2; exit 1; }
python3 -I - "${XCALYGRAPH_GUARD_HOME_LOG}" <<'PYTHON'
from pathlib import Path
import sys

homes = Path(sys.argv[1]).read_text().splitlines()
if len(homes) != 4 or len(set(homes)) != 4 or any(
    "/.cache/instruction-check." not in home or not Path(home).name.startswith("run.")
    for home in homes
):
    raise SystemExit("instruction measurements did not use one isolated HOME per run")
if any(Path(home).exists() for home in homes):
    raise SystemExit("instruction guard left a temporary HOME or index behind")
PYTHON
cp "${XCALYGRAPH_INSTRUCTION_BASELINE}" "${work}/blessed.tsv"
if bash "${GUARD}" --bless "${work}/binary" "${work}/root-one" 2> "${work}/stderr"; then
  echo "blessing overwrote the existing baseline" >&2
  exit 1
fi
cmp -s "${work}/blessed.tsv" "${XCALYGRAPH_INSTRUCTION_BASELINE}" || {
  echo "failed blessing changed the existing baseline" >&2
  exit 1
}
printf '%s\t%s\n%s\t%s\n' "${work}/root-one" "$(( blessed_one * 100 ))" \
  "${work}/root-two" "$(( blessed_two * 100 ))" > "${XCALYGRAPH_INSTRUCTION_BASELINE}"
bash "${GUARD}" "${work}/binary"

# Regressions on both the first and last configured roots fail with that root's name.
printf '%s\t%s\n%s\t%s\n' "${work}/root-one" 1 \
  "${work}/root-two" "$(( blessed_two * 100 ))" > "${XCALYGRAPH_INSTRUCTION_BASELINE}"
if bash "${GUARD}" "${work}/binary" 2> "${work}/stderr"; then
  echo "regression passed the guard" >&2
  exit 1
fi
grep -q 'more instructions than the baseline on root-one' "${work}/stderr"

printf '%s\t%s\n%s\t%s\n' "${work}/root-one" "$(( blessed_one * 100 ))" \
  "${work}/root-two" 1 > "${XCALYGRAPH_INSTRUCTION_BASELINE}"
if bash "${GUARD}" "${work}/binary" 2> "${work}/stderr"; then
  echo "last-root regression passed the guard" >&2
  exit 1
fi
grep -q 'more instructions than the baseline on root-two' "${work}/stderr"

# A vanished root is skipped; a usage error is an error.
printf '%s\t%s\n' "${work}/missing" 1 > "${XCALYGRAPH_INSTRUCTION_BASELINE}"
bash "${GUARD}" "${work}/binary" 2> "${work}/stderr"
grep -q 'missing corpus missing; instructions unchecked' "${work}/stderr"
if bash "${GUARD}" "${work}/absent-binary" 2>/dev/null; then
  echo "missing binary passed" >&2
  exit 1
fi

# Frozen corpus paths resolve to the manifest name, not the repeated snapshot basename `source`.
hono_root="$(awk -F '\t' '$1 == "hono" { print $4; exit }' "${REPO_ROOT}/.cache/edge-baseline/roots.tsv")"
[[ -d "${hono_root}" ]]
printf '%s\t%s\n' "${hono_root}" 1 > "${XCALYGRAPH_INSTRUCTION_BASELINE}"
if bash "${GUARD}" "${work}/binary" 2> "${work}/stderr"; then
  echo "frozen-corpus regression passed" >&2
  exit 1
fi
grep -q 'more instructions than the baseline on hono' "${work}/stderr"

echo "PASS: instruction guard"
