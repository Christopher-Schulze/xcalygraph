# xcalygraph Specification

## 1. Scope

- Input: one root directory. Output: a persistent index per root plus command output on stdout.
- Offline and deterministic: no LLM, no network except `update`; the same index and arguments yield byte-identical output.
- Consumer: AI agents through the CLI and the embedded skill (section 10).

## 2. Command line

Syntax: `xcalygraph <command> [args] [flags]`.

| flag | default | description |
|------|---------|-------------|
| `-C, --root PATH` | nearest ancestor of cwd containing `.git`, else cwd | index root |
| `-b, --budget N` | per command | output budget in tokens; estimate = ceil(bytes / 3) |
| `--json` | off | JSON Lines instead of text (section 8) |
| `--read-only` | off | never create or refresh the index; exit 5 if missing or stale |
| `--index-dir PATH` | `$XCALYGRAPH_INDEX_DIR`, else `$XDG_CACHE_HOME/xcalygraph`, else `~/.cache/xcalygraph` | index base directory |

| command | arguments | budget | result |
|---------|-----------|--------|--------|
| `map` | `[PATH]` | 2500 | orientation (7.1); PATH narrows to a subtree |
| `find` | `TERMS...` | 600 | ranked symbols and files, one line each (7.2) |
| `ctx` | `TERMS...` | 2000 | context bundle: seeds plus related symbols with signatures (7.3) |
| `show` | `NAME [--all]` | 1200 | symbol card (7.4); `--all` lifts per-section caps |
| `impact` | `NAME...` or `--diff [REV]` | 1500 | changed and affected symbols, tests, run commands (7.5) |
| `path` | `FROM TO` | 400 | shortest dependency path (7.6) |
| `audit` | `[PATH]` | 2000 | dead code, cycles, stale doc references, hidden coupling, hotspots (7.7) |
| `index` | `[--drop]` | none | explicit build or refresh; `--drop` deletes the index of the root |
| `install` | `[--link claude]` | none | install the embedded skill (section 10) |
| `update` | `[--check]` | none | self-update the binary, then `install` |
| `version` | none | none | binary, index format and installed skill versions |
| `help` | `[COMMAND]` | none | syntax reference |

- Query commands (`map`, `find`, `ctx`, `show`, `impact`, `path`, `audit`) refresh the index first (section 4) unless `--read-only`.
- NAME forms: `qualified.name`; `path:line`; `path::name`; bare name (must be unique).
- Aliases: `-h`, `--help` = `help`; `<command> -h|--help` = `help <command>`; `-V`, `--version` = `version`.
- Usage errors: one stderr line `xcalygraph: <error>; run 'xcalygraph help'`, exit 2. A reader that closes stdout early is not an error.

| exit code | meaning |
|-----------|---------|
| 0 | answer |
| 1 | runtime error |
| 2 | usage error |
| 3 | ambiguous NAME; candidates listed |
| 4 | no match |
| 5 | index missing or stale under `--read-only` |

## 3. Corpus model

| role | detection | indexing |
|------|-----------|----------|
| `src` | supported language file without another role | full extraction |
| `test` | language test convention (table below) | full extraction; rank weight 0.3 |
| `gen` | first 1 KiB contains `Code generated` with `DO NOT EDIT`, or `@generated` | full extraction; rank weight 0.2; excluded from `map` and `audit` |
| `fixture` | path segment `testdata`, `fixtures`, `__fixtures__` or `__snapshots__` | path only |
| `doc` | `.md`, `.mdx` | headings, links, code mentions |
| `manifest` | `go.mod`, `Cargo.toml`, `package.json`, `pyproject.toml` | module metadata, dependencies, entrypoints |
| `sensitive` | `.env*` except `.env.example`; `*.pem`, `*.key`, `*.p12`, `id_rsa*`, `id_ed25519*`; anything under `.ssh/`, `.aws/`, `.gnupg/` | path only; content never read |
| `binary` | NUL byte in the first 8 KiB | path only |
| `large` | size > 8 MiB | path only |
| `other` | everything else | path only |

