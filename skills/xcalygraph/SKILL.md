---
name: xcalygraph
description: Code graph CLI for locating definitions, handlers, callers, implementations, impact and repository structure. Use before source reads for code navigation; use rg or direct reads for exact text, known files, logs and non-code content.
---

# xcalygraph

Local deterministic repository index: symbols, calls, type uses, imports, implementations, tests and env reads. Queries refresh incrementally; answers cite `path:line`.

## When

- Unknown code: `map`, then `find` or `ctx`.
- "Where is X handled", "who calls X", "what implements X", "path from A to B": `find`, `show`, `path`.
- Before or after editing: `impact NAME` or `impact --diff`; cleanup and review: `audit`.
- Exact strings/config/logs/prose: use `rg` or read directly.

## Commands

| command | use |
|---|---|
| `xcalygraph map` | languages, largest modules with top symbols, entrypoints, tasks (make, scripts, CI, services) with what they run, hubs, API operations with handlers, cycles, layers, external deps, env vars |
| `xcalygraph find TERMS...` | ranked symbols and files, one line each; natural-language terms work |
| `xcalygraph ctx TERMS...` | best matches with doc sentence plus related symbols, grouped by file |
| `xcalygraph show NAME` | card: signature, doc, members, supertypes, implementations, callers, callees, tests, env reads owned by the symbol or file when unscoped |
| `xcalygraph path FROM TO` | shortest dependency path |
| `xcalygraph impact NAME... \| --diff [REV]` | dependents of a change, tests reaching them, test commands to run |
| `xcalygraph audit` | dead code, unused exports, cycles, stale doc references, env drift, unreferenced assets, files co-changed without static link, hotspots |
| `xcalygraph index`, `ls`, `clean [ROOT...] [--missing]` | refresh explicitly; list indexes (root, size, last use); delete all, some or orphaned indexes, preserving unverified entries |
| `xcalygraph install`, `update [--check]`, `version` | install/repair binary and skill; version flags stamp/content mismatch; queries stay silent |
| `xcalygraph help [COMMAND]` | full syntax of every command and flag |

Use `exact symbol NAME` in `find`/`ctx`; misses return no match; `ctx` may add related context.

NAME: `pkg.Type.Method`, `crate::mod::fn`, a qualified suffix (`Store.Get`), `path:line`, `path::name`, a file path, or a unique bare name. A file-path match wins locator collisions, including through the existing `./` alias; xcalygraph checks the input spelling first, then its surrounding-whitespace-trimmed form before locator syntax.

Flags: `-p GLOB` (paths, gitignore syntax, repeatable), `-l go,rust` (languages), `-k fn,method,struct` (kinds), `--no-tests`, `--exact` (drop inferred edges), `-d N` (show depth), `--all` (show without caps), `-b N` (token budget, hard cap), `--json`, `--read-only` (never write; exit 5 when stale), `-C DIR` (root).

## Reading the output

- `<line> <signature>` lines group under file paths.
- Decode escaped paths/commands before use.
- Rust `#[path]`: exact file; shared-file aliases unresolved, no same-name fallback.
- Map JSON: counts, task runs, API sig/handlers, cycle/layer members, env path/line; audit: name/path/line, drift kind/status.
- Card: `m` member, `f` fields, `^` supertype, `v` implementation, `<` caller, `>` definition, `t` test, `t2` test via caller, `e` env, `!` notice.
- `?`=inferred edges/impact; Julia Base uses typed args; Rust closure calls use generic trait bounds; `--exact` drops inferred edges/runs.
- `impact` run lines retain Go module/package, Cargo manifest and pytest node-ID context; args are quoted. Unsupported runners emit `t`, no `r`.
- `impact --diff`: removals cite REV path:line; `moved?` gives both locations; `sig?`=inferred signature change; `uncertain?`=ambiguous/unavailable identity; sensitive, oversized or non-regular sides get path-only notes; missing known REV blobs error; inspect paths beyond filters.
- Truncation: `+N more`, `+N`, `+` or JSON `{}`; narrow with `-p`, `-k`, `-l` or raise `-b`.
- On shared-index failure, use local `.xcalygraph/`; stderr asks to ignore it in git.

## Exit codes

0 answer, 2 usage error, 3 ambiguous NAME (candidates printed: repeat with a qualified name or `path::name`), 4 no match, 5 stale under `--read-only`.

## Rules

- Open the cited `path:line` before editing; the graph locates, the source decides.
- In read-only tasks pass `--read-only`.
- `show` exposes callers/callees without whole-file reads.
