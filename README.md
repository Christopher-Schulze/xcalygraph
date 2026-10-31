# xcalygraph

**A code graph built for AI agents, not for humans.** One static binary turns a repository into a dense, verifiable knowledge graph: symbols, calls, type uses, imports, implementations, tests, env reads, plus precomputed analyses (impact, dead code, cycles, stale docs, hidden coupling). Every answer is a few hundred tokens of `path:line` facts that the agent can check, instead of thousands of tokens of grep output and whole-file reads.

- **Agent-first output**: line-oriented, no decoration, stable vocabulary, token budgets on every query, the most relevant facts first.
- **Verifiable**: every fact carries file and line, and inferred edges are marked `?`.
- **Fast and incremental**: parallel tree-sitter extraction, a zero-copy memory-mapped index, content hashes per file. Only changed files are re-extracted and only changed shards are rewritten.
- **Deterministic**: the same tree always gives byte-identical output.
- **Self-contained**: one binary with the agent skill embedded. No LLM calls, no interpreter, no service, no network except `update`.

Clean-room Rust rewrite of a selected subset of [Graphify](https://github.com/Graphify-Labs/graphify), redesigned around one question: how much useful, verifiable context does the agent get per token?

## Numbers

Measured on public corpora on an Apple Silicon laptop with a warm cache. Reproduce them with `scripts/benchmarks/run-benchmarks.sh` and `scripts/benchmarks/run-evaluation.sh`.

**Retrieval** compares hit@5 and tokens per answer for natural-language questions (`scripts/benchmarks/questions/`):

| corpus | questions | xcalygraph `find` | Graphify 0.9.97 `query` | `rg` + reading the file |
|---|---|---|---|---|
| xcalygraph (Rust) | 32 | **81 %, 583 tokens** | 75 %, 2796 tokens | 10240 tokens |
| Graphify clone (Python) | 26 | **92 %, 589 tokens** | 88 %, 2254 tokens | 175310 tokens |

`impact` finds 100 % of the expected dependents and tests on the xcalygraph impact set at 292 tokens per answer.

**Index build** compares xcalygraph against Graphify 0.9.97 on the same tracked files (`graphify update .`):

| corpus | files | full build | peak RSS | index on disk | refresh after a 1-file change |
|---|---|---|---|---|---|
| Graphify clone | 1111 | **0.91 s** vs 33.4 s | **87 MB** vs 684 MB | **7.5 MB** vs 61 MB | 96 ms |
| [hono](https://github.com/honojs/hono) @17ad0e2 | 577 | **0.48 s** vs 5.0 s | **43 MB** vs 211 MB | **1.4 MB** vs 7.6 MB | 69 ms |

Freshness check without changes: 6 to 8 ms. Every query runs it first, so answers never come from a stale index.

## What an agent sees

`xcalygraph show refresh::refresh` returns a symbol card with its signature, callers (`<`, at the call site), callees (`>`, at the definition) and tests (`t`):

```
src/app/refresh.rs:40-103 fn app::refresh::refresh
pub fn refresh(workspace: &Workspace, mode: Mode) -> Result<Report, AppError>
< app::run src/app/mod.rs:45
< app::index src/app/mod.rs:215
> infra::lock::WriterLock::acquire src/infra/lock.rs:20
> app::refresh::write_derived src/app/refresh.rs:137
> app::refresh::write_shards src/app/refresh.rs:180
t app::refresh::tests::first_refresh_builds_then_quiet_refresh_writes_nothing src/app/refresh.rs:285
t app::refresh::tests::corrupt_shard_is_rebuilt_from_its_files src/app/refresh.rs:362
+9 more
```

`xcalygraph impact derived::build` returns the dependents by depth, the tests reaching them and the exact command to run:

```
~ infra::store::derived::build src/infra/store/derived.rs:209
< 1 app::refresh::write_derived src/app/refresh.rs:137
< 2 app::refresh::refresh src/app/refresh.rs:40
< 3 app::run src/app/mod.rs:16
t resolve::tests::derived_bytes_are_identical_across_thread_counts src/resolve/tests.rs:155
r cargo test -p xcalygraph -- corrupt_shard_is_rebuilt_from_its_files derived_bytes_are_identical_across_thread_counts ...
```

`xcalygraph impact --diff` does the same for the uncommitted changes in the work tree (or against any revision), and flags exported signature changes.

`xcalygraph find "where is the index refreshed after a change"` returns ranked hits grouped by file:

```
src/app/refresh.rs
  180 fn write_shards(workspace: &Workspace, changes: &ChangeSet, rebuilt: bool) -> Result<u64, AppError>
  40 pub fn refresh(workspace: &Workspace, mode: Mode) -> Result<Report, AppError>
src/domain/corpus.rs
  76 pub struct ChangeSet
```

## Commands

| command | answers |
|---|---|
| `map` | How is this repository laid out? Languages, largest modules with their top symbols, entrypoints, hubs, cycles, layers, external dependencies, env vars. |
| `find TERMS...` | Where is X? Ranked symbols and files (BM25F over names, parents, paths, signatures, docs, combined with graph rank). |
| `ctx TERMS...` | What do I need to read for X? The best seeds with their doc sentence plus related symbols by personalized PageRank. |
| `show NAME` | What is X? Signature, doc, members, fields, supertypes, implementations, callers, callees, tests, env reads. |
| `impact NAME... \| --diff [REV]` | What does this change affect? Dependents, tests reaching them, test commands to run. |
| `path FROM TO` | How does A reach B? The shortest dependency path. |
| `audit` | What should be cleaned up? Dead code, unused exports, cycles, stale doc references, env drift, files changed together without a static link, hotspots. |
| `index`, `install`, `update`, `version`, `help` | Index control, skill installation, self-update, versions, syntax. |

Shared flags: `-p GLOB` (paths), `-l go,rust` (languages), `-k fn,struct` (kinds), `--no-tests`, `--exact` (drop inferred edges), `-b N` (token budget), `--json` (JSON Lines), `--read-only` (never write; exit 5 when stale), `-C DIR` (root).

NAME accepts `pkg.Type.Method`, `crate::mod::fn`, a qualified suffix (`Store.Get`), `path:line`, `path::name`, a file path, or a unique bare name. An ambiguous NAME exits with code 3 and prints the candidates.

## Languages

| language | depth |
|---|---|
| Go | packages, `go.mod` and `go.work` modules, receivers, interfaces satisfied by method sets, build-tag variants |
| Rust | module tree, workspace crates, `use` paths and re-exports, traits and impls, `cfg` variants |
| Python | dotted modules and packages, `src/` layout, absolute and relative imports, aliases, docstrings, env reads |
| TypeScript, JavaScript, JSX, TSX | `tsconfig` paths, `package.json` exports and workspaces, CommonJS, re-exports |
| Vue, Svelte, Astro | script blocks and template component usage |
| Java | packages, single-type, wildcard and static imports, implicit `this`, overloads by argument count, JUnit tests, Gradle and Maven test commands, framework entrypoints |
| Kotlin | as Java, plus extension functions narrowed by receiver type, objects and companions, trailing lambdas |
| Bash | functions, commands as calls, sourced files, env reads |
| Markdown | headings, links and backticked mentions resolved to files and symbols (the basis of stale doc detection) |

Next on the roadmap: Scala, Groovy, C# and Visual Basic .NET, the C family and Swift, config and infrastructure files (env definitions, task runners, SQL), Ruby, PHP, Lua, Zig, Elixir, Erlang and Dart, then documents and assets.

## Install

Build from source with the pinned stable toolchain (`rust-toolchain.toml`):

```sh
git clone https://github.com/Christopher-Schulze/xcalygraph
cd xcalygraph
cargo build --release
install -m 755 target/release/xcalygraph ~/.local/bin/xcalygraph
xcalygraph install --link claude
```

`install` writes the embedded skill to `~/.agents/skills/xcalygraph/`. `--link claude` links it into `~/.claude/skills/` for Claude Code, and any other agent host can read the same directory. Once GitHub Releases are published, `scripts/utils/install.sh` installs a verified prebuilt binary, and `xcalygraph update` replaces the binary and refreshes the skill in one step.

## How it works

```
walk (gitignore aware, parallel) -> stat diff -> hash candidates -> extract changed files (tree-sitter, parallel)
  -> rewrite affected fact shards -> resolve all facts to one symbol graph -> rank, modules, cycles, layers, search index
  -> derived file (fixed-size records, memory-mapped, read zero-copy)
```

- **Extraction** runs one generic tree-sitter query engine. Each language is a query file plus small hooks.
- **Resolution** follows each language's module rules: lexical scope, imports, the same module, receiver typing through bindings, call results and field chains, then interface and trait dispatch. A name that is only inferred keeps the `?` mark, and an ambiguous name stays ambiguous instead of being guessed.
- **Storage**:
  - 64 deflated fact shards keyed by path hash;
  - one checksummed `derived` file with string tables, CSR adjacency, ranks, modules and delta-coded search postings;
  - a git history cache keyed by HEAD.

  Every write goes to a temp file and is then renamed into place, and unchanged files are never rewritten.
- **Correctness gates**:
  - golden fact and edge fixtures per language;
  - byte-identical output across thread counts;
  - edge precision sampled against source;
  - a retrieval evaluation against Graphify and `rg`.

## Status

Pre-release (0.1.0). The commands, output format and index format may still change until the first tagged release.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
