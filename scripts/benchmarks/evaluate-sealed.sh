#!/usr/bin/env bash
# Internal JSONL evaluation runner used by run-evaluation.sh --set sealed.
# Usage: evaluate-sealed.sh MANIFEST final|development|validate BINARY [--budget N] [--reuse-graphify RUN] [CORPUS...]
# All tools run on verified frozen sources with isolated HOME and Git environments and explicit index paths.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
python3 -I - "$REPO_ROOT" "$@" <<'PYTHON'
import collections
import hashlib
import io
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import time


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write_new(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as stream:
        stream.write(data)


def json_bytes(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def restore_source(repo, work, corpus, environment):
    source = repo / corpus["snapshot"]
    if source.exists():
        return source
    name = corpus["name"]
    origin = repo if name == "xcalygraph" else repo / "research/corpora" / name
    if name == "graphify":
        origin = repo / "research/upstream/graphify"
    # A nested corpus archives its subtree; never checkout or modify the live tree.
    prefix = subprocess.run(["git", "-C", str(origin), "rev-parse", "--show-prefix"],
                            env=environment, capture_output=True, text=True, check=True).stdout.strip()
    archive = subprocess.run(["git", "-C", str(origin), "archive", "--format=tar",
                              corpus["commit"] + (":" + prefix.rstrip("/") if prefix else "")],
                             env=environment, capture_output=True, check=True).stdout
    target = work / ("restore-" + name.replace("/", "-"))
    target.mkdir()
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as bundle:
        for member in bundle.getmembers():
            relative = Path(member.name)
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("unsafe archive path")
            if not member.isfile():
                continue  # Match the snapshot's regular-file contract, never follow symlinks.
            reader = bundle.extractfile(member)
            if reader is None:
                raise ValueError("archive member has no content")
            path = target / relative
            write_new(path, reader.read())
            path.chmod(0o755 if member.mode & 0o111 else 0o644)
    # Archive members were tracked even when their names match ignore rules.
    subprocess.run(["git", "init", "-q", "-b", "main", str(target)], env=environment, check=True)
    subprocess.run(["git", "-C", str(target), "add", "-f", "."], env=environment, check=True)
    result = subprocess.run(["bash", str(repo / "scripts/utils/prepare-corpus.sh"), str(target),
                             str(repo / ".cache/corpus-snapshots"), "--graphify"],
                            env=environment, capture_output=True, text=True, check=True)
    if not Path(result.stdout.strip()).samefile(source):
        raise ValueError("pinned sources do not reproduce the frozen identity: " + name)
    return source


def validate_corpus(repo, work, directory, corpus, environment):
    data = (directory / corpus["cases"]).read_bytes()
    if sha(data) != corpus["cases_sha256"]:
        raise ValueError("question checksum mismatch: " + corpus["name"])
    cases = [json.loads(line) for line in data.splitlines()]
    if len(cases) != corpus["count"] or len({c["id"] for c in cases}) != len(cases):
        raise ValueError("invalid case count or duplicate id")
    source = restore_source(repo, work, corpus, environment)
    subprocess.run(["bash", str(repo / "scripts/utils/prepare-corpus.sh"), "--verify", str(source)],
                   env=environment, capture_output=True, check=True)
    metadata = json.loads((source.parent / "snapshot.json").read_text())
    if (metadata["source_sha256"], metadata["upstream"]) != (corpus["source_sha256"], corpus["upstream"]):
        raise ValueError("frozen source/tool identity mismatch")
    source_paths = {f["path"] for f in metadata["source"]["files"]}
    for case in cases:
        if case["family"] not in ("symbol", "behavior", "relationship", "multi", "negative"):
            raise ValueError("unknown question family")
        if case["family"] == "negative":
            needle = case["absence"]["identifier"].encode()
            if case["absence"]["source_sha256"] != corpus["source_sha256"] or case["requires"] or any(needle in (source / f["path"]).read_bytes()
                                       for f in metadata["source"]["files"]):
                raise ValueError("negative identifier is present")
        elif not case["requires"] or case["family"] == "multi" and len(case["requires"]) < 2:
            raise ValueError("missing positive requirements")
        for group in case["requires"]:
            if not group["alternatives"]:
                raise ValueError("empty alternative group")
            for locator in group["alternatives"]:
                if locator["path"] not in source_paths:
                    raise ValueError("locator is not part of the frozen source")
                content = (source / locator["path"]).read_bytes()
                lines = content.decode().splitlines(keepends=True)
                start, end = locator["line"] - 1, locator["evidence_end"]
                if not 0 <= start < end <= len(lines) or locator["name"] not in lines[start]:
                    raise ValueError("invalid source declaration")
                if (sha(content), sha("".join(lines[start:end]).encode())) != (locator["file_sha256"], locator["evidence_sha256"]):
                    raise ValueError("source evidence mismatch")
    return source, cases


def records(text, form):
    result = []
    if form == "json":
        for line in text.splitlines():
            record = json.loads(line)
            if record.get("t") in ("sym", "file"):
                result.append((record["path"], record.get("line", 0), record.get("name", "")))
    elif form == "graphify":
        for line in text.splitlines():
            match = re.match(r"NODE (.*?) \[src=(.*?) loc=L(\d+)(?:-L?\d+)?(?: |\])", line)
            if match:
                result.append((match[2], int(match[3]), match[1]))
    else:
        path, has_symbol = None, False
        for line in text.splitlines() + [""]:
            if line.startswith("  "):
                match = re.match(r"  (\d+) (.*)", line)
                if match and path:
                    result.append((path, int(match[1]), match[2]))
                    has_symbol = True
            elif not line.startswith(("+", "~ ")):
                if path and not has_symbol:
                    result.append((path, 0, ""))
                path, has_symbol = line or None, False
    return result


def matches(record, locator):
    path, line, name = record
    return (path == locator["path"] and line == locator["line"]
            and re.search(r"(?<![\w])" + re.escape(locator["name"]) + r"(?![\w])", name) is not None)


def score(case, found, output, budget):
    groups = case["requires"]
    covered = [any(matches(record, loc) for record in found for loc in group["alternatives"])
               for group in groups]
    top = [any(matches(record, loc) for record in found[:5] for loc in group["alternatives"])
           for group in groups]
    primary = {loc["path"] for loc in groups[0]["alternatives"]} if groups else set()
    return {"id": case["id"], "family": case["family"], "covered": sum(covered),
            "required": len(groups), "complete": bool(groups) and all(covered),
            "complete5": bool(groups) and all(top),
            "file5": any(record[0] in primary for record in found[:5]),
            "file_any": any(record[0] in primary for record in found),
            "false_positive": not groups and bool(found), "bytes": len(output),
            "tokens": (len(output) + 2) // 3, "over_budget": len(output) > budget * 3}


def wilson(hits, count):
    if not count:
        return None
    z = 1.959963984540054
    proportion = hits / count
    denominator = 1 + z * z / count
    center = (proportion + z * z / (2 * count)) / denominator
    width = z * math.sqrt(proportion * (1 - proportion) / count + z * z / (4 * count * count)) / denominator
    return [round(max(0, center - width), 4), round(min(1, center + width), 4)]


def aggregate(rows):
    positive = [row for row in rows if row["required"]]
    negative = [row for row in rows if not row["required"]]
    hits = sum(row["complete"] for row in positive)
    return {"positive": len(positive), "complete": hits, "complete95": wilson(hits, len(positive)),
            "complete5": sum(row["complete5"] for row in positive),
            "file5": sum(row["file5"] for row in positive), "file_any": sum(row["file_any"] for row in positive),
            "covered": sum(row["covered"] for row in positive), "required": sum(row["required"] for row in positive),
            "multi_complete": sum(row["complete"] for row in positive if row["family"] == "multi"),
            "multi_total": sum(row["family"] == "multi" for row in positive),
            "multi_location_complete": sum(row["complete"] for row in positive if row["required"] > 1),
            "multi_location_total": sum(row["required"] > 1 for row in positive),
            "negative_false_positive": sum(row["false_positive"] for row in negative), "negative_total": len(negative),
            "mean_positive_tokens": round(sum(row["tokens"] for row in positive) / max(1, len(positive)), 1),
            "over_budget": sum(row["over_budget"] for row in rows)}


def observe(command, root, environment, destination, accepted):
    started = time.monotonic()
    result = subprocess.run(command, cwd=root, env=environment, capture_output=True)
    write_new(destination.with_suffix(".stdout"), result.stdout)
    write_new(destination.with_suffix(".stderr"), result.stderr)
    write_new(destination.with_suffix(".json"), json_bytes({"command": command, "exit": result.returncode,
                                                           "wall_ms": (time.monotonic() - started) * 1000}))
    if result.returncode not in accepted:
        raise ValueError("tool failed; retained evidence: " + str(destination))
    return result.stdout, result.returncode


def observe_graphify(command, root, environment, destination, baseline, case, budget, score_file):
    if baseline is None:
        return observe(command, root, environment, destination, (0, 3, 4))
    previous = baseline / destination.relative_to(destination.parents[2])
    if json.loads((previous.parent / "case.json").read_text()) != case:
        raise ValueError("baseline case mismatch: " + case["id"])
    metadata = json.loads(previous.with_suffix(".json").read_text())
    output = previous.with_suffix(".stdout").read_bytes()
    stderr = previous.with_suffix(".stderr").read_bytes()
    status = metadata["exit"]
    if metadata["command"] != command or status not in (0, 3, 4):
        raise ValueError("baseline command or exit mismatch: " + case["id"])
    if "stdout_sha256" in metadata and metadata["stdout_sha256"] != sha(output):
        raise ValueError("baseline output checksum mismatch: " + case["id"])
    found = records(output.decode(), "graphify") if status == 0 else []
    stored = [json.loads(line) for line in score_file.read_text().splitlines()]
    matching = [row for row in stored if row["id"] == case["id"]]
    if len(matching) != 1 or matching[0] != score(case, found, output, budget):
        raise ValueError("baseline score mismatch: " + case["id"])
    metadata["reused_from"] = str(previous)
    metadata["stdout_sha256"] = sha(output)
    write_new(destination.with_suffix(".stdout"), output)
    write_new(destination.with_suffix(".stderr"), stderr)
    write_new(destination.with_suffix(".json"), json_bytes(metadata))
    return output, status


def evaluate(repo, work, corpus, cases, source, binary, environment, budgets, baseline):
    slug = corpus["name"].replace("/", "-")
    index = work / ("index-" + slug)
    common = ["-C", str(source), "--index-dir", str(index)]
    observe([str(binary), "index"] + common, source, environment, work / slug / "index", (0,))
    outcomes = collections.defaultdict(list)
    for number, case in enumerate(cases, 1):
        directory = work / slug / case["id"]
        write_new(directory / "case.json", json_bytes(case))
        for budget in budgets:
            for verb in ("find",):
                for form in ("text", "json"):
                    arm = verb + "-" + form
                    command = [str(binary), verb, case["question"], "-b", str(budget)] + common
                    if form == "json":
                        command.append("--json")
                    output, status = observe(command, source, environment, directory / (arm + "-" + str(budget)), (0, 4))
                    found = records(output.decode(), form) if status == 0 else []
                    outcomes[(arm, budget)].append(score(case, found, output, budget))
            command = ["graphify", "query", case["question"], "--budget", str(budget),
                       "--graph", str(source / "graphify-out/graph.json")]
            score_file = baseline / slug / ("graphify-" + str(budget) + ".scores.jsonl") if baseline else None
            output, status = observe_graphify(command, source, environment, directory / ("graphify-" + str(budget)),
                                               baseline, case, budget, score_file)
            found = records(output.decode(), "graphify") if status == 0 else []
            outcomes[("graphify", budget)].append(score(case, found, output, budget))
        # Keep TASK 025's asymmetric legacy comparison visible, separate from equal-budget curves.
        command = ["graphify", "query", case["question"], "--budget", "2000",
                   "--graph", str(source / "graphify-out/graph.json")]
        score_file = baseline / slug / "graphify-legacy-2000.scores.jsonl" if baseline else None
        output, status = observe_graphify(command, source, environment, directory / "graphify-legacy",
                                           baseline, case, 2000, score_file)
        found = records(output.decode(), "graphify") if status == 0 else []
        outcomes[("graphify-legacy", 2000)].append(score(case, found, output, 2000))
        if number % 5 == 0:
            print(corpus["name"], str(number) + "/" + str(len(cases)), "observations retained", flush=True)
    for (arm, budget), rows in outcomes.items():
        write_new(work / slug / (arm + "-" + str(budget) + ".scores.jsonl"), b"".join(json_bytes(row) for row in rows))
    return outcomes


def main():
    if len(sys.argv) < 5:
        raise ValueError("usage: evaluate-sealed.sh MANIFEST final|development|validate BINARY [--budget N] [--reuse-graphify RUN] [CORPUS...]")
    repo, manifest_path, phase, binary = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], Path(sys.argv[4]).resolve()
    arguments = sys.argv[5:]
    budget = None
    baseline = None
    while arguments and arguments[0].startswith("--"):
        if len(arguments) < 2:
            raise ValueError(arguments[0] + " needs a value")
        if arguments[0] == "--budget" and budget is None:
            budget = int(arguments[1])
        elif arguments[0] == "--reuse-graphify" and baseline is None:
            baseline = Path(arguments[1]).resolve()
        else:
            raise ValueError("unknown or repeated option " + arguments[0])
        arguments = arguments[2:]
    selected = set(arguments)
    if phase not in ("final", "development", "validate"):
        raise ValueError("phase must be final, development or validate")
    if (budget is not None or baseline is not None) and phase != "development":
        raise ValueError("--budget and --reuse-graphify are restricted to development")
    manifest_data = manifest_path.read_bytes()
    manifest = json.loads(manifest_data)
    if manifest["schema"] != 1:
        raise ValueError("unsupported manifest schema")
    if len({c["name"] for c in manifest["corpora"]}) != len(manifest["corpora"]):
        raise ValueError("duplicate corpus names")
    if (not manifest["budgets"] or any(type(b) is not int or b < 1 for b in manifest["budgets"])
            or len(set(manifest["budgets"])) != len(manifest["budgets"])):
        raise ValueError("budgets must be distinct positive integers")
    if budget is not None and budget not in manifest["budgets"]:
        raise ValueError("--budget must be one of the frozen manifest budgets")
    budgets = [budget] if budget is not None else manifest["budgets"]
    corpora = [c for c in manifest["corpora"] if not selected or c["name"] in selected]
    if not corpora or selected - {c["name"] for c in corpora}:
        raise ValueError("unknown or empty corpus selection")
    if baseline is not None:
        identity = json.loads((baseline / "identity.json").read_text())
        if (identity["manifest_sha256"] != sha(manifest_data)
                or sha((baseline / "manifest.json").read_bytes()) != sha(manifest_data)):
            raise ValueError("baseline manifest mismatch")
        report = json.loads((baseline / "report.json").read_text())
        if any(c["name"] not in report["corpora"] for c in corpora):
            raise ValueError("baseline lacks a completed selected corpus")
    base = repo / ".cache/evaluation"
    if base.parent.is_symlink() or base.is_symlink():
        raise ValueError("evaluation cache must not be a symlink")
    base.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="sealed.", dir=base))
    home = work / "home"
    home.mkdir()
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update(HOME=str(home), PYTHONHASHSEED="0", PYTHONSAFEPATH="1", PYTHONDONTWRITEBYTECODE="1",
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    write_new(work / "manifest.json", manifest_data)
    binary_sha256 = sha(binary.read_bytes()) if phase != "validate" else None
    write_new(work / "identity.json", json_bytes({"manifest_sha256": sha(manifest_data), "binary_sha256": binary_sha256,
                                                  "phase": phase, "budgets": budgets, "corpora": [c["name"] for c in corpora],
                                                  "graphify_baseline": str(baseline) if baseline else None}))
    validated = [(c, *validate_corpus(repo, work, manifest_path.parent, c, environment)) for c in corpora]
    print("validated:", len(corpora), "corpora; evidence:", work, flush=True)
    if phase == "validate":
        write_new(work / ".xcalygraph-disposable", b"xcalygraph-cache-v1\n")
        return
    receipts = repo / ".cache/evaluation/exposure" / sha(manifest_data)
    exposure = {}
    for corpus, _, _ in validated:
        previous = receipts / (corpus["name"].replace("/", "-") + ".json")
        exposure[corpus["name"]] = {"first_evaluation": not previous.exists(),
                                    "previous": json.loads(previous.read_text()) if previous.exists() else None}
        if previous.exists() and phase == "final":
            raise ValueError("sealed corpus already evaluated; use --phase development and report prior exposure")
    write_new(work / "exposure.json", json_bytes(exposure))
    # Any observation consumes final eligibility, including a development or interrupted run.
    for corpus, _, _ in validated:
        previous = receipts / (corpus["name"].replace("/", "-") + ".json")
        if not previous.exists():
            write_new(previous, json_bytes({"run": str(work), "phase": phase}))
    report = {"phase": phase, "limitations": manifest.get("exposure"), "exposure": exposure, "corpora": {}}
    for corpus, source, cases in validated:
        outcomes = evaluate(repo, work, corpus, cases, source, binary, environment, budgets, baseline)
        aggregates = {arm + "-" + str(budget): aggregate(rows) for (arm, budget), rows in outcomes.items()}
        paired = {}
        for budget in budgets:
            for arm in ("find-text", "find-json"):
                counts = collections.Counter((a["complete"], b["complete"])
                                             for a, b in zip(outcomes[(arm, budget)], outcomes[("graphify", budget)])
                                             if a["required"])
                paired[arm + "-" + str(budget)] = {str(a) + "/" + str(b): count for (a, b), count in sorted(counts.items())}
        report["corpora"][corpus["name"]] = {"metrics": aggregates, "paired_complete": paired}
    write_new(work / "report.json", json_bytes(report))
    public = {"schema": 1, "phase": phase, "manifest_sha256": sha(manifest_data),
              "binary_sha256": binary_sha256,
              "evaluator_sha256": sha((repo / "scripts/benchmarks/evaluate-sealed.sh").read_bytes()),
              "budgets": budgets, "prior_exposure": {name: not item["first_evaluation"] for name, item in exposure.items()},
              "graphify_baseline_reused": baseline is not None, "limitations": manifest.get("exposure"),
              "corpora": report["corpora"]}
    write_new(work / "public-report.json", json_bytes(public))
    print(json.dumps(report, sort_keys=True), flush=True)
    print("evidence:", work, flush=True)
    write_new(work / ".xcalygraph-disposable", b"xcalygraph-cache-v1\n")


try:
    main()
except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
    print("evaluate-sealed.sh: " + str(error), file=sys.stderr)
    sys.exit(1)
PYTHON
