#!/usr/bin/env bash
# One binary against itself reports no difference; a deliberately slowed copy is detected.
# Usage: scripts/tests/test-compare-binaries.sh BINARY
set -euo pipefail
(( $# == 1 )) && [[ -x "$1" ]] || { echo "usage: test-compare-binaries.sh BINARY" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && /bin/pwd -P)"
work="$(mktemp -d "${REPO_ROOT}/.cache/test-compare-binaries.XXXXXXXX")"
trap 'rm -rf -- "${work}"' EXIT
mkdir "${work}/root"
printf 'package a\n\nfunc Config() int { return 1 }\nfunc Other() int { return Config() }\n' > "${work}/root/a.go"
binary="$(cd "$(dirname "$1")" && /bin/pwd -P)/$(basename "$1")"
cat > "${work}/slow" <<WRAPPER
#!/bin/sh
sleep 0.4
exec "${binary}" "\$@"
WRAPPER
chmod +x "${work}/slow"
export COMPARE_LOAD_LIMIT=1000 COMPARE_COOL_DOWN=0 COMPARE_OUTPUT_BASE="${work}/out"
if bash "${REPO_ROOT}/scripts/benchmarks/compare-binaries.sh" --runs 1 "${binary}" /usr/bin/false "${work}/root" >"${work}/failed.out" 2>&1; then
  echo "a failed binary was accepted as a measurement" >&2
  exit 1
fi
cat > "${work}/fail-refresh" <<WRAPPER
#!/bin/sh
if [ "\$1" = index ]; then
  for argument do
    case "\$argument" in
      */copy)
        if [ -e "${work}/copy-indexed" ]; then exit 1; fi
        : > "${work}/copy-indexed"
        ;;
    esac
  done
fi
exec "${binary}" "\$@"
WRAPPER
chmod +x "${work}/fail-refresh"
if bash "${REPO_ROOT}/scripts/benchmarks/compare-binaries.sh" --runs 1 "${binary}" "${work}/fail-refresh" "${work}/root" >"${work}/failed-refresh.out" 2>&1; then
  echo "a failed refresh was accepted as a measurement" >&2
  exit 1
fi
grep -q 'exit 1' "${work}/failed-refresh.out" || { echo "refresh failed before the intended control" >&2; exit 1; }
if find "${work}/out" -type d -name 'root.*' -print -quit | grep -q .; then
  echo "failed comparison left its working copy behind" >&2
  exit 1
fi
mkdir "${work}/empty"
empty="$(bash "${REPO_ROOT}/scripts/benchmarks/compare-binaries.sh" --runs 1 "${binary}" "${binary}" "${work}/empty" 2>&1)" || {
  echo "empty corpus comparison failed: ${empty}" >&2; exit 1;
}
grep -q $'facts\tcomparison\tidentical' <<<"${empty}" || { echo "empty corpus facts differ" >&2; exit 1; }
same="$(bash "${REPO_ROOT}/scripts/benchmarks/compare-binaries.sh" --runs 3 "${binary}" "${binary}" "${work}/root" 2>/dev/null)"
grep -q "facts	comparison	identical" <<<"${same}" || { echo "same binary: facts differ" >&2; exit 1; }
for metric in refresh_body refresh_signature find_common quiet; do
  grep -q "^root	${metric}	median_ms" <<<"${same}" || { echo "missing ${metric}" >&2; exit 1; }
done
# A body edit that shifts lines must reach the index in every run.
awk -F'\t' '$2 == "refresh_body" && $3 == "changed_index_bytes" && $4 > 2000' <<<"${same}" | grep -q . || {
  echo "body edit did not reach the index" >&2; exit 1; }
if awk -F'\t' '$3 == "median_ms" && $7 == "slower" && $5 - $4 > 300' <<<"${same}" | grep -q .; then
  echo "same binary reported as slower" >&2; exit 1
fi
slow="$(bash "${REPO_ROOT}/scripts/benchmarks/compare-binaries.sh" --runs 3 "${binary}" "${work}/slow" "${work}/root" 2>/dev/null)"
for metric in build quiet find_common; do
  awk -F'\t' -v metric="${metric}" '$2 == metric && $3 == "median_ms" && $7 == "slower"' <<<"${slow}" | grep -q . || {
    echo "slowed copy not detected on ${metric}" >&2; exit 1; }
done
noisy="$(COMPARE_LOAD_LIMIT=0 bash "${REPO_ROOT}/scripts/benchmarks/compare-binaries.sh" --runs 1 "${binary}" "${binary}" "${work}/root" 2>/dev/null)"
grep -q "noisy" <<<"${noisy}" || { echo "load above the limit is not marked" >&2; exit 1; }
echo "PASS: compare-binaries detects a slowed copy, equal binaries, noisy rows"
