#!/usr/bin/env bash
# Score a complete bundle or chain, not independent file hits.
# Usage: scripts/benchmarks/measure-cross-artifact.sh BINARY [BUDGET] [text|json]
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
(( $# >= 1 && $# <= 3 )) && [[ -x "$1" ]] || { echo "usage: measure-cross-artifact.sh BINARY [BUDGET] [text|json]" >&2; exit 2; }
python3 - "${REPO_ROOT}" "$1" "${2:-600}" "${3:-text}" <<'PY'
import csv
import json
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

repo, binary, budget, mode = sys.argv[1:]
repo, binary, budget = Path(repo), Path(binary).resolve(), int(budget)
if budget < 1 or mode not in ('text', 'json'):
    sys.exit('expected a positive budget and text or json')


def evidence(answer, name):
    if mode == 'json':
        records = [json.loads(line) for line in answer.splitlines()]
        bundles = []
        for record in records:
            if record.get('t') == 'evidence' and record['name'] == name:
                roles = {r['role']: f"{r['path']}:{r['line']}" for r in record['roles']}
                roles.update((r, 'absent') for r in record.get('unavailable', []))
                bundles.append(roles)
        return bundles, [' '.join(r['hops']) for r in records if r.get('t') == 'chain']
    aliases = {'doc': 'document', 't': 'test', 't2': 'test'}
    bundles, chains = [], []
    for line in answer.splitlines():
        if line.startswith(f'~ {name} '):
            roles = {aliases.get(role, role): site for role, site in re.findall(
                r' <(\w+)\?? (\S+:\d+)(?= |$)', line)}
            roles.update((aliases.get(role, role), 'absent') for role in re.findall(
                r' -(definition|read|doc|t)(?= |$)', line))
            if roles:
                bundles.append(roles)
            else:
                chains.append(line)
        elif line.startswith('~ '):
            chains.append(line)
    return bundles, chains


def score(case, answer):
    bundles, chains = evidence(answer, case['name'])
    required = [term.split('=', 1) for term in case['required'].split(';') if term]
    forbidden = case['forbidden'].split(';') if case['forbidden'] else []
    bad = sum(any(bundle.get(role) not in (None, 'absent') for role in forbidden) for bundle in bundles)
    if case['kind'] == 'chain':
        met = any(all(locator in chain for _, locator in required) for chain in chains)
    else:
        met = any(all(bundle.get(role) in locators.split('|') for role, locators in required) for bundle in bundles)
    return met and not bad, bad


with (repo / 'scripts/benchmarks/questions/cross-artifact.tsv').open() as manifest:
    cases = list(csv.DictReader(manifest, delimiter='\t'))
complete = false_joins = 0
with tempfile.TemporaryDirectory(prefix='xcalygraph-cross-') as index:
    roots = {case['corpus']: repo / 'research/corpora' / case['corpus'] for case in cases}
    for root in roots.values():
        subprocess.run([binary, 'index', '-C', root, '--index-dir', index], check=True, stdout=subprocess.DEVNULL)
    for case in cases:
        flags = ['--json'] if mode == 'json' else []
        start = time.perf_counter_ns()
        result = subprocess.run([binary, 'find', case['question'], '-C', roots[case['corpus']],
                                 '--index-dir', index, '--read-only', '-b', str(budget), *flags],
                                capture_output=True, check=False)
        elapsed = (time.perf_counter_ns() - start) / 1e6
        if result.returncode not in (0, 4):
            sys.exit(result.stderr.decode())
        if len(result.stdout) > 3 * budget:
            sys.exit(f"{case['id']}: byte budget exceeded")
        met, bad = score(case, result.stdout.decode())
        complete += met
        false_joins += bad
        print(f"{case['id']}\t{'met' if met and not bad else 'miss'}\t{bad}\t{len(result.stdout)}\t{elapsed:.2f}ms")
print(f'complete {complete}/{len(cases)}; false joins {false_joins}; {mode}; budget {budget}')
PY
