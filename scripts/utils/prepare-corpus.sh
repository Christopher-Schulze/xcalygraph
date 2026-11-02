#!/usr/bin/env bash
# Prepare a content-addressed, verified source snapshot, optionally with a matching Graphify graph.
# Usage: prepare-corpus.sh ROOT SNAPSHOT_BASE [--graphify]
#        prepare-corpus.sh --verify SNAPSHOT_SOURCE
# Prints only the source directory; evidence and tool output stay beside it. Existing artifacts
# are verified and reused, never overwritten or deleted. Requires python3, git/rg; Graphify for --graphify.
set -euo pipefail
python3 -I - "$@" <<'PYTHON'
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys

# Keep Git rooted at the requested corpus while preserving configured ignore rules.
GIT_CONTEXT_VARIABLES = (
    "GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_NAMESPACE", "GIT_PREFIX", "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM", "GIT_IMPLICIT_WORK_TREE",
)
GIT_ENVIRONMENT = os.environ.copy()
for name in GIT_CONTEXT_VARIABLES:
    GIT_ENVIRONMENT.pop(name, None)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(",", ":")).encode()


def source_files(root):
    git = subprocess.run(["git", "-C", str(root), "rev-parse", "--is-inside-work-tree"],
                         env=GIT_ENVIRONMENT, capture_output=True, check=False)
    ignored = subprocess.run(["git", "-C", str(root), "check-ignore", "-q", "."],
                             env=GIT_ENVIRONMENT, capture_output=True, check=False)
    command = ["git", "-C", str(root), "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "."]
    if git.returncode != 0 or ignored.returncode == 0:
        command = ["rg", "--files", "--hidden", "--no-ignore-parent", "-0", "-g", "!.git/**", "-g", "!.cache/**",
                   "-g", "!target/**", "-g", "!node_modules/**", "-g", "!graphify-out/**"]
    result = subprocess.run(command, cwd=root, env=GIT_ENVIRONMENT, capture_output=True, check=False)
    if result.returncode != 0:
        raise ValueError("source enumeration failed: " + result.stderr.decode(errors="replace"))
    files = []
    for raw in sorted(set(result.stdout.split(b"\0")) - {b""}):
        relative = os.fsdecode(raw)
        path = root / relative
        try:
            info = path.lstat()
        except FileNotFoundError:
            continue  # A deleted tracked file is absent from the working-tree snapshot.
        if not stat.S_ISREG(info.st_mode):
            continue
        if not path.resolve().is_relative_to(root):
            raise ValueError("source path escapes the corpus")
        data = path.read_bytes()
        after = path.stat()
        if (after.st_ino, after.st_size, after.st_mtime_ns, after.st_mode) != (info.st_ino, info.st_size, info.st_mtime_ns, info.st_mode):
            raise ValueError("source changed while hashing: " + relative)
        files.append({"path": relative, "sha256": digest(data), "bytes": len(data),
                      "executable": bool(info.st_mode & 0o111)})
    if not files:
        raise ValueError("empty or unsupported corpus")
    return files


def verify_source(root, files):
    expected_paths = {item["path"] for item in files}
    actual_paths = set()
    for directory, children, names in os.walk(root, followlinks=False):
        if Path(directory) == root:
            children[:] = [child for child in children if child != "graphify-out"]
        for child in children:
            if (Path(directory) / child).is_symlink():
                raise ValueError("snapshot contains a linked directory")
        actual_paths.update(str((Path(directory) / name).relative_to(root)) for name in names)
    if actual_paths != expected_paths:
        raise ValueError("snapshot source file set changed; existing evidence retained")
    for item in files:
        path = root / item["path"]
        if path.is_symlink() or not path.resolve().is_relative_to(root):
            raise ValueError("snapshot contains an unsafe source path")
        if digest(path.read_bytes()) != item["sha256"]:
            raise ValueError("snapshot source checksum mismatch: " + item["path"])
        if bool(path.stat().st_mode & 0o111) != item["executable"]:
            raise ValueError("snapshot executable mode mismatch: " + item["path"])


def graphify_identity():
    executable = shutil.which("graphify")
    if executable is None:
        raise ValueError("Graphify was requested but is not installed")
    shebang = Path(executable).read_text().splitlines()[0]
    interpreter = shebang.removeprefix("#!")
    if not shebang.startswith("#!") or not Path(interpreter).is_file():
        raise ValueError("cannot identify the Graphify package interpreter")
    probe = """import hashlib, importlib.metadata, json
d = importlib.metadata.distribution('graphifyy')
h = hashlib.sha256()
for f in sorted(d.files or [], key=str):
    if str(f).startswith('graphify/') and str(f).endswith('.py'):
        h.update(str(f).encode()); h.update(d.locate_file(f).read_bytes())
print(json.dumps({'version': d.version, 'code_sha256': h.hexdigest(), 'mode': ['update', '.'],
                  'environment': {'PYTHONHASHSEED': '0', 'PYTHONSAFEPATH': '1', 'PYTHONDONTWRITEBYTECODE': '1'}}))
"""
    result = subprocess.run([interpreter, "-I", "-c", probe], capture_output=True, text=True, check=True)
    return executable, json.loads(result.stdout)


def prepare(root, base, upstream):
    files = source_files(root)
    identity = {"schema": 1, "files": files}
    source_digest = digest(encoded(identity))
    executable, tool = graphify_identity() if upstream else (None, None)
    variant = digest(encoded(tool)) if tool else "source"
    target = base / source_digest / variant
    metadata = {"source_sha256": source_digest, "source": identity, "upstream": tool}
    if target.exists():
        if json.loads((target / "snapshot.json").read_text()) != metadata:
            raise ValueError("snapshot identity mismatch; existing evidence retained")
        verify_source(target / "source", files)
    else:
        required = sum(item["bytes"] for item in files)
        if shutil.disk_usage(base).free < max(4 * 1024**3, 2 * 1024**3 + required * 3):
            raise ValueError("insufficient free space for an isolated corpus and graph")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.mkdir()  # Exclusive creation; a collision preserves the other writer's evidence.
        source = target / "source"
        source.mkdir()
        for item in files:
            data = (root / item["path"]).read_bytes()
            if digest(data) != item["sha256"]:
                raise ValueError("source changed during snapshot copy: " + item["path"])
            path = source / item["path"]
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as stream:
                stream.write(data)
            path.chmod(0o755 if item["executable"] else 0o644)
        with (target / "snapshot.json").open("xb") as stream:
            stream.write(encoded(metadata))
    source = target / "source"
    if upstream:
        graph = source / "graphify-out" / "graph.json"
        proof = target / "graph.json.identity"
        if proof.exists():
            expected = {"snapshot": digest(encoded(metadata)), "graph_sha256": digest(graph.read_bytes())}
            if json.loads(proof.read_text()) != expected:
                raise ValueError("Graphify graph identity mismatch; existing evidence retained")
        else:
            if graph.exists() or (target / "graphify.log").exists():
                raise ValueError("incomplete Graphify build; existing evidence retained")
            home = target / "home"
            home.mkdir()
            environment = dict(GIT_ENVIRONMENT, HOME=str(home), **tool["environment"])
            with (target / "graphify.log").open("xb") as log:
                result = subprocess.run([executable, "update", "."], cwd=source, env=environment,
                                        stdout=log, stderr=subprocess.STDOUT, check=False)
            if result.returncode != 0:
                raise ValueError("Graphify extraction failed; inspect " + str(target / "graphify.log"))
            content = json.loads(graph.read_text())
            if not isinstance(content.get("nodes"), list) or not content["nodes"] or not isinstance(content.get("links"), list):
                raise ValueError("Graphify produced an empty or invalid graph")
            verify_source(source, files)
            with proof.open("xb") as stream:
                stream.write(encoded({"snapshot": digest(encoded(metadata)), "graph_sha256": digest(graph.read_bytes())}))
    return source


def verify_snapshot(source):
    metadata = json.loads((source.parent / "snapshot.json").read_text())
    if digest(encoded(metadata["source"])) != metadata["source_sha256"]:
        raise ValueError("snapshot metadata checksum mismatch")
    verify_source(source, metadata["source"]["files"])
    if metadata["upstream"] is not None:
        if graphify_identity()[1] != metadata["upstream"]:
            raise ValueError("installed Graphify identity changed")
        proof = json.loads((source.parent / "graph.json.identity").read_text())
        expected = {"snapshot": digest(encoded(metadata)),
                    "graph_sha256": digest((source / "graphify-out/graph.json").read_bytes())}
        if proof != expected:
            raise ValueError("Graphify graph identity mismatch; existing evidence retained")
    return source


try:
    if len(sys.argv) == 3 and sys.argv[1] == "--verify":
        print(verify_snapshot(Path(sys.argv[2]).resolve(strict=True)))
        sys.exit(0)
    if len(sys.argv) not in (3, 4) or len(sys.argv) == 4 and sys.argv[3] != "--graphify":
        raise ValueError("usage: prepare-corpus.sh ROOT SNAPSHOT_BASE [--graphify]")
    root, base = Path(sys.argv[1]).resolve(strict=True), Path(sys.argv[2]).resolve()
    base.mkdir(parents=True, exist_ok=True)
    print(prepare(root, base, len(sys.argv) == 4))
except (OSError, ValueError, subprocess.SubprocessError) as error:
    print("prepare-corpus.sh: " + str(error), file=sys.stderr)
    sys.exit(1)
PYTHON
