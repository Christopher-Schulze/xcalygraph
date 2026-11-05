#!/usr/bin/env bash
# Exercise verification and maintenance tools from checkout roots ending in newlines.
# Usage: scripts/tests/test-checkout-root.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-checkout-root.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="checkout-root.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    home, target, tmp, decoy = (work / name for name in ("home", "target", "tmp", "project"))
    for directory in (home, target, tmp, decoy):
        directory.mkdir()
    sentinel = decoy / "sentinel"
    sentinel.write_text("preserve\n")
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "GITHUB_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_", "RUSTUP_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")}
    environment.update(HOME=str(home), TMPDIR=str(tmp), CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE="true",
                       RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                       CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                       GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    # Keep the real ShellCheck regression in the script tier: Rust tests must not start --all.
    gate = work / "script-gate"
    for relative in ("scripts/tests/test.sh", "scripts/utils/prune-cache.sh"):
        destination = gate / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(repo / relative, destination)
    (gate / "Cargo.toml").write_text("not a valid manifest\n")
    (gate / "scripts/utils/invalid.sh").write_text("#!/usr/bin/env bash\necho $UNQUOTED\n")
    checked = subprocess.run(["bash", str(gate / "scripts/tests/test.sh"), "--all"],
                             env=environment, capture_output=True, text=True)
    diagnostics = checked.stdout + checked.stderr
    assert checked.returncode != 0 and "SC2086" in diagnostics, diagnostics
    files = ["scripts/audits/check-publication.sh", "scripts/build/build-release.sh",
             "scripts/tests/test-verification-target.sh", "scripts/tests/test-benchmark-inputs.sh",
             "scripts/benchmarks/run-benchmarks.sh", "scripts/benchmarks/run-evaluation.sh"]
    files += ["scripts/utils/" + name for name in ("verification-target.sh", "verification-source.sh",
              "verification-lock.sh", "guard-benchmark.sh", "capture-benchmark-binary.sh", "cargo-artifact.sh",
              "check-build-space.sh", "prune-cache.sh", "sync-corpora.sh", "read-private-corpus.sh")]
    for count in (1, 2):
        root = work / ("project" + "\n" * count)
        (root / "src").mkdir(parents=True)
        (root / "skills/xcalygraph").mkdir(parents=True)
        (root / "Cargo.toml").write_text('[package]\nname="xcalygraph"\nversion="0.0.0"\nedition="2024"\n')
        (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "xcalygraph"\nversion = "0.0.0"\n')
        (root / ".gitignore").write_text('/.cache/\n/dist/\n/research/\n')
        (root / "src/main.rs").write_text('fn main() { println!("CURRENT_CHECKOUT"); }\n')
        (root / "skills/xcalygraph/SKILL.md").write_text("fixture skill\n")
        for relative in files:
            destination = root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(repo / relative, destination)
        (root / "scripts/audits/private-patterns.txt").write_text("# fixture\n")
        (root / "scripts/benchmarks/corpora.tsv").write_text("name\turl\tcommit\tlanguages\n")
        subprocess.run(["git", "init", "-q", str(root)], env=environment, check=True)
        subprocess.run(["git", "add", "Cargo.toml", "Cargo.lock", ".gitignore", "src", "skills"], cwd=root,
                       env=environment, check=True)
        subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture"],
                       cwd=root, env=environment, check=True)
        audit = subprocess.run(["bash", str(root / "scripts/audits/check-publication.sh"), "HEAD"], cwd=work,
                               env=environment, capture_output=True, text=True)
        assert audit.returncode == 0 and "1 commit(s) clean" in audit.stdout, audit.stderr
        sync = subprocess.run(["bash", str(root / "scripts/utils/sync-corpora.sh"), "not-selected"], cwd=work,
                              env=environment, capture_output=True, text=True)
        assert sync.returncode == 0 and (root / "research/corpora").is_dir(), sync.stderr
        unknown = root / ".cache/unknown"
        unknown.mkdir(parents=True)
        (unknown / "sentinel").write_text("keep\n")
        prune = subprocess.run(["bash", str(root / "scripts/utils/prune-cache.sh"), "0"], cwd=work,
                               env=environment, capture_output=True, text=True)
        assert prune.returncode == 0 and "removed 0 entries" in prune.stdout, prune.stderr
        assert (unknown / "sentinel").read_text() == "keep\n"
        invalid = subprocess.run(["bash", str(root / "scripts/build/build-release.sh"), "unsupported-target"], cwd=work,
                                 env=environment, capture_output=True, text=True)
        assert invalid.returncode != 0 and "unsupported target" in invalid.stderr, invalid.stderr
        (root / ".cache").mkdir(exist_ok=True)
        nested = subprocess.run(["bash", str(root / "scripts/tests/test-benchmark-inputs.sh")], cwd=work,
                                env=environment, capture_output=True, text=True)
        assert nested.returncode == 0 and "PASS:" in nested.stdout, nested.stderr
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        assert shutil.disk_usage(work).free > 2 * 1024**3 + 128 * 1024**2
        nested = subprocess.run(["bash", str(root / "scripts/tests/test-verification-target.sh")], cwd=work,
                                env=environment, capture_output=True, text=True)
        assert nested.returncode == 0 and "PASS:" in nested.stdout, nested.stderr
        binary = work / f"capture-{count}"
        subprocess.run(["df", "-h", "/"], check=True, stdout=subprocess.DEVNULL)
        assert shutil.disk_usage(work).free > 2 * 1024**3 + 128 * 1024**2
        capture = subprocess.run(["bash", str(root / "scripts/utils/guard-benchmark.sh"), "bash",
                                  str(root / "scripts/utils/capture-benchmark-binary.sh"), "--debug", str(binary)],
                                 cwd=work, env=environment, capture_output=True, text=True)
        assert capture.returncode == 0, capture.stderr
        assert json.loads(capture.stdout)["binary_sha256"] == hashlib.sha256(binary.read_bytes()).hexdigest()
        assert subprocess.check_output([str(binary)], text=True).strip() == "CURRENT_CHECKOUT"
        identity = hashlib.sha256(os.fsencode(root.resolve())).hexdigest()
        assert (target / ".xcalygraph-verification" / identity).is_dir()
        assert sentinel.read_text() == "preserve\n" and list(decoy.iterdir()) == [sentinel]
    print("PASS: exact newline-ending checkout roots, real guarded debug builds and script-tier ShellCheck failures")
PYTHON
