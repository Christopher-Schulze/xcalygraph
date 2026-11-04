#!/usr/bin/env bash
# Exercise frozen-cohort review and source verification without indexing or changing evidence.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import collections
import csv
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
runner = repo / "scripts/benchmarks/sample-precision.sh"
cache = repo / ".cache"
assert not cache.is_symlink()
cache.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix="precision-review-test.", dir=cache) as temporary:
    work = Path(temporary)
    source = work / "source"
    source.mkdir()
    content = "".join(f"target(); // reference {line}\n" for line in range(1, 81)).encode()
    (source / "main.rs").write_bytes(content)
    (source / "target.rs").write_text("fn target() {}\n")
    (source / "asset.bin").write_bytes(b"\x00\xffbinary data")
    prepared = subprocess.run(["bash", str(repo / "scripts/utils/prepare-corpus.sh"),
                               str(source), str(work / "snapshots")], capture_output=True,
                              text=True, check=True)
    snapshot = Path(prepared.stdout.strip())
    digest = json.loads((snapshot.parent / "snapshot.json").read_text())["source_sha256"]
    run = work / "run"
    run.mkdir()
    roots = [[name, "fixture", digest, str(snapshot)] for name in ("alpha", "beta")]
    with (run / "roots.tsv").open("x", newline="") as stream:
        csv.writer(stream, delimiter="\t", lineterminator="\n").writerows(roots)
    samples = []
    for name in ("alpha", "beta"):
        for line, raw in enumerate(content.split(b"\n")[:-1], 1):
            samples.append({"record_type": "sample", "sample_id": hashlib.sha256(f"{name}:{line}".encode()).hexdigest(),
                            "corpus": name, "corpus_revision": "fixture", "corpus_snapshot_sha256": digest,
                            "source_language": "rust", "edge_kind": "references", "confidence": "exact",
                            "source_path": "main.rs", "source_line": line, "source_name": "main.rs",
                            "source_text_sha256": hashlib.sha256(raw).hexdigest(),
                            "target_path": "target.rs", "target_line": 1, "target_name": "target"})
    sparse = dict(samples[0], sample_id="a" * 64, source_language="bash", edge_kind="runs", confidence="inferred")
    samples.append(sparse)
    samples.append(dict(samples[0], sample_id="b" * 64, source_language="text", target_path="asset.bin"))
    (run / "manifest.jsonl").write_text("".join(json.dumps(row) + "\n" for row in samples))
    fields = ("sample_id", "corpus", "corpus_revision", "corpus_snapshot_sha256", "source_language",
              "edge_kind", "confidence", "source_path", "source_line", "target_path", "target_line", "verdict")
    with (run / "adjudication.tsv").open("x", newline="") as stream:
        writer = csv.DictWriter(stream, fields, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows({key: "pending" if key == "verdict" else row[key] for key in fields} for row in samples)
    original = {name: (run / name).read_bytes() for name in ("manifest.jsonl", "adjudication.tsv", "roots.tsv")}
    decisions = work / "decisions.jsonl"
    decisions.write_text("")

    def invoke(destination):
        return subprocess.run(["bash", str(runner), "--review", str(run), str(decisions), str(destination)],
                              capture_output=True, text=True, env=dict(os.environ, HOME=str(work / "home")))

    selected = work / "selected"
    result = invoke(selected)
    assert result.returncode == 0, result.stderr
    cohort = [json.loads(line) for line in (selected / "selection.jsonl").read_text().splitlines()]
    assert len(cohort) == 102
    main = [row for row in cohort if row["source_language"] == "rust"]
    assert collections.Counter(row["corpus"] for row in main) == {"alpha": 50, "beta": 50}
    for name in ("alpha", "beta"):
        assert {row["sample_id"] for row in main if row["corpus"] == name} == set(sorted(
            row["sample_id"] for row in samples if row["source_language"] == "rust" and row["corpus"] == name)[:50])
    decisions.write_text("".join(json.dumps({"sample_id": row["sample_id"], "verdict": verdict,
                                             "reason": "Source and target reviewed in the fixture."}) + "\n"
                               for row, verdict in zip(cohort[:2], ("correct", "incorrect"))))
    reviewed = work / "reviewed"
    assert invoke(reviewed).returncode == 0
    ledger = list(csv.DictReader((reviewed / "adjudication.tsv").open(), delimiter="\t"))
    assert collections.Counter(row["verdict"] for row in ledger) == {"correct": 1, "incorrect": 1, "pending": 100}
    assert all(row["source_file_sha256"] and row["target_file_sha256"] for row in ledger)
    contexts = [json.loads(line) for line in (reviewed / "context.jsonl").read_text().splitlines()]
    assert "target();" in contexts[0]["source_context"] and "fn target()" in contexts[0]["target_context"]
    binary = next(row for row in contexts if row["target_path"] == "asset.bin")
    assert binary["target_context"] == "binary data: 13 bytes", binary["target_context"]
    retained = {path.name: path.read_bytes() for path in reviewed.iterdir()}
    assert invoke(reviewed).returncode != 0
    assert retained == {path.name: path.read_bytes() for path in reviewed.iterdir()}
    valid = json.loads(decisions.read_text().splitlines()[0])
    for label, rows in [("duplicate", [valid, valid]),
                        ("foreign", [dict(valid, sample_id="f" * 64)]),
                        ("invalid-verdict", [dict(valid, verdict="pass")]),
                        ("missing-reason", [dict(valid, reason="")])]:
        decisions.write_text("".join(json.dumps(row) + "\n" for row in rows))
        output = work / label
        assert invoke(output).returncode != 0 and not output.exists(), label
    decisions.write_text("")
    original_source = (snapshot / "main.rs").read_bytes()
    (snapshot / "main.rs").write_bytes(b"changed source\n")
    assert invoke(work / "stale").returncode != 0 and not (work / "stale").exists()
    (snapshot / "main.rs").write_bytes(original_source)
    forged = [dict(row) for row in samples]
    for row in forged:
        row["source_text_sha256"] = "0" * 64
    (run / "manifest.jsonl").write_text("".join(json.dumps(row) + "\n" for row in forged))
    assert invoke(work / "forged").returncode != 0 and not (work / "forged").exists()
    (run / "manifest.jsonl").write_bytes(original["manifest.jsonl"])
    assert original == {name: (run / name).read_bytes() for name in original}
print("PASS: frozen pooled ranks, explicit verdicts, source binding, sparse counts and original/output preservation")
PYTHON
