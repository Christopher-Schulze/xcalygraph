# xcalygraph

**The code graph built for AI agents.** One static binary turns any repository into a precise, verifiable map of symbols, calls, imports, implementations and tests, and answers an agent's structural questions in a few hundred tokens.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Rust](https://img.shields.io/badge/rust-stable-orange.svg)
![Languages](https://img.shields.io/badge/languages-~60-green.svg)
![Release](https://img.shields.io/github/v/release/Christopher-Schulze/xcalygraph)

[Install](#install) · [What an agent sees](#what-an-agent-sees) · [Commands](#commands) · [Benchmarks](#benchmarks) · [Languages](#languages) · [Install and update](#install-and-update) · [How it works](#how-it-works) · [Documentation](docs/documentation.md) · [Releases](https://github.com/Christopher-Schulze/xcalygraph/releases)

Coding agents spend most of their tokens finding things: grepping, opening whole files, and following calls by hand. xcalygraph does that work up front and hands the agent exactly the facts it needs, each with `path:line` so it can verify instead of trust.

| | xcalygraph | Graphify | `rg` + reading files |
|---|---|---|---|
| tokens per answer | **~590** | ~2,200 to 2,800 | 16,000 to 175,000 |
| full index build | **0.4 to 1.2 s** | 3 to 29 s | n/a |
| update after editing one file | **50 to 110 ms** | 2 to 13 s | n/a |

<sub>Measured on three public corpora; details and method in [Benchmarks](#benchmarks).</sub>

## Install

**macOS and Linux**

```bash
curl -fsSL https://github.com/Christopher-Schulze/xcalygraph/releases/latest/download/install.sh | bash
```

**Build from source**

```bash
git clone https://github.com/Christopher-Schulze/xcalygraph && cd xcalygraph
cargo build --release && target/release/xcalygraph install
```

Both install the binary to `~/.local/bin`, the agent skill to `~/.agents/skills/xcalygraph/`, and link it for Claude Code when `~/.claude` exists. Your agent then uses xcalygraph on its own; the first query builds the index. Details: [Install and update](#install-and-update).

## Why

- **Built for agents**: dense, line-oriented output with a stable vocabulary, a token budget on every query, and the most relevant facts first. No decoration, no prose.
- **Verifiable**: every fact carries file and line. Inferred edges are marked `?`, and ambiguous names stay ambiguous instead of being guessed.
- **Precomputed analyses**: change impact with the exact test command to run, dead code, unused exports, cycles, layers, stale doc references, env drift and hidden coupling from git history.
- **Always fresh**: every query checks for changes first (about 8 ms when nothing changed) and re-extracts only the files that did.
- **Fast and lean**: parallel tree-sitter extraction, a zero-copy memory-mapped index, content hashes per file, atomic shard rewrites.
- **Deterministic**: the same tree always gives byte-identical output.
- **Self-contained**: one binary with the agent skill embedded. No LLM calls, no interpreter, no service, no network except `update`.

## Try it

```sh
xcalygraph map                      # layout, entrypoints, tasks, hubs
xcalygraph find "where are requests authenticated"
xcalygraph impact --diff            # what your uncommitted changes affect
```

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
t app::refresh::tests::first_refresh_builds_then_quiet_refresh_writes_nothing src/app/refresh.rs:282
t app::refresh::tests::changes_rewrite_only_affected_shards_and_read_only_sees_staleness src/app/refresh.rs:311
+9 more
```

`xcalygraph impact derived::build` returns the dependents by depth, the tests reaching them and the exact command to run:

```
~ infra::store::derived::build src/infra/store/derived.rs:209
< 1 app::refresh::write_derived src/app/refresh.rs:137
< 2 app::refresh::refresh src/app/refresh.rs:40
< 2 app::refresh::rederive src/app/refresh.rs:126
< 3 app::run src/app/mod.rs:16
t app::refresh::tests::first_refresh_builds_then_quiet_refresh_writes_nothing src/app/refresh.rs:282
t app::refresh::tests::changes_rewrite_only_affected_shards_and_read_only_sees_staleness src/app/refresh.rs:311
r cargo test -p xcalygraph -- changes_rewrite_only_affected_shards_and_read_only_sees_staleness corrupt_file_table_triggers_full_rebuild ...
```

`xcalygraph impact --diff` does the same for the uncommitted changes in the work tree (or against any revision) and flags exported signature changes.

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
| `map` | How is this repository laid out and run? Languages, largest modules with their top symbols, entrypoints, tasks (Makefile, scripts, CI jobs, services) with what they run, hubs, API operations and framework routes with their handlers, cycles, layers, external dependencies, env vars. |
| `find TERMS...` | Where is X? Ranked symbols and files (BM25F over names, parents, paths, signatures, docs and the words of each body, combined with graph rank). |
| `ctx TERMS...` | What do I need to read for X? The best seeds with their doc sentence plus related symbols by personalized PageRank. |
| `show NAME` | What is X? Signature, doc, members, fields, supertypes, implementations, callers, callees, tests, env reads. |
| `impact NAME... \| --diff [REV]` | What does this change affect? Dependents, tests reaching them, test commands to run. |
| `path FROM TO` | How does A reach B? The shortest dependency path. |
| `audit` | What should be cleaned up? Dead code, unused exports, cycles, stale doc references, env drift, unreferenced assets, files changed together without a static link, hotspots. |
| `index`, `ls`, `clean`, `install`, `update`, `version`, `help` | Index control and housekeeping, installation, self-update, versions, syntax. |

Shared flags: `-p GLOB` (paths), `-l go,rust` (languages), `-k fn,struct` (kinds), `--no-tests`, `--exact` (drop inferred edges), `-b N` (token budget), `--json` (JSON Lines), `--read-only` (never write; exit 5 when stale), `-C DIR` (root).

NAME accepts `pkg.Type.Method`, `crate::mod::fn`, a qualified suffix (`Store.Get`), `path:line`, `path::name`, a file path or a unique bare name. An ambiguous NAME exits with code 3 and prints the candidates.

## Benchmarks

All measurements use the same three public corpora:

| corpus | language | files | why |
|---|---|---|---|
| [hono](https://github.com/honojs/hono) @17ad0e2 | TypeScript | 577 | home ground for neither tool |
| [Graphify](https://github.com/Graphify-Labs/graphify) source | Python | 1,111 | Graphify's home ground |
| xcalygraph source | Rust | 481 | xcalygraph's home ground |

Apple Silicon laptop, warm cache, Graphify 0.9.97. Reproduce with `scripts/benchmarks/run-evaluation.sh` and `scripts/benchmarks/run-benchmarks.sh --upstream`.

### Finding code from a question

Each question describes code in plain words ("compare two secrets in constant time"), never by its name. A hit means the file with the expected definition is among the first five results. Tokens are what the agent reads per answer. The `rg` baseline greps the two longest question words; its hit means the expected file shows up, and its tokens include reading that file.

| corpus | questions | xcalygraph `find` | Graphify `query` | `rg` + read |
|---|---|---|---|---|
| hono | 32 | 81 %, 589 tokens | 75 %, 2,200 tokens | 75 %, 33,693 tokens |
| Graphify source | 26 | 100 %, 581 tokens | 88 %, 2,254 tokens | 100 %, 175,310 tokens |
| xcalygraph source | 32 | 90 %, 589 tokens | 75 %, 2,796 tokens | 100 %, 18,507 tokens |

At a similar or higher hit rate, xcalygraph answers with about a quarter of Graphify's tokens and 1/30 to 1/300 of what `rg` plus reading costs. A Graphify hit counts anywhere in its answer, an xcalygraph hit only in the first five results; counted anywhere in its 600-token answer, xcalygraph reaches 87 %, 100 % and 96 %. Held-out questions written before the latest tuning round (`run-evaluation.sh --set heldout2`) land at 90 %, 90 % and 90 % (Graphify 90 %, 70 %, 20 %). `impact` finds 100 % of the expected dependents and tests on the xcalygraph impact set, at 335 tokens per answer.

### Building and updating the index

Same files for both tools; Graphify runs `graphify update .`, and its numbers are the faster of two runs.

| corpus | tool | full build | peak memory | index on disk | update after editing one file |
|---|---|---|---|---|---|
| hono | xcalygraph | **0.48 s** | **44 MB** | **1.4 MB** | **73 ms** |
| | Graphify | 5.6 s | 192 MB | 7.5 MB | 4.5 s |
| Graphify source | xcalygraph | **1.2 s** | **90 MB** | **7.4 MB** | **110 ms** |
| | Graphify | 29 s | 718 MB | 60 MB | 13 s |
| xcalygraph source | xcalygraph | **0.36 s** | **29 MB** | **1.0 MB** | **49 ms** |
| | Graphify | 2.9 s | 99 MB | 8.4 MB | 1.9 s |

Compared with Graphify, xcalygraph builds 8 to 24 times faster, needs 3 to 8 times less memory and disk, and updates after an edit 40 to 120 times faster.

### Language depth

`scripts/benchmarks/compare-depth.sh` checks, for each language corpus, which of Graphify's relations between files of the corpus xcalygraph also finds. Across the 26 public corpora with at least 50 such calls (pinned in `scripts/benchmarks/corpora.tsv`), the median match is 95 % for calls (lowest 74 %) and 97 % for inheritance and implementation. Spot checks of the misses against the source show they are mostly Graphify errors (library methods bound to same-named local functions), recursion (xcalygraph drops self-loops) or generated files left out on purpose.

## Languages

About 60 languages and formats, each resolved by its own module rules rather than by name matching alone.

<details>
<summary>Depth per language</summary>

| language | depth |
|---|---|
| Go | packages, `go.mod` and `go.work` modules, receivers, interfaces satisfied by method sets, build-tag variants |
| Rust | module tree, workspace crates, `use` paths and re-exports, traits and impls, `cfg` variants |
| Python | dotted modules and packages, `src/` layout, absolute and relative imports, aliases, docstrings, env reads |
| TypeScript, JavaScript, JSX, TSX | `tsconfig` paths, `package.json` exports and workspaces, CommonJS, re-exports, React components, hooks and context providers |
| Next.js, SvelteKit | file-based routes (`page /products/[id]`, `GET /api/posts`, `load /login`), middleware, hooks and server actions, each calling its handler, so `impact` of a component lists the pages rendering it |
| Vue, Svelte, Astro | script blocks and template component usage; Svelte markup expressions, directives and `$store` reads; SvelteKit `$lib` |
| Java | packages, single-type, wildcard and static imports, implicit `this`, overloads by argument count, JUnit tests, Gradle and Maven test commands, framework entrypoints |
| Kotlin | as Java, plus extension functions narrowed by receiver type, objects and companions, trailing lambdas |
| Scala, Groovy | packages and imports as Java; Scala objects, traits and package-qualified calls; Groovy scripts and Spock specifications |
| C#, Visual Basic .NET | namespaces, `using`/`Imports` (aliases, global usings), partial types, extension methods, overloads with optional parameters, xUnit/NUnit/MSTest tests, `dotnet test` filters, framework entrypoints |
| C, C++, CUDA, Metal | include resolution with CMake and `compile_commands.json` include directories, declarations mapped to definitions, namespaces, out-of-line members, virtual dispatch, function pointers and macro bodies |
| Objective-C, Swift | classes, protocols, messages and selectors; SwiftPM modules, extensions, XCTest and `@Test`, `swift test` filters |
| Ruby, PHP | Ruby constants, modules and `require`, RSpec and Minitest commands; PHP namespaces, `use`, traits, typed property chains, PHPUnit filters |
| Lua, Zig | `require` bindings and `.toc` manifests; Zig `@import` bindings, `build.zig` named modules, `test` blocks |
| Elixir, Erlang | modules by full name, `alias`/`import`/`use`, remote and piped calls, multi-clause functions, OTP callbacks; `m:f()` remote calls, `-include`, `mix test` paths |
| Dart | `package:` imports, `export` and `part` libraries, constructor and call-result typing, `dart test` paths |
| Julia, R, OCaml | Julia modules, `using`/`include`; R packages from `DESCRIPTION`, `pkg::f`, testthat namespaces; OCaml file modules, `.mli` declarations, `open` |
| Fortran, Pascal and Delphi, Common Lisp | Fortran modules and `use`; Pascal units, class methods, DFM/LFM form event handlers; Lisp packages, `defpackage` uses, ASDF systems, generic functions |
| Solidity, Verilog and SystemVerilog | Solidity contracts, remappings, modifiers, events, Foundry tests; Verilog module instantiations resolved across the design |
| PowerShell, Robot Framework, DM | PowerShell modules and dot-sourcing; Robot keywords resolved to resources and Python libraries; DM type paths with inheritance |
| Apex, Razor, Blade, EJS, Jinja, COBOL, XAML, DM resources | structural depth: classes, methods, triggers, components, views, template includes and blocks, COBOL paragraphs and copybooks, XAML handlers to code-behind |
| Bash | functions, commands as calls, sourced files, env reads |
| OpenAPI, Protobuf, GraphQL | operations, rpcs and fields matched to the code handlers that implement them |
| Terraform | resources, data sources, variables, locals, outputs and modules by address, with references and local module imports |
| SQL | tables, views and functions from DDL; `queries` edges from code that names them in SQL strings |
| Config and env | key paths of JSON, YAML, TOML, XML, INI and properties files; env definitions from `.env.example`, Dockerfiles, compose, GitHub Actions and Kubernetes against code reads (values never stored) |
| Markdown, reStructuredText, Org, AsciiDoc, LaTeX, HTML, text | headings, links, includes and inline-code mentions resolved to files and symbols (the basis of stale doc detection) |
| PDF, DOCX, XLSX | outlines, heading styles and sheets as headings; text layer, paragraphs and cell strings searchable through `find` |
| Images, SVG, audio, video | dimensions and durations from headers, SVG ids; literal paths in code, templates and config become `references`, so `audit` lists unreferenced assets |

</details>

## Install and update

### Release installer

The [one-liner](#install) fetches `install.sh` from the latest [release](https://github.com/Christopher-Schulze/xcalygraph/releases), never from `main`. The installer:

- picks the archive for your platform (macOS and Linux, arm64 and x86-64);
- verifies it against the release's `SHA256SUMS`;
- smoke-tests the new binary before it replaces an existing one;
- lets the binary install itself, the skill and the Claude Code link;
- reports a missing PATH entry with the exact `export` line instead of editing your shell profile.

```bash
# a specific version, or the binary without the skill
curl -fsSL https://github.com/Christopher-Schulze/xcalygraph/releases/latest/download/install.sh | bash -s -- --version 0.1.1
curl -fsSL https://github.com/Christopher-Schulze/xcalygraph/releases/latest/download/install.sh | bash -s -- --no-skill
```

`XCALYGRAPH_INSTALL_DIR` changes the target directory (default `~/.local/bin`).

### What `xcalygraph install` does

```bash
xcalygraph install               # binary, skill, Claude Code link
xcalygraph install --skill-only  # skill (and link) only
xcalygraph install --no-link     # no Claude Code link
```

- **Binary**: copies the running executable to `~/.local/bin/xcalygraph` unless it already runs from there.
- **Skill**: writes the embedded skill to `~/.agents/skills/xcalygraph/SKILL.md`, the shared location agent hosts read. The skill always matches the binary's version.
- **Claude Code**: when `~/.claude` exists, links `~/.claude/skills/xcalygraph` to that directory. A foreign file or link there is reported and never replaced.

### Update

```bash
xcalygraph update           # newest release, then a fresh skill
xcalygraph update --check   # only report whether one is available
```

`update` downloads the release for your platform, verifies the checksum, smoke-tests the new binary, replaces the old one atomically and installs the matching skill. When the binary is already current, it still rewrites a skill that is out of date. On a terminal every step shows live progress (download bar with speed and remaining time); in scripts and agent runs only the result lines appear.

### Uninstall

```bash
xcalygraph clean                # every index
rm ~/.local/bin/xcalygraph
rm -r ~/.agents/skills/xcalygraph ~/.claude/skills/xcalygraph
```

## How it works

```
walk (gitignore aware, parallel) -> stat diff -> hash candidates -> extract changed files (tree-sitter, parallel)
  -> rewrite affected fact shards -> resolve all facts to one symbol graph -> rank, modules, cycles, layers, search index
  -> derived file (fixed-size records, memory-mapped, read zero-copy)
```

- **Extraction** runs one generic tree-sitter query engine. Each language is a query file plus small hooks.
- **Resolution** follows each language's module rules: lexical scope, imports, the same module, receiver typing through bindings, call results and field chains, then interface and trait dispatch.
- **Storage**: indexes are disposable and live in the system temp directory, never in your repository (inside it only when an agent may not write elsewhere); `xcalygraph ls` lists them, `xcalygraph clean` removes them, and a lost index costs one full build (1.3 s for 8,000 files). Each index holds 64 deflated fact shards keyed by path hash; one checksummed `derived` file with string tables, CSR adjacency, ranks, modules and delta-coded search postings; a git history cache keyed by HEAD. Every write goes to a temp file that is then renamed into place, and unchanged files are never rewritten.
- **Correctness gates**: golden fact and edge fixtures per language, byte-identical output across thread counts, edge precision sampled against source, and a retrieval evaluation against Graphify and `rg`.

xcalygraph is a clean-room Rust rewrite of a selected subset of [Graphify](https://github.com/Graphify-Labs/graphify), redesigned around one question: how much useful, verifiable context does the agent get per token?

## Status

Early release (0.1.1). Commands, output format and index format may still change between minor versions; `update` keeps binary and skill in step, and indexes rebuild themselves when their format changes.

## License

[MIT](LICENSE)
