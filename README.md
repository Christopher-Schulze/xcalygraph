# xcalygraph

**The code graph built for AI agents.**

One static binary turns any repository into a precise, verifiable map of symbols, calls, imports, implementations and tests, and answers an agent's structural questions in a few hundred tokens. No LLM, no service, no runtime dependencies.

[![CI](https://github.com/Christopher-Schulze/xcalygraph/actions/workflows/ci.yml/badge.svg)](https://github.com/Christopher-Schulze/xcalygraph/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Christopher-Schulze/xcalygraph?color=2563eb)](https://github.com/Christopher-Schulze/xcalygraph/releases)
[![Rust](https://img.shields.io/badge/Rust-stable-B7410E?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Languages](https://img.shields.io/badge/languages-60-111827)](#languages)

[Install](#install) · [Why](#why) · [Commands](#commands) · [Benchmarks](#benchmarks) · [Languages](#languages) · [How it works](#how-it-works) · [Documentation](docs/documentation.md) · [Releases](https://github.com/Christopher-Schulze/xcalygraph/releases)

Coding agents spend most of their tokens finding things: grepping, opening whole files, and following calls by hand. xcalygraph does that work up front and hands the agent the facts it needs, each with `path:line` so it can verify instead of trust.

## Install

**macOS & Linux**

```bash
curl -fsSL https://github.com/Christopher-Schulze/xcalygraph/releases/latest/download/install.sh | bash
```

**Build from source**

```bash
git clone https://github.com/Christopher-Schulze/xcalygraph && cd xcalygraph
cargo build --release && target/release/xcalygraph install
```

Both install the binary to `~/.local/bin`, the agent skill to `~/.agents/skills/xcalygraph/`, and link it for Claude Code when `~/.claude` exists. Your agent then uses xcalygraph on its own; the first query builds the index. Update with `xcalygraph update`. Pinned versions, uninstalling and the installer's checks: [documentation](docs/documentation.md#installation-and-releases).

## Why

- **Built for agents**: dense, line-oriented output with a stable vocabulary, a token budget on every query, and the most relevant facts first. No decoration, no prose.
- **Verifiable**: every fact carries file and line. Inferred edges are marked `?`, and ambiguous names stay ambiguous instead of being guessed.
- **Precomputed analyses**: change impact with the exact test command to run, dead code, unused exports, cycles, layers, stale doc references, env drift and hidden coupling from git history.
- **Always fresh**: every query checks for changes first (about 30 ms on a 7,500-file repository when nothing changed) and re-extracts only the files that did.
- **Fast and lean**: parallel tree-sitter extraction, a zero-copy memory-mapped index, content hashes per file, atomic shard rewrites.
- **Deterministic**: the same tree always gives byte-identical output.
- **Self-contained**: one binary with the agent skill embedded. No LLM calls, no interpreter, no service, no network except `update`.

## Try it

```sh
xcalygraph map                      # layout, entrypoints, tasks, hubs
xcalygraph find "where are requests authenticated"
xcalygraph impact --diff            # what your uncommitted changes affect
```

## Commands

| command | answers |
|---|---|
| `map` | How is this repository laid out and run? Languages, largest modules with their top symbols, entrypoints, tasks (Makefile, scripts, CI jobs, services) with what they run, hubs, API operations and framework routes with their handlers, cycles, layers, external dependencies, env vars. |
| `find TERMS...` | Where is X? Ranked symbols and files (BM25F over names, parents, paths, signatures, docs and the words of each body, combined with graph rank). |
| `show NAME` | What is X? Signature, doc, members, fields, supertypes, implementations, callers, callees, tests, env reads. |
| `impact NAME... \| --diff [REV]` | What does this change affect? Dependents, tests reaching them, test commands to run. |
| `path FROM TO` | How does A reach B? The shortest dependency path. |
| `audit` | What should be cleaned up? Dead code, unused exports, cycles, stale doc references, env drift, unreferenced assets, files changed together without a static link, hotspots. |
| `index`, `ls`, `clean`, `install`, `update`, `version`, `help` | Index control and housekeeping, installation, self-update, versions, syntax. |

Every query takes a token budget (`-b N`), path, language and kind filters (`-p`, `-l`, `-k`) and `--exact` to drop inferred facts. `xcalygraph help` lists all flags; NAME accepts qualified names, `path:line`, file paths or a unique bare name.

## Benchmarks

Real agent runs: headless Codex (`gpt-6-luna`, reasoning `max`) answering 90 questions on three code bases in isolated copies. Ranges span two runs with one answer per question and arm, so differences of a few points are within noise.

| arm | correct | total tokens per answer | time per answer |
|---|---|---|---|
| **xcalygraph** | **83 to 87 %** | **70,000 to 78,000** | 16 to 21 s |
| `rg` and reading files | 85 % | 71,000 to 72,000 | 14 to 18 s |
| Graphify | 87 to 89 % | 119,000 to 150,000 | 22 to 48 s |

So far xcalygraph answers these "where is X" questions about as well and as cheaply as plain search, at roughly half of Graphify's cost. On the largest of the three code bases it used about 40 % fewer tokens than `rg`; on the two smaller ones it cost the same or more. Questions about callers, impact and change planning, where search is weakest, are not part of this set yet.

Building and updating the index, same files for both tools:

| corpus | tool | full build | peak memory | index on disk | update after one edit |
|---|---|---|---|---|---|
| [hono](https://github.com/honojs/hono) (TypeScript, 577 files) | xcalygraph | **0.32 s** | **51 MB** | **1.44 MiB** | **94 ms** |
| | Graphify | 4.55 s | 202 MB | 7.46 MiB | 3.85 s |
| [Graphify](https://github.com/Graphify-Labs/graphify) source (Python, 1,111 files) | xcalygraph | **0.56 s** | **102 MB** | **7.20 MiB** | **145 ms** |
| | Graphify | 18.70 s | 918 MB | 59.66 MiB | 12.65 s |

Fact precision is measured against compiler indexes over about 60,000 edges in ten corpora: exact facts 96 to 100 %, inferred 98 to 100 % (one small exception is documented). On a frozen suite of 240 source-verified questions in eight corpora, `find` returns every required definition for 91 % of the questions at 600 tokens (Graphify at 2,000 tokens: 56 %) and no false hits on absent symbols (Graphify: 12 of 24). Method, limitations and reproduction: [documentation](docs/documentation.md#measured-results).

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
| Bash | functions, commands as calls, sourced files, env reads, literal `trap` handlers and `getopts` short options with argument and `case` facts |
| OpenAPI, Protobuf, GraphQL | operations, rpcs and fields matched to the code handlers that implement them |
| Terraform | resources, data sources, variables, locals, outputs and modules by address, with references and local module imports |
| SQL | tables, views and functions from DDL; `queries` edges from code that names them in SQL strings |
| Config and env | key paths of JSON, YAML, TOML, XML, INI and properties files; env definitions from `.env.example`, Dockerfiles, compose, GitHub Actions and Kubernetes against code reads (values never stored) |
| Markdown, reStructuredText, Org, AsciiDoc, LaTeX, HTML, text | headings, links, includes and inline-code mentions resolved to files and symbols (the basis of stale doc detection) |
| PDF, DOCX, XLSX | outlines, heading styles and sheets as headings; text layer, paragraphs and cell strings searchable through `find` |
| Images, SVG, audio, video | dimensions and durations from headers, SVG ids; literal paths in code, templates and config become `references`, so `audit` lists unreferenced assets |

</details>

## How it works

```
walk (gitignore aware, parallel) -> stat diff -> hash candidates -> extract changed files (tree-sitter, parallel)
  -> rewrite affected fact shards -> resolve all facts to one symbol graph -> rank, modules, cycles, layers, search index
  -> derived file (fixed-size records, memory-mapped, read zero-copy)
```

- **Extraction** runs one generic tree-sitter query engine. Each language is a query file plus small hooks.
- **Resolution** follows each language's module rules: lexical scope, imports, the same module, receiver typing through bindings, call results and field chains, then interface and trait dispatch.
- **Storage**: indexes are disposable and live in the system temp directory, never in your repository; `xcalygraph ls` lists them, `xcalygraph clean` removes them, and a lost index costs one full build. Writes are atomic and unchanged files are never rewritten.
- **Correctness gates**: golden fact and edge fixtures per language, byte-identical output across thread counts, edge precision and recall against compiler indexes, and a retrieval evaluation against Graphify.

xcalygraph is a clean-room Rust rewrite of a selected subset of [Graphify](https://github.com/Graphify-Labs/graphify), redesigned around one question: how much useful, verifiable context does the agent get per token?

## Status

Early development: `main` is heading for 0.2.0, the latest release is 0.1.1. Commands, output format and index format may still change between minor versions; `update` keeps binary and skill in step, and indexes rebuild themselves when their format changes.

## License

[MIT](LICENSE) Copyright (c) 2026 Christopher Schulze.
