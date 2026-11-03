---
name: xcalygraph
description: Code graph CLI for repository structure, definitions, callers, implementations and impact. Use before source reads; use rg or direct reads for exact text, known files, logs and non-code content.
---

# xcalygraph

Local repository index: symbols, relationships, tests and env reads. Queries refresh incrementally; answers cite `path:line`.

## When

- Unknown code: `map`, then `find`/`ctx`.
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

NAME: `pkg.Type.Method`, `crate::mod::fn`, qualified suffix (`Store.Get`), `path:line`, `path::name`, file path or unique bare name. Paths incl. `./` aliases win collisions; check spelling before trim.

Flags: `-p GLOB` (repeatable, gitignore); `-l go,rust`; `-k fn,method,struct,option`; `--no-tests`; `--exact` (drop inferred); `-d N`; `--all`; `-b N` (hard token cap); `--json` (for programs; text answers more per token); `--read-only` (no writes; exit 5 if stale); `-C DIR` (root).

## Reading the output

- `<line> <signature>` lines group under file paths.
- Decode escaped paths/commands before use.
- Card: `m`=member, `f`=fields, `^`=supertype, `v`=implementation, `<`=caller, `>`=definition, `t`=test, `t2`=test via caller, `e`=env, `!`=notice.
- `option -x`: `arg=none|required;case=<line|*:line|none|ambiguous>`.
- A symbol declared in several places lists each location.
- `?`=inferred (heuristic) edge or impact; `--exact` omits them.
- `impact`: `t`=test reaching the change, `r`=command that runs it; no `r` = no supported runner.
- `impact --diff`: removals cite REV path:line; `moved?`=locations; `sig?`=inferred signature change; `uncertain?`=ambiguous/missing identity. Sensitive/oversized/nonregular sides are path-only; missing REV blobs error; inspect beyond filters.
- Truncation: `+N more`, `+N`, `+` or JSON `{}`; narrow with filters or raise `-b`.
- Shared-index failure: use local `.xcalygraph/`; ignore it in git.

## Exit codes

0 answer, 2 usage error, 3 ambiguous NAME (candidates printed: repeat with a qualified name or `path::name`), 4 no match, 5 stale under `--read-only`.

## Rules

- Read the cited `path:line` before editing.
- In read-only tasks pass `--read-only`.
- `show` exposes callers/callees without whole-file reads.
