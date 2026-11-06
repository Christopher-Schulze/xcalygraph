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
same="$(bash "${REPO_ROOT}/scripts/benchmarks/compare-binaries.sh" --runs 3 "${binary}" "${binary}" "${work}/root" 2>/dev/null)"
grep -q "facts	comparison	identical" <<<"${same}" || { echo "same binary: facts differ" >&2; exit 1; }
for metric in refresh_body refresh_signature find_common quiet; do
  grep -q "^root	${metric}	median_ms" <<<"${same}" || { echo "missing ${metric}" >&2; exit 1; }
done
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
