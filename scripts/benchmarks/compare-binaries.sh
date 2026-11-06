#!/usr/bin/env bash
# Compare two xcalygraph binaries on the same inputs: alternating runs, one index directory per
# binary, one private copy per refresh. Every later performance claim of the plan comes from here.
# Usage: scripts/benchmarks/compare-binaries.sh [--runs N] [--rare WORD] [--common WORD] OLD NEW ROOT...
#   Build, quiet query, rare-word and common-word find, show, one-file refresh after a body edit and
#   after a signature edit: median and p95 of wall time per binary, ratio NEW / OLD, verdict.
#   Also peak RSS, peak footprint, retired instructions (macOS), index size, bytes of index files
#   changed per refresh, binary size, corpus manifest, facts comparison, load before and after.
#   A row measured above load COMPARE_LOAD_LIMIT (default 4) is marked noisy and counts as no
#   evidence; up to COMPARE_COOL_DOWN seconds (default 120) pass first. Verdict `same` means within COMPARE_NOISE (default 0.10) or the spread of the runs.
#   Output: directory compare.* below COMPARE_OUTPUT_BASE (default .cache/plan-baseline) with report.tsv, manifest.tsv and a
#   table on stdout. Sources stay read-only: edits run on a copy and are restored by hash.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
readonly OUTPUT_BASE="${COMPARE_OUTPUT_BASE:-${REPO_ROOT}/.cache/plan-baseline}"
mkdir -p "${OUTPUT_BASE}"
OUTPUT="$(mktemp -d "${OUTPUT_BASE}/compare.XXXXXXXX")"
readonly OUTPUT
python3 -I - "${OUTPUT}" "$@" <<'PYTHON'
import argparse
import filecmp
import hashlib
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

output = sys.argv[1]
parser = argparse.ArgumentParser(prog="compare-binaries.sh")
parser.add_argument("--runs", type=int, default=5)
parser.add_argument("--rare", default="zqxjkvwhisper")
parser.add_argument("--common", default="config")
parser.add_argument("old")
parser.add_argument("new")
parser.add_argument("roots", nargs="+")
args = parser.parse_args(sys.argv[2:])
LOAD_LIMIT = float(os.environ.get("COMPARE_LOAD_LIMIT", "4"))
NOISE = float(os.environ.get("COMPARE_NOISE", "0.10"))
COPY_LIMIT = 20000
binaries = {"old": os.path.abspath(args.old), "new": os.path.abspath(args.new)}
for binary in binaries.values():
    if not os.access(binary, os.X_OK):
        sys.exit(f"compare-binaries.sh: not executable: {binary}")
home = os.path.join(output, "home")
os.makedirs(home)
TIME = ["/usr/bin/time", "-l"] if sys.platform == "darwin" else None
COUNTERS = {"rss": r"(\d+)\s+maximum resident set size", "footprint": r"(\d+)\s+peak memory footprint",
            "instructions": r"(\d+)\s+instructions retired"}

def measured(tag, index, command, root):
    environment = dict(os.environ, HOME=home, XCALYGRAPH_INDEX_DIR=index)
    full = [binaries[tag], *command, "-C", root]
    if TIME:
        full = TIME + full
    start = time.perf_counter()
    done = subprocess.run(full, env=environment, capture_output=True, text=True, errors="replace")
    elapsed = (time.perf_counter() - start) * 1000
    row = {"ms": elapsed, "returncode": done.returncode, "stdout": done.stdout}
    for name, pattern in COUNTERS.items():
        found = re.search(pattern, done.stderr)
        row[name] = float(found.group(1)) if found else None
    return row

def stats(values):
    values = sorted(values)
    return statistics.median(values), values[min(len(values) - 1, int(len(values) * 0.95))]

def verdict(old, new):
    median_old, median_new = statistics.median(old), statistics.median(new)
    if median_old == 0:
        return 1.0, "same"
    ratio = median_new / median_old
    spread = max((max(values) - min(values)) / max(statistics.median(values), 1e-9) for values in (old, new))
    return ratio, "same" if abs(ratio - 1) <= max(NOISE, min(spread, 0.25)) else ("slower" if ratio > 1 else "faster")

report = open(os.path.join(output, "report.tsv"), "w")
report.write("root\tmetric\tstat\told\tnew\tratio\tverdict\tload\n")
def row(root, metric, stat, old, new, ratio, result, load):
    noisy = "noisy" if load > LOAD_LIMIT else f"{load:.1f}"
    line = f"{root}\t{metric}\t{stat}\t{old:.1f}\t{new:.1f}\t{ratio:.3f}\t{result}\t{noisy}"
    report.write(line + "\n")
    print(line)

