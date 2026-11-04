---
name: xcalygraph
description: Code graph CLI, no setup. Before reading source run `xcalygraph find WORDS` (where is X; a clear top hit includes its source and neighbours), `show NAME` (callers, callees, tests), `impact NAME`, `map`. Use rg or direct reads for exact text, known files, logs and non-code content.
---

# xcalygraph

Indexes the current repository on first use (seconds, nothing to set up) and refreshes itself on every query. Answers cite `path:line`.

## Pick the command

- Behavior described in words ("where is X handled"): `xcalygraph find WORDS`. A clear top hit comes with its source lines and neighbours: answer from it without another call. `ctx WORDS` adds related symbols.
- A name you already know: `xcalygraph show NAME` (signature, members, callers, callees, tests); no prefixes or quotes around NAME.
- New codebase: `map` first.
- Who breaks if this changes: `impact NAME` or `impact --diff`; route between two symbols: `path FROM TO`; review/cleanup: `audit`.
- Exact text, config values, logs, prose: `rg` or read the file.

Every further call costs far more than a longer answer: stop as soon as the output settles the question; read a cited `path:line` only if it does not.

## Commands

`map`, `find TERMS...`, `ctx TERMS...`, `show NAME`, `path FROM TO`, `impact NAME... | --diff [REV]`, `audit`; housekeeping: `index`, `ls`, `clean [ROOT...] [--missing]`, `install`, `update [--check]`, `version`; `help [COMMAND]` for every flag.

NAME: `pkg.Type.Method`, `crate::mod::fn`, `Store.Get`, `path:line`, `path::name`, a file path or a unique bare name. Exit 3 prints candidates: repeat with one of them.

Useful flags: `-p GLOB` (only under a path), `-l go,rust`, `-k fn,method`, `--exact` (drop inferred `?` facts), `-b N` (token cap), `-C DIR` (other repository).

## Reading the output

- `<line> <signature>` lines group under file paths; `?` = inferred, not proven. Indented below the top hit: `<line> <code>` from its body, `+N lines` left out, `<` caller, `>` callee.
- Card: `m` member, `f` fields, `^` supertype, `v` implementation, `<` caller, `>` callee, `t` test, `t2` test via a caller, `e` env read, `!` notice.
- `impact`: `t` test reaching the change, `r` command to run it; `moved?`, `sig?`, `uncertain?` mark inferred diff facts.
- `+N more` means truncated: narrow with `-p`/`-k` or raise `-b`.

Exit codes: 0 answer, 2 usage error, 3 ambiguous NAME, 4 no match. If the shared index location fails, xcalygraph uses `.xcalygraph/` in the repository: keep it out of commits.
