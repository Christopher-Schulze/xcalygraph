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


def evidence(answer, name, relation=None):
    aliases = {'doc': 'document', 't': 'test', 't2': 'test', 't3': 'test', 'write': 'writer'}

    def bundle(present, missing):
        roles = {}
        for role, site in [*present, *((role, 'absent') for role in missing)]:
            role = aliases.get(role, role)
            roles.setdefault(role, set()).add(site)
        return roles

    if mode == 'json':
        records = [json.loads(line) for line in answer.splitlines()]
        bundles = []
        for record in records:
            if record.get('t') == 'evidence' and record['name'] == name:
                bundles.append(bundle(
                    [(r['role'], f"{r['path']}:{r['line']}") for r in record['roles']],
                    record.get('unavailable', []),
                ))
        chains, origin = [], None
        for record in records:
            kind = record.get('t')
            if kind == 'chain':
                chains.append([hop.split(maxsplit=1)[0] for hop in record['hops']])
            elif kind == 'sym':
                origin = f"{record['path']}:{record['line']}"
            elif kind == 'file':
                origin = None
            elif (kind == 'edge' and origin and relation
                  and record.get('rel') == 'out' and record.get('kind') == relation):
                chains.append([origin, f"{record['path']}:{record['line']}"])
        return bundles, chains
    bundles, chains = [], []
    path, origin = None, None
    for line in answer.splitlines():
        if relation:
            if not line.startswith((' ', '~', '+')):
                path, origin = line, None
            elif path and (symbol := re.match(r'^  (\d+) ', line)):
                origin = f'{path}:{symbol[1]}'
            elif origin and (edge := re.search(
                rf'^ +> (?:.* )?(\S+:\d+) {re.escape(relation)}\??$', line
            )):
                chains.append([origin, edge[1]])
        if line.startswith(f'~ {name} '):
            roles = bundle(
                re.findall(r' <(\w+)\?? (\S+:\d+)(?= |$)', line),
                re.findall(r' -(definition|read|write|doc|t)(?= |$)', line),
            )
            if roles:
                bundles.append(roles)
                continue
        if line.startswith('~ '):
            hops, last_path = [], ''
            for label in re.split(r' >\d+\?? ', line[2:]):
                location = re.search(r'(?:^| )(\S*):(\d+)$', label)
                if location:
                    path, number = location.groups()
                    last_path = path or last_path
                    hops.append(f'{last_path}:{number}')
            if len(hops) >= 2:
                chains.append(hops)
    return bundles, chains


def score(case, answer):
    relation = 'runs' if case['kind'] == 'runs' else None
    bundles, chains = evidence(answer, case['name'], relation)
    alternatives = [[term.split('=', 1) for term in group.split(';')]
                    for group in case['required'].split('||')]
    forbidden = case['forbidden'].split(';') if case['forbidden'] else []
    bad = sum(any(sites - {'absent'} for role, sites in bundle.items() if role in forbidden)
              or any('absent' in sites and len(sites) > 1 for sites in bundle.values())
              for bundle in bundles)
    if case['kind'] in ('chain', 'runs'):
        met = any(all(locator in chain for _, locator in required)
                  for chain in chains for required in alternatives)
    else:
        met = any(all(len(bundle.get(role, ())) == 1
                      and bundle[role] <= set(locators.split('|'))
                      for role, locators in required)
                  for bundle in bundles for required in alternatives)
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