def cool_down():
    limit = float(os.environ.get("COMPARE_COOL_DOWN", "120"))
    start = time.time()
    while os.getloadavg()[0] > LOAD_LIMIT and time.time() - start < limit:
        time.sleep(5)
    return os.getloadavg()[0]

def paired(root_name, metric, prepare, command, root, indexes, keep=("rss", "footprint", "instructions")):
    load = cool_down()
    runs = {"old": [], "new": []}
    for _ in range(args.runs):
        for tag in ("old", "new"):
            if prepare:
                prepare(tag)
            result = measured(tag, indexes[tag], command, root)
            # An empty answer is an answer; a signal or a panic is not.
            if result["returncode"] < 0 or result["returncode"] == 101:
                sys.exit(f"compare-binaries.sh: {tag} {command} exit {result['returncode']}")
            runs[tag].append(result)
    old_ms, new_ms = [r["ms"] for r in runs["old"]], [r["ms"] for r in runs["new"]]
    ratio, result = verdict(old_ms, new_ms)
    (old_median, old_p95), (new_median, new_p95) = stats(old_ms), stats(new_ms)
    row(root_name, metric, "median_ms", old_median, new_median, ratio, result, load)
    row(root_name, metric, "p95_ms", old_p95, new_p95, new_p95 / max(old_p95, 1e-9), "-", load)
    for name in keep:
        old_values, new_values = [r[name] for r in runs["old"] if r[name]], [r[name] for r in runs["new"] if r[name]]
        if old_values and new_values:
            old_value, new_value = statistics.median(old_values), statistics.median(new_values)
            row(root_name, metric, name, old_value, new_value, new_value / max(old_value, 1e-9), "-", load)
    return runs

def snapshot(index):
    found = {}
    for directory, _, files in os.walk(index):
        for name in files:
            path = os.path.join(directory, name)
            state = os.stat(path)
            found[path] = (state.st_ino, state.st_size, state.st_mtime_ns)
    return found

def size_of(index):
    return sum(entry[1] for entry in snapshot(index).values())

def manifest(root):
    files = total = largest = 0
    for directory, names, entries in os.walk(root):
        names[:] = [name for name in names if name != ".git"]
        for name in entries:
            try:
                size = os.lstat(os.path.join(directory, name)).st_size
            except OSError:
                continue
            files += 1
            total += size
            largest = max(largest, size)
    return files, total, largest

COMMENT = {".py": "# {}\n", ".rb": "# {}\n", ".sh": "# {}\n"}
ADDED = {".go": "\nfunc XcalygraphEdit() {{}}\n", ".py": "\ndef xcalygraph_edit():\n    pass\n",
         ".rs": "\npub fn xcalygraph_edit() {{}}\n", ".ts": "\nexport function xcalygraphEdit() {{}}\n",
         ".js": "\nfunction xcalygraphEdit() {{}}\n", ".php": "\nfunction xcalygraphEdit() {{}}\n",
         ".java": "\n// no top-level function {}\n"}

def edit_target(root):
    best = None
    for directory, names, entries in os.walk(root):
        names[:] = [name for name in names if name not in (".git", "node_modules", "vendor")]
        for name in entries:
            extension = os.path.splitext(name)[1]
            if extension not in ADDED:
                continue
            path = os.path.join(directory, name)
            size = os.lstat(path).st_size
            if size and (best is None or (size, path) > best):
                best = (size, path)
    return best[1] if best else None

manifest_file = open(os.path.join(output, "manifest.tsv"), "w")
manifest_file.write("root\tfiles\tbytes\tlargest\tload_before\tload_after\n")
print("root\tmetric\tstat\told\tnew\tratio\tverdict\tload")
for tag, binary in binaries.items():
    print(f"{tag} binary size {os.path.getsize(binary)} sha256 {hashlib.sha256(open(binary, 'rb').read()).hexdigest()}", file=sys.stderr)
    report.write(f"-\tbinary_{tag}\tbytes\t{os.path.getsize(binary)}\t\t\t\t\n")