| language | detection | test convention |
|----------|-----------|-----------------|
| Go | `.go` | `_test.go` files; `Test*`, `Benchmark*`, `Fuzz*`, `Example*` functions |
| Rust | `.rs` | `#[test]` functions, `#[cfg(test)]` modules, files under `tests/` |
| TypeScript | `.ts`, `.tsx`, `.mts`, `.cts` | `*.test.*`, `*.spec.*`, files under `__tests__/` |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` | as TypeScript |
| Python | `.py`, `.pyi` | `test_*.py`, `*_test.py`, files under `tests/`; `test_*` functions |
| Bash | `.sh`, `.bash`; extensionless files with `sh`, `bash` or `zsh` shebang | none |
| Markdown | `.md`, `.mdx` | none |

- Ignore rules, lowest to highest precedence: global git excludes (`$XDG_CONFIG_HOME/git/ignore`), `.git/info/exclude`, nested `.gitignore`, nested `.xcalygraphignore` (gitignore syntax).
- Always pruned: `.git`, `.hg`, `.svn`, `node_modules`, `__pycache__`, `graphify-out`, directories containing `CACHEDIR.TAG` or `pyvenv.cfg`, `vendor` next to `go.mod` when `vendor/modules.txt` exists.
- Hidden files are indexed; symlinks are not followed.

## 4. Pipeline

walk -> stat diff -> hash candidates -> extract changed files (parallel) -> write changed fact shards -> resolve all facts (parallel per file) -> derive graph, ranks, modules, search index -> write derived data if changed -> answer.

Freshness:

1. A file is unchanged when path, size and mtime_ns equal its stored record and mtime_ns is older than the stored index write time minus 2 s.
2. Otherwise the XXH3-128 content hash decides; an equal hash updates only the stat record.
3. New, changed and deleted files form the change set; an empty change set answers from the existing index without writes.
4. A format or extraction version mismatch triggers a full rebuild.

- Locking: writers hold an exclusive advisory lock on `<index>/lock`, waiting at most 30 s before exit 1; readers never lock and map the files current at open time.
- Parallelism: walk, hashing, extraction and per-file resolution run on a work-stealing pool sized to the available cores; one parser and one reusable arena per worker, reset per file.
- IO: files up to 1 MiB are read into a per-worker reusable buffer; larger files are memory-mapped; index files are memory-mapped for reading.

## 5. Extraction

Per language: tree-sitter grammar plus an embedded query file; query captures map to fact records.

| fact | fields |
|------|--------|
| symbol | kind, name, parent symbol, line start, line end, signature (one line, whitespace-normalized, body removed, max 160 chars), doc (first sentence of the attached doc comment, max 120 chars), visibility (exported or private), flags (test, entry, generated) |
| import | module specifier, alias, imported names, line |
| ref | name (identifier or last two segments of a member chain), kind (call, type, inherit, implement), enclosing symbol, line |
| env | variable name, enclosing symbol, line |
| fields | owning type, field names (card content only, no graph nodes) |
| mention | doc heading, backticked identifier or relative path, line |
| link | doc heading, target path, line |
| manifest | module name; dependency name and version requirement; workspace members; entrypoints (package.json `bin` and `scripts`, pyproject `[project.scripts]`, Cargo `[[bin]]`, `src/main.rs`, `src/bin/*.rs`) |

- Symbol kinds: `mod`, `struct`, `class`, `enum`, `iface`, `trait`, `alias`, `fn`, `method`, `const`, `var`, `macro`, `heading`.
- Never emitted: locals, parameters, generic type parameters, anonymous functions (their calls count for the enclosing symbol), fields as nodes, symbols without source location.
- Env reads: Go `os.Getenv`, `os.LookupEnv`; Rust `std::env::var`, `std::env::var_os`, `env!`, `option_env!`; TypeScript/JavaScript `process.env.X`, `process.env["X"]`, `import.meta.env.X`; Python `os.environ[...]`, `os.environ.get`, `os.getenv`.

## 6. Resolution and derived data

| language | module unit | import resolution | call and type resolution |
|----------|-------------|-------------------|--------------------------|
| Go | package directory | module path from `go.mod` plus import path -> directory | `pkg.Name` through import alias: exact; same-package name: exact; `x.M()`: unique method name in own or imported packages: inferred |
| Rust | module tree from `mod` declarations and file layout; crate from `Cargo.toml` | `use` paths with `crate::`, `super::`, `self::`, workspace crates | path-qualified name: exact; `x.m()`: unique method name in scope crates: inferred |
| TypeScript, JavaScript | file | relative specifiers with extension and index probing, `tsconfig.json` `paths` and `baseUrl`, workspace packages | imported binding: exact; member of an imported namespace: exact; other member calls: unique name: inferred |
| Python | module file or package; source roots: root, `src/` | absolute and relative imports | imported name: exact; module attribute: exact; method calls: unique name: inferred |
| Bash | file | `source` and `.` with a literal relative path | function in the file or a sourced file: exact |
| Markdown | file | relative links | backticked identifier with a unique symbol name: inferred; relative path mention: exact |

- Implementation edges: Rust `impl Trait for Type`, explicit `implements` and base classes: exact; Go: type method set contains every interface method name with equal parameter count: inferred.
- Edge kinds: `calls`, `uses` (type reference), `inherits`, `implements`, `imports` (file -> file or module), `mentions` (doc -> symbol or file), `tests` (test symbol -> symbol it calls).
- Confidence: `exact` or `inferred`. Several candidates: no edge, counted per file as ambiguous. Targets outside the root: external dependency per module (name, use count), no node.
- Edges are unique per (source, target, kind); exact wins over inferred.

| derived item | definition |
|--------------|------------|
| symbol graph | CSR forward and reverse adjacency over resolved edges |
| rank | PageRank, damping 0.85, 30 iterations, edge weight exact 1.0 and inferred 0.5; `test` and `gen` symbols excluded |
| modules | language module units; module graph = aggregated cross-module edges with counts |
| cycles | strongly connected components with more than one module in the module import graph |
| layers | longest-path level per module on the condensed module DAG |
| entrypoints | Go `main` in package `main`; Rust `main` of bin targets; manifest entrypoints; Python `if __name__ == "__main__"` blocks; executable files with shebang |
| test map | symbol -> tests reaching it within 2 `calls` hops |
| dead candidates | private, non-entry, non-test symbols without incoming edges, excluding interface and trait implementations and init hooks; unused exported symbols listed separately |
| stale doc references | doc mentions of identifiers matching no symbol and of relative paths matching no file |
| search index | token -> symbol postings; tokens from name, parent chain, path, signature, doc |

- Tokenization: split on non-alphanumerics, camelCase and PascalCase boundaries and letter-digit boundaries; lowercase; an English stopword list applies to query terms only.

Git data requires `git` on PATH and a git work tree; otherwise git sections print `! git unavailable`. Cached in the `git` store file keyed by HEAD.

| git item | definition |
|----------|------------|
| diff | `git diff --unified=0 REV` (default `HEAD`, includes the working tree) -> changed line ranges |
| churn | commits touching each file within the last 1000 non-merge commits |
| co-change | file pairs changed together in at least 3 commits with Jaccard >= 0.3; commits touching more than 50 files ignored |
| hidden coupling | co-change pairs without a static edge between the two files |
| hotspots | churn x (1 + max symbol rank in the file), top 10 |

## 7. Query semantics

### 7.1 map

Sections in priority order: root line (name, files per language, symbol count) -> modules (tree by path, depth max 3; files, symbols, top 3 symbols by rank; max 40 lines) -> entrypoints -> hubs (top 10 by rank) -> cycles -> layers -> external dependencies (top 15 by use count) -> env vars (name, read count, first location). Sections other than modules hold max 10 lines; the budget cuts from the lowest-priority section upward.

### 7.2 find

1. Tokenize the query; look up exact names for the raw query and each term.
2. Candidates = union of the postings.
3. Score = BM25F (k1 1.2, b 0.75; field weights name 3, parent 1.5, path 1, signature 0.5, doc 0.5) + exact-name bonus 5, multiplied by the role weight (src 1, test 0.3, gen 0.2, doc 0.5), + 0.5 x normalized log rank.
4. Order: score descending, then path, then line; files matched by path tokens appear as file lines.

### 7.3 ctx

1. Seeds = top 5 `find` results with score >= 0.25 x top score.
2. Personalized PageRank from the seeds over the symbol graph in both directions, restart probability 0.3, 15 iterations.
3. Order: seeds first, then PPR score x role weight.
4. Packing: group by file in first-appearance order; seeds with signature and doc, others with signature; stop at the budget.

### 7.4 show

Card order: header (path:start-end, kind, qualified name) -> signature -> doc -> members (methods, field names) -> supertypes -> implementations -> callers by rank -> callees -> tests -> env reads -> ambiguity notices. Each section max 10 lines unless `--all`.

### 7.5 impact

1. Targets: NAME symbols, or symbols whose line ranges intersect the `--diff` ranges. Exported signature changes: extract the REV version of each changed file (`git show REV:path`) and compare signatures.
2. Affected: reverse BFS over `calls`, `uses`, `inherits`, `implements` up to depth 3, ordered by depth, then rank.
3. Tests: tests reaching any target or affected symbol (test map).
4. Run lines: Go `go test ./<pkg> -run '^(<names>)$'` per package; Rust `cargo test -p <crate> -- <names>` per crate; Python `pytest <path>::<name> ...`; TypeScript and JavaScript list test files only.

### 7.6 path

Shortest path by BFS over forward `calls`, `uses`, `inherits`, `implements`, `imports` edges; without a directed path it falls back to undirected search and marks the result inferred. One line per hop.

### 7.7 audit

Sections: dead candidates -> unused exports -> cycles -> stale doc references -> hidden coupling -> hotspots; each max 15 lines.

## 8. Output format

Text:

- Paths are root-relative with `/`. Symbol lines are grouped under their file line and indented two spaces: `<line> <kind> <signature or name>`.
- No headers, banners or empty fields. Truncation emits one final line `+<n> more`.
- `map` and `audit` open each section with one keyword line: `modules`, `entry`, `hubs`, `cycles`, `layers`, `ext`, `env`, `dead`, `unused`, `stale`, `coupling`, `hotspots`.

| marker | meaning |
|--------|---------|
| `<` | incoming: caller, user, importer |
| `>` | outgoing: callee, used type, import |
| `^` | supertype or implemented interface or trait |
| `v` | subtype or implementation |
| `t` | test |
| `e` | environment variable read |
| `r` | run command |
| `~` | changed symbol |
| `!` | notice: ambiguity, stale reference, unavailable data |
| trailing `?` | inferred fact |
| `+<n> more` | truncated remainder |

JSON Lines (`--json`): one object per line; absent fields omitted.

| field | type | description |
|-------|------|-------------|
| `t` | string | record type: `sym`, `file`, `edge`, `sec`, `note`, `more` |
| `path` | string | root-relative path |
| `line`, `end` | int | 1-based line range |
| `kind` | string | symbol kind or edge kind |
| `name` | string | qualified name |
| `sig` | string | signature |
| `doc` | string | doc sentence |
| `rel` | string | edge role: `in`, `out`, `sup`, `sub`, `test`, `env`, `run`, `chg` |
| `conf` | string | `exact` or `inferred` |
| `score` | float | ranking score (`find`, `ctx`) |
| `n` | int | count (`sec`, `more`) |

## 9. Index store

Location: `<index-dir>/<root basename>-<XXH3-64 hex of the canonical root path>/`.

| file | content | rewritten when |
|------|---------|----------------|
| `meta` | format version, extraction version, binary version, canonical root, write time | a refresh has changes |
| `files` | per file: path, size, mtime_ns, content hash, role, language, shard | file set or stat records change |
| `facts/NN` (64 shards by path hash) | fact records of the shard's files, shard-local string table | a file of the shard changes |
| `derived` | interned strings, symbol table, CSR graph, ranks, modules, search index, test map, audit data | derived content changes |
| `git` | churn, co-change, hotspots keyed by HEAD | HEAD changes and git data is requested |
| `lock` | writer lock | never |

- Encoding: little-endian fixed-size records; strings as (offset, length) into string tables; 64-byte aligned sections; header with magic, format version and XXH3-64 checksum.
- Writes: temp file in the same directory -> rename; no fsync; unchanged files untouched. A checksum mismatch or truncation on open rebuilds that file.
- Reads: memory-mapped, zero-copy.

## 10. Skill, install, update

| item | value |
|------|-------|
| source | `skills/xcalygraph/SKILL.md`, embedded in the binary |
| content | purpose; when to use it and when `rg` or direct reads are better; command table; output markers; exit codes; rules (verify against source, `--read-only` in read-only tasks, budget) |
| size | max 1500 tokens |
| install target | `~/.agents/skills/xcalygraph/SKILL.md` plus `.version` stamp, written atomically |
| `--link claude` | creates the symlink `~/.claude/skills/xcalygraph` -> install target if absent |
| update source | GitHub Releases of the project repository; assets `xcalygraph-<version>-<os>-<arch>.tar.gz` and `SHA256SUMS` |
| update flow | latest release metadata -> download asset -> verify SHA-256 -> atomic replace of the running binary -> `install` with the new binary |
| version mismatch | binary vs installed skill, one line from `version`, `install`, `update`; query commands stay silent |
| platforms | darwin-arm64, darwin-amd64, linux-arm64, linux-amd64 |
| binary location | `~/.local/bin/xcalygraph` |

Environment: `XCALYGRAPH_INDEX_DIR` sets the index base; `HOME` determines install targets.

## 11. Performance targets

Reference: Apple Silicon laptop, warm filesystem cache, corpus of 8,000 tracked files.

| metric | target |
|--------|--------|
| full index build | max 3 s |
| freshness check without changes | max 80 ms |
| query compute (`find`, `show`) | max 10 ms |
| refresh after a single-file change | max 150 ms |
| index size | max 20 MB |
| bytes written per single-file change | max 5 MB |
| peak RSS, full build | max 400 MB |
| skill size | max 1500 tokens |

## 12. Evaluation

Reference corpora: xcalygraph itself, the upstream Graphify clone at the pinned commit (Python), private corpora evaluated locally.

| metric | definition |
|--------|------------|
| hit@5 | share of questions whose expected symbol or file appears in the first 5 results of `find` or `ctx` |
| tokens per answer | output tokens of the command that answers the question |
| rg baseline | tokens of `rg` output plus the file reads an agent needs to reach the expected location |
| upstream baseline | hit@5 and tokens of `graphify query` on the same corpus |
| edge precision | share of emitted edges confirmed by source inspection, 50 samples per edge kind |

Question sets: `scripts/benchmarks/questions/<corpus>.tsv` with columns question, expected location (`path:line` or qualified name); sets for private corpora under `scripts/benchmarks/questions/private/` (gitignored).

## 13. Deferred features - See docs/deferred.md
