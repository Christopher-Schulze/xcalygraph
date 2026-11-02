---
name: xcalygraph
description: Code graph CLI for structure questions: where something is defined or handled, who calls or uses it, what implements an interface, what a change affects, how a repository is laid out. Use before reading files when locating code or tracing relations; use rg or direct reads for exact text, known files, logs and non-code content.
---

# xcalygraph

Local, deterministic index of the repository (symbols, calls, type uses, imports, implementations, tests, env reads). Every query refreshes the index incrementally first; answers cite `path:line`.

## When

- Unknown code: `map`, then `find` or `ctx`.
- "Where is X handled", "who calls X", "what implements X", "path from A to B": `find`, `show`, `path`.
- Before or after editing: `impact NAME` or `impact --diff`; cleanup and review: `audit`.
- Exact strings, config values, logs, prose: `rg` and direct reads are better.

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
| `xcalygraph install`, `update [--check]`, `version` | install or repair binary and skill; self-update with skill refresh; versions |
| `xcalygraph help [COMMAND]` | full syntax of every command and flag |

NAME: `pkg.Type.Method`, `crate::mod::fn`, a qualified suffix (`Store.Get`), `path:line`, `path::name`, a file path, or a unique bare name. A file-path match wins locator collisions, including through the existing `./` alias; xcalygraph checks the input spelling first, then its surrounding-whitespace-trimmed form before locator syntax.

Flags: `-p GLOB` (paths, gitignore syntax, repeatable), `-l go,rust` (languages), `-k fn,method,struct` (kinds), `--no-tests`, `--exact` (drop inferred edges), `-d N` (show depth), `--all` (show without caps), `-b N` (token budget, hard cap), `--json`, `--read-only` (never write; exit 5 when stale), `-C DIR` (root).

## Reading the output

- `<line> <signature>` lines are grouped under their file path.
- Map JSON adds counts, task runs, API sig/handlers, cycle/layer members and env path/line; audit separates name/path/line and adds drift kind/status.
- Card markers: `m` member, `f` fields, `^` supertype, `v` implementation, `<` incoming (caller site), `>` outgoing (definition), `t` test, `t2` test through one caller, `e` env read, `!` notice.
- `?` marks inferred name/dispatch edges and indirect test/impact paths; `--exact` drops them and dependent test commands.
- `impact` run lines retain Go module/package, Cargo manifest and pytest node-ID context; args are quoted. Unsupported runners emit `t`, no `r`.
- `impact --diff` cites REV path/line for removals, both locations for `moved?`, `sig?` for inferred signature changes and `uncertain?` for ambiguous/unavailable identities; inspect incomplete paths beyond filters too.
- Truncation: `+N more`, `+N`, `+` or JSON `{}`; narrow with `-p`, `-k`, `-l` or raise `-b`.
- A shared-index failure can use `.xcalygraph/` locally; the stderr notice asks to ignore it in git.

## Exit codes

0 answer, 2 usage error, 3 ambiguous NAME (candidates printed: repeat with a qualified name or `path::name`), 4 no match, 5 stale under `--read-only`.

## Rules

- Open the cited `path:line` before editing; the graph locates, the source decides.
- In read-only tasks pass `--read-only`.
- Prefer `show` on a found name over reading whole files to see callers and callees.