for root in args.roots:
    root = os.path.abspath(root)
    name = os.path.basename(root.rstrip("/"))
    load_before = os.getloadavg()[0]
    files, total, largest = manifest(root)
    work = tempfile.mkdtemp(prefix=f"{name}.", dir=output)
    indexes = {tag: os.path.join(work, f"index-{tag}") for tag in binaries}
    def fresh(tag):
        shutil.rmtree(indexes[tag], ignore_errors=True)
    paired(name, "build", fresh, ["index"], root, indexes)
    old_size, new_size = size_of(indexes["old"]), size_of(indexes["new"])
    row(name, "index", "bytes", old_size, new_size, new_size / max(old_size, 1), "-", os.getloadavg()[0])
    facts = "unknown"
    def locate(index, wanted):
        for directory, names, entries in os.walk(index):
            if wanted in names or wanted in entries:
                return os.path.join(directory, wanted)
    old_derived, new_derived = locate(indexes["old"], "derived"), locate(indexes["new"], "derived")
    if old_derived and new_derived:
        old_facts, new_facts = locate(indexes["old"], "facts"), locate(indexes["new"], "facts")
        same_derived = filecmp.cmp(old_derived, new_derived, shallow=False)
        same_facts = old_facts and new_facts and subprocess.run(["diff", "-rq", old_facts, new_facts], capture_output=True).returncode == 0
        facts = "identical" if same_derived and same_facts else "different"
    report.write(f"{name}\tfacts\tcomparison\t{facts}\t\t\t\t\n")
    print(f"{name}\tfacts\tcomparison\t{facts}")
    paired(name, "quiet", None, ["index"], root, indexes, keep=("rss", "instructions"))
    paired(name, "find_rare", None, ["find", args.rare], root, indexes, keep=("rss", "instructions"))
    paired(name, "find_common", None, ["find", args.common], root, indexes, keep=("rss", "instructions"))
    first = measured("old", indexes["old"], ["find", args.common, "--json"], root)["stdout"]
    symbol = re.search(r'"t":"sym".*?"name":"([^"]+)"', first)
    if symbol:
        paired(name, "show", None, ["show", symbol.group(1)], root, indexes, keep=("rss", "instructions"))
    if files > COPY_LIMIT:
        report.write(f"{name}\trefresh\tskipped\tabove {COPY_LIMIT} files\t\t\t\t\n")
    else:
        copy = os.path.join(work, "copy")
        shutil.copytree(root, copy, symlinks=True, ignore=shutil.ignore_patterns(".git"))
        target = edit_target(copy)
        if target is None:
            report.write(f"{name}\trefresh\tskipped\tno editable source\t\t\t\t\n")
        else:
            original = open(target, "rb").read()
            digest = hashlib.sha256(original).hexdigest()
            extension = os.path.splitext(target)[1]
            comment = COMMENT.get(extension, "// {}\n")
            written = {}
            copy_indexes = {tag: os.path.join(work, f"copy-{tag}") for tag in binaries}
            for tag in binaries:
                measured(tag, copy_indexes[tag], ["index"], copy)
            for kind, text in (("body", comment.format("xcalygraph edit")), ("signature", ADDED[extension].format("xcalygraph edit"))):
                counter = [0]
                sizes = {"old": [], "new": []}
                def apply(tag):
                    counter[0] += 1
                    with open(target, "wb") as handle:
                        handle.write(original + text.encode() + f"{comment.format(counter[0])}".encode())
                load = cool_down()
                runs = {"old": [], "new": []}
                for _ in range(args.runs):
                    for tag in ("old", "new"):
                        apply(tag)
                        before = snapshot(copy_indexes[tag])
                        result = measured(tag, copy_indexes[tag], ["index"], copy)
                        after = snapshot(copy_indexes[tag])
                        sizes[tag].append(sum(state[1] for path, state in after.items() if before.get(path) != state))
                        runs[tag].append(result["ms"])
                ratio, result = verdict(runs["old"], runs["new"])
                row(name, f"refresh_{kind}", "median_ms", statistics.median(runs["old"]), statistics.median(runs["new"]), ratio, result, load)
                row(name, f"refresh_{kind}", "changed_index_bytes", statistics.median(sizes["old"]), statistics.median(sizes["new"]),
                    statistics.median(sizes["new"]) / max(statistics.median(sizes["old"]), 1), "-", load)
            with open(target, "wb") as handle:
                handle.write(original)
            if hashlib.sha256(open(target, "rb").read()).hexdigest() != digest:
                sys.exit("compare-binaries.sh: edited copy not restored")
        shutil.rmtree(copy, ignore_errors=True)
    manifest_file.write(f"{root}\t{files}\t{total}\t{largest}\t{load_before:.1f}\t{os.getloadavg()[0]:.1f}\n")
    manifest_file.flush()
    report.flush()
    shutil.rmtree(work, ignore_errors=True)
report.close()
print(f"evidence: {output}", file=sys.stderr)
PYTHON
