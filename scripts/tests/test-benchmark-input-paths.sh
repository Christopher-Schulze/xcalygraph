#!/usr/bin/env bash
# Exercise benchmark path selection without running agents or starting a build.
# Usage: scripts/tests/test-benchmark-input-paths.sh
set -euo pipefail
(( $# == 0 )) || { echo "usage: test-benchmark-input-paths.sh" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "$0")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
[[ ! -L "${REPO_ROOT}/.cache" ]] || { echo "test cache must not be a symlink" >&2; exit 1; }
mkdir -p "${REPO_ROOT}/.cache"
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="benchmark-input-paths.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    home, tools, foreign = work / "home", work / "tools", work / "foreign"
    for directory in (home, tools, foreign):
        directory.mkdir()
    shell = shutil.which("bash")
    assert shell
    for name in ("bash", "dirname", "basename", "awk", "seq", "jq", "rg", "python3"):
        executable = shutil.which(name)
        assert executable, name
        (tools / name).symlink_to(executable)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "XCALYGRAPH_", "BASH_FUNC_", "CARGO_"))
                   and key not in ("BASH_ENV", "ENV", "CDPATH", "TAR_OPTIONS")}
    environment.update(HOME=str(home), PATH=str(tools), XDG_CONFIG_HOME=str(home / "config"))
    assert shutil.which("cargo", path=environment["PATH"]) is None
    dash, old_directory = work / "-", work / "old-directory"
    dash.mkdir()
    old_directory.mkdir()
    (dash / "question.tsv").write_text("question\texpected\nFind Create\tmain.go::Create\n")
    environment["OLDPWD"] = str(old_directory)

    def assignment(name, value):
        result = subprocess.run([shell, "-x", "-c", name + '="$1"', "trace", value],
                                env=environment, capture_output=True, check=True)
        return result.stderr

    for count in (1, 2):
        decoy = work / f"checkout-{count}"
        decoy.mkdir()
        sentinel = decoy / "sentinel"
        sentinel.write_text("preserve\n")
        root = work / (f"checkout-{count}" + "\n" * count)
        benchmark = root / "scripts/benchmarks"
        benchmark.mkdir(parents=True)
        for name in ("run-agent-benchmark.sh", "run-evaluation.sh", "measure-query-compute.sh"):
            shutil.copyfile(repo / "scripts/benchmarks" / name, benchmark / name)
        helper = repo / "scripts/utils/read-private-corpus.sh"
        if helper.is_file():
            (root / "scripts/utils").mkdir()
            shutil.copyfile(helper, root / "scripts/utils" / helper.name)
        questions = benchmark / "questions"
        (questions / "private").mkdir(parents=True)
        (questions / "xcalygraph.tsv").write_text("question\texpected\nFind Create\tmain.go::Create\n")
        (root / ".cache").symlink_to(foreign, target_is_directory=True)

        def invoke(script, *arguments, trace=False):
            command = [shell]
            if trace:
                command.append("-x")
            command.extend((str(benchmark / script), *map(str, arguments)))
            return subprocess.run(command, cwd=work, env=environment, capture_output=True)

        plan = invoke("run-agent-benchmark.sh", "--runs", "1", "xcalygraph")
        assert plan.returncode == 0 and ("(git archive of " + str(root) + "), arm none").encode() in plan.stdout, plan.stderr
        unknown = invoke("run-agent-benchmark.sh", "--runs", "1", "unknown-corpus")
        assert unknown.returncode != 0 and b"codex exec" not in unknown.stdout
        evaluation = invoke("run-evaluation.sh", "--no-upstream", "xcalygraph", trace=True)
        assert evaluation.returncode != 0 and b"evaluation cache must not be a symlink" in evaluation.stderr
        assert assignment("root", str(root)) in evaluation.stderr

        corpus = work / (f"corpus-{count}" + "\n" * count)
        corpus.mkdir()
        (work / f"corpus-{count}").mkdir()
        question = work / (f"question-{count}" + "\n" * count)
        question.write_text("question\texpected\nFind Create\tmain.go::Create\n")
        (work / f"question-{count}").write_text("decoy\n")
        query = invoke("measure-query-compute.sh", os.path.relpath(corpus, work), os.path.relpath(question, work), "1", trace=True)
        assert query.returncode != 0 and b"benchmark cache must not be a symlink" in query.stderr
        assert assignment("ROOT", str(corpus.resolve())) in query.stderr
        assert assignment("QUESTIONS", str(question.resolve())) in query.stderr
        question_directory = work / (f"question-dir-{count}" + "\n" * count)
        question_directory.mkdir()
        (work / f"question-dir-{count}").mkdir()
        nested_question = question_directory / "nested.tsv"
        nested_question.write_text("question\texpected\nFind Create\tmain.go::Create\n")
        (work / f"question-dir-{count}" / "nested.tsv").write_text("decoy\n")
        alias = work / f"question-alias-{count}"
        alias.symlink_to(question_directory, target_is_directory=True)
        nested_query = invoke("measure-query-compute.sh", os.path.relpath(corpus, work),
                              os.path.relpath(alias / "nested.tsv", work), "1", trace=True)
        assert b"benchmark cache must not be a symlink" in nested_query.stderr
        assert assignment("QUESTIONS", str(nested_question.resolve())) in nested_query.stderr
        dash_query = invoke("measure-query-compute.sh", "-", "-/question.tsv", "1", trace=True)
        assert b"benchmark cache must not be a symlink" in dash_query.stderr
        assert assignment("ROOT", str(dash)) in dash_query.stderr
        assert assignment("QUESTIONS", str(dash / "question.tsv")) in dash_query.stderr

        private = work / f"private-{count}"
        private.mkdir()
        listing = questions / "private/corpora.tsv"
        (questions / "private/private-name.tsv").write_text("question\texpected\nFind Create\tmain.go::Create\n")
        listing.write_text(f"private-name\t{private}\n")
        private_plan = invoke("run-agent-benchmark.sh", "--runs", "1", "private-name")
        assert private_plan.returncode == 0 and ("(git archive of " + str(private) + "), arm none").encode() in private_plan.stdout
        private_evaluation = invoke("run-evaluation.sh", "--no-upstream", "private-name", trace=True)
        assert b"evaluation cache must not be a symlink" in private_evaluation.stderr
        assert assignment("root", str(private)) in private_evaluation.stderr
        relative = os.path.relpath(private, work)
        listing.write_text(f"# private corpora\nprivate-name\t{relative}")
        relative_plan = invoke("run-agent-benchmark.sh", "--runs", "1", "private-name")
        assert relative_plan.returncode == 0 and ("(git archive of " + relative + "), arm none").encode() in relative_plan.stdout
        relative_evaluation = invoke("run-evaluation.sh", "--no-upstream", "private-name", trace=True)
        assert b"evaluation cache must not be a symlink" in relative_evaluation.stderr
        assert assignment("root", str(work / relative)) in relative_evaluation.stderr
        listing.write_text("private-name\t-\n")
        dash_plan = invoke("run-agent-benchmark.sh", "--runs", "1", "private-name")
        assert dash_plan.returncode == 0 and b"(git archive of ./-), arm none" in dash_plan.stdout

        for name, directory in (("graphify", root / "research/upstream/graphify"),
                                ("hono", root / "research/corpora/hono")):
            directory.mkdir(parents=True)
            (questions / (name + ".tsv")).write_text("question\texpected\nFind Create\tmain.go::Create\n")
        listing.write_bytes(b"absent\tmissing-corpus")
        default = invoke("run-evaluation.sh", "--no-upstream")
        assert default.returncode != 0 and b"missing corpus or questions" in default.stderr
        assert b"evaluation cache must not be a symlink" not in default.stderr

        tricky = "literal\\nname"
        (questions / "private" / (tricky + ".tsv")).write_text("question\texpected\nFind Create\tmain.go::Create\n")
        listing.write_text(f"{tricky}\t{private}\n")
        tricky_plan = invoke("run-agent-benchmark.sh", "--runs", "1", tricky)
        assert tricky_plan.returncode == 0 and ("(git archive of " + str(private) + "), arm none").encode() in tricky_plan.stdout

        valid = os.fsencode(private)
        for data in (b"private-name\t" + valid + b"\ncontinuation\n",
                     b"private-name\t" + valid + b"\textra\n",
                     b"private-name\t" + valid + b"\x00\n",
                     b"private-name\t" + valid + b"\nprivate-name\t" + valid + b"\n"):
            listing.write_bytes(data)
            invalid = invoke("run-agent-benchmark.sh", "--runs", "1", "private-name")
            assert invalid.returncode != 0 and b"codex exec" not in invalid.stdout
            invalid_eval = invoke("run-evaluation.sh", "--no-upstream", "private-name")
            assert invalid_eval.returncode != 0 and b"evaluation cache must not be a symlink" not in invalid_eval.stderr
        assert sentinel.read_text() == "preserve\n" and list(decoy.iterdir()) == [sentinel]
        assert not list(foreign.iterdir())
    print("PASS: exact benchmark corpus and question paths, decoys and invalid private records before any build or agent")
PYTHON
