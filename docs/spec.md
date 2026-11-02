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
| `-b, --budget N` | per command | output budget in tokens; estimate = ceil(bytes / 3); stdout of every query, truncation line, ambiguity candidates and `impact` run commands included, is at most 3N bytes; tiny budgets use a compact or count-free truncation marker, including `{}` for the smallest JSON budgets |
| `--json` | off | JSON Lines instead of text (section 8) |
| `--read-only` | off | never create or refresh the index; exit 5 if missing or stale |
| `--index-dir PATH` | nonempty `$XCALYGRAPH_INDEX_DIR`, else `<system temp directory>/xcalygraph-<user>` (created mode at most 0700); on actual shared-index location failure, `<root>/.xcalygraph/` with a one-line notice | index base directory; explicit paths never fall back |
| `-p, --path GLOB` | whole root | keep results whose root-relative path matches GLOB (gitignore glob syntax); repeatable |
| `-l, --lang LIST` | all | comma-separated language ids (section 3) |
| `-k, --kind LIST` | all | comma-separated symbol kinds (section 5) |
| `-d, --depth N` | `show` 1, `impact` 3 | traversal depth of `show` callers and callees and of `impact` |
| `--exact` | off | drop inferred facts |
| `--no-tests` | off | drop `test`, `gen` and `fixture` roles |

| command | arguments | budget | result |
|---------|-----------|--------|--------|
| `map` | none | 2500 | orientation (7.1) |
| `find` | `TERMS...` | 600 | ranked symbols and files, one line each (7.2) |
| `ctx` | `TERMS...` | 2000 | context bundle: seeds plus related symbols with signatures (7.3) |
| `show` | `NAME [--all]` | 1200 | symbol card (7.4); `--all` lifts per-section caps |
| `impact` | `NAME...` or `--diff [REV]` | 1500 | changed and affected symbols, tests, run commands (7.5) |
| `path` | `FROM TO` | 400 | shortest dependency path (7.6) |
| `audit` | none | 2000 | dead code, cycles, stale doc references, env drift, unreferenced assets, hidden coupling, hotspots (7.7) |
| `index` | none | none | explicit build or refresh |
| `ls` | none | none | indexes in the index base: root, size, last use, `missing` when the root is gone, totals |
| `clean` | `[ROOT...] [--missing]` | none | delete verified indexes in the index base; with ROOTs only theirs (also their local index under `<root>/.xcalygraph/`); `--missing` only those whose root is gone; preserve and report unverified entries |
| `install` | `[--no-link] [--skill-only]` | none | install the running binary, the embedded skill and the Claude Code link (section 10) |
| `update` | `[--check]` | none | self-update the binary, then refresh a stale skill |
| `version` | none | none | binary, index format and installed skill versions |
| `help` | `[COMMAND]` | none | syntax reference |

- Query commands (`map`, `find`, `ctx`, `show`, `impact`, `path`, `audit`) refresh the index first (section 4) unless `--read-only`.
- `impact --diff` verifies each required fact shard against the indexed source hashes and exact file roster; writable queries re-derive damaged shards and retry, while `--read-only` returns exit 5 without writes.
- Filters (`-p`, `-l`, `-k`, `--exact`, `--no-tests`) apply to every query command and restrict seeds, expansions and listed facts alike: every NAME form (an excluded file or `path:line` is no match), `path` searches only the subgraph of included nodes, `impact` traverses the whole graph but lists only included dependents, tests and their run lines, and `map` module sections (cycles, layers, externals) show only modules with an included file. `map` and `audit` cycles are recomputed on that induced module graph so excluded intermediates cannot create a reported cycle. `-k` restricts symbols; a file NAME, file-level facts and import hops stay usable.
- NAME forms: `qualified.name`; `path:line`; `path::name`; bare name (must be unique). Resolution checks the input as a file path first (including the existing `./` alias), then its surrounding-whitespace-trimmed form before locator syntax; a matching file path wins a collision.
- Aliases: `-h`, `--help` = `help`; `<command> -h|--help` = `help <command>`; `-V`, `--version` = `version`.
- Usage errors: one stderr line `xcalygraph: <error>; run 'xcalygraph help'`, exit 2. A reader that closes stdout early is not an error.

| exit code | meaning |
|-----------|---------|
| 0 | answer |
| 1 | runtime error |
| 2 | usage error |
| 3 | ambiguous NAME; candidates listed |
| 4 | no match |
| 5 | index missing, stale, corrupt or written by another extraction, resolver or binary version under `--read-only` |

## 3. Corpus model

| role | detection | indexing |
|------|-----------|----------|
| `src` | file of a language in the language table | extraction per language depth |
| `test` | language test convention (language table) | as `src`; rank weight 0.3 |
| `gen` | first 1 KiB contains `Code generated` with `DO NOT EDIT`, or `@generated` | as `src`; rank weight 0.2; excluded from `map` and `audit` |
| `fixture` | path segment `testdata`, `fixtures`, `__fixtures__` or `__snapshots__` | path only |
| `manifest` | `go.mod`, `go.work`, `Cargo.toml`, `package.json`, `pnpm-workspace.yaml`, `pyproject.toml`, `requirements*.txt`, `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts`, `*.csproj`, `*.fsproj`, `*.vbproj`, `*.sln`, `*.slnx`, `Gemfile`, `*.gemspec`, `composer.json`, `Package.swift`, `pubspec.yaml`, `mix.exs`, `rebar.config`, `Project.toml`, `DESCRIPTION`, `dune-project`, `build.zig`, `build.zig.zon`, `foundry.toml`, `remappings.txt`, `CMakeLists.txt`, `compile_commands.json`, `tsconfig.json`, `jsconfig.json` | module metadata, dependencies, workspace members, source roots, include directories, entrypoints |
| `config` | `.json`, `.jsonc`, `.yaml`, `.yml`, `.toml`, `.xml`, `.ini`, `.properties`, `.proto`, `.graphql`, `.gql`; `Dockerfile`, `*.dockerfile`, `Makefile`, `*.mk`; `.env.example` | key paths and schema facts (section 5) |
| `doc` | `.md`, `.mdx`, `.qmd`, `.markdown`, `.skill`, `.txt`, `.rst`, `.org`, `.adoc`, `.tex`, `.html`, `.htm`, `.pdf`, `.docx`, `.xlsx` | headings, links, mentions, searchable text |
| `asset` | `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`, `.ico`, `.mp4`, `.mov`, `.webm`, `.mkv`, `.avi`, `.m4v`, `.mp3`, `.wav`, `.m4a`, `.ogg` | header metadata; SVG ids and text; incoming references |
| `sensitive` | `.env*` except `.env.example`; `*.pem`, `*.key`, `*.p12`, `id_rsa*`, `id_ed25519*`; anything under `.ssh/`, `.aws/`, `.gnupg/` | path only; content never read |
| `large` | size > 8 MiB; `.pdf`, `.docx`, `.xlsx` > 64 MiB | path only |
| `binary` | NUL byte in the first 8 KiB of a file without another role | path only |
| `other` | everything else | path only |

- Role precedence, first match wins: `sensitive`, `fixture`, `large`, `manifest`, `gen`, `test`, `src`, `config`, `doc`, `asset`, `binary`, `other`.
- Depth `deep`: grammar-based facts (section 5), module-aware import resolution and the full resolution order (section 6), test map, run lines where section 7.5 defines them. Depth `structural`: symbols, includes and calls from grammar or line patterns; imports by literal path; calls by unique name, inferred.

| language | id | extensions | depth | test convention |
|----------|----|------------|-------|-----------------|
| Go | `go` | `.go` | deep | `_test.go` files; `Test*`, `Benchmark*`, `Fuzz*`, `Example*` functions |
| Rust | `rust` | `.rs` | deep | `#[test]` functions, `#[cfg(test)]` modules, files under `tests/` |
| TypeScript | `typescript` | `.ts`, `.tsx`, `.mts`, `.cts`, `.ets` | deep | `*.test.*`, `*.spec.*`, `*.stories.*`, files under `__tests__/`, `__mocks__/`, `test/`, `tests/`, `e2e/`, `mocks/` |
| JavaScript | `javascript` | `.js`, `.jsx`, `.mjs`, `.cjs` | deep | as TypeScript |
| Vue, Svelte, Astro | `vue`, `svelte`, `astro` | `.vue`, `.svelte`, `.astro` | deep: actual script blocks outside HTML comments and styles as TypeScript or JavaScript; template component tags outside comments, scripts and styles as calls | as TypeScript |
| Python | `python` | `.py`, `.pyi` | deep | `test_*.py`, `*_test.py`, files under `tests/`; `test_*` functions, `Test*` classes |
| Java | `java` | `.java` | deep | files under `src/test/`; classes `*Test`, `*Tests`, `*IT`; `@Test` methods |
| Kotlin | `kotlin` | `.kt`, `.kts` | deep | as Java |
| Scala | `scala` | `.scala` | deep | as Java; classes `*Spec`, `*Suite` |
| Groovy | `groovy` | `.groovy`, `.gradle` | deep | as Java; Spock classes `*Spec` |
| C# | `csharp` | `.cs` | deep | `[Fact]`, `[Theory]`, `[Test]`, `[TestCase]`, `[TestMethod]` methods |
| Visual Basic .NET | `vbnet` | `.vb` | deep | as C# with `<...>` attributes |
| C | `c` | `.c`, `.h` | deep | files `*_test.*`, `test_*.*`, `*_unittest.*`, files under `test/`, `tests/` |
| C++ | `cpp` | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hh`, `.hxx`, `.h`, `.cu`, `.cuh`, `.metal` | deep | as C; GoogleTest `TEST`, `TEST_F`, `TEST_P`; Catch2 `TEST_CASE` |
| Objective-C | `objc` | `.m`, `.mm`, `.h` | deep | files `*Tests.m`, `*Tests.mm`; `XCTestCase` subclasses |
| Swift | `swift` | `.swift` | deep | files under `Tests/`, `*Tests.swift`; `XCTestCase` subclasses, `@Test` functions |
| Ruby | `ruby` | `.rb`, `.rake` | deep | `*_spec.rb`, `*_test.rb`, `test_*.rb`, files under `spec/`, `test/` |
| PHP | `php` | `.php` | deep | `*Test.php`, files under `tests/`; PHPUnit `test*` methods, Pest `test()`, `it()` |
| Lua | `lua` | `.lua`, `.luau`, `.toc` | deep | `*_spec.lua`, files under `spec/` |
| Zig | `zig` | `.zig` | deep | `test` blocks |
| Elixir | `elixir` | `.ex`, `.exs` | deep | `*_test.exs`, files under `test/` |
| Erlang | `erlang` | `.erl`, `.hrl`, `.escript` | deep | `*_SUITE.erl`, `*_tests.erl`, files under `test/` |
| Dart | `dart` | `.dart` | deep | `*_test.dart`, files under `test/` |
| Julia | `julia` | `.jl` | deep | files under `test/`; `@testset` blocks |
| R | `r` | `.r`, `.R` | deep | `tests/testthat/test-*.R` |
| OCaml | `ocaml` | `.ml`, `.mli` | deep | files under `test/`, `tests/` |
| Fortran | `fortran` | `.f`, `.F`, `.f90`, `.F90`, `.f95`, `.F95`, `.f03`, `.F03`, `.f08`, `.F08` | deep | files under `test/`, `tests/` |
| Pascal, Delphi | `pascal` | `.pas`, `.pp`, `.dpr`, `.dpk`, `.lpr`, `.inc`, `.dfm`, `.lfm`, `.lpk` | deep | `TTestCase` subclasses |
| Common Lisp | `lisp` | `.lisp`, `.cl`, `.lsp`, `.asd` | deep | files under `test/`, `tests/`, `t/` |
| Solidity | `solidity` | `.sol` | deep | `*.t.sol`, files under `test/` |
| Verilog, SystemVerilog | `verilog` | `.v`, `.sv`, `.svh`, `.vh` | deep | `*_tb.*`, `tb_*.*` |
| PowerShell | `powershell` | `.ps1`, `.psm1`, `.psd1` | deep | `*.Tests.ps1` |
| Bash | `bash` | `.sh`, `.bash`, `.bats`; extensionless files with `sh`, `bash` or `zsh` shebang | deep | `.bats` files |
| SQL | `sql` | `.sql` | deep | none |
| Terraform, HCL | `terraform` | `.tf`, `.tfvars`, `.hcl` | deep | `*.tftest.hcl` |
| Robot Framework | `robot` | `.robot`, `.resource` | deep | test cases of `.robot` files |
| DM | `dm` | `.dm`, `.dme` | deep | none |
| Apex | `apex` | `.cls`, `.trigger` | structural | `@isTest` classes and methods |
| Razor | `razor` | `.razor`, `.cshtml` | structural | none |
| Blade | `blade` | `.blade.php` | structural | none |
| EJS | `ejs` | `.ejs` | structural | none |
| Jinja | `jinja` | `.j2`, `.jinja`, `.jinja2` | structural | none |
| COBOL | `cobol` | `.cbl`, `.cob`, `.cobol`, `.cpy` | structural | none |
| XAML | `xaml` | `.xaml` | structural | none |
| DM resources | `dmres` | `.dmi`, `.dmm`, `.dmf` | structural | none |

- Detection: longest suffix wins (`.blade.php` before `.php`). Ambiguous suffixes are decided by content markers in the first 8 KiB, else role `other`: `.h` (Objective-C on `@interface` or `#import`, C++ on `class`, `namespace`, `template` or a C++ sibling with the same stem, else C; a sibling appearing, disappearing, being renamed or ignored reclassifies the unchanged header, so an update equals a fresh build); `.m` (Objective-C on `#import`, `@interface` or `@implementation`); `.v` (Verilog on `module` with `endmodule`); `.inc`, `.pp` (Pascal on `unit`, `procedure`, `function` or `begin`); `.cl` (Common Lisp on a leading `(`); `.cls` (Apex on `class` or `interface` with an access modifier).
- Embedded code: `<script>` blocks of HTML, Vue, Svelte and Astro are extracted in their language; Markdown fenced code contributes mentions only.
- Ignore rules, lowest to highest precedence: global git excludes (`$XDG_CONFIG_HOME/git/ignore`), `.git/info/exclude`, nested `.gitignore`, nested `.xcalygraphignore` (gitignore syntax).
- Always pruned: `.git`, `.hg`, `.svn`, `node_modules`, `__pycache__`, `graphify-out`, directories containing `CACHEDIR.TAG` or `pyvenv.cfg`, `vendor` next to `go.mod` when `vendor/modules.txt` exists.
- Hidden files are indexed; symlinks are not followed.

## 4. Pipeline

walk -> stat diff -> hash candidates -> extract changed files (parallel) -> write changed fact shards -> resolve all facts (parallel per file) -> derive graph, ranks, modules, search index -> write derived data if changed -> answer.

Freshness:

1. A file is unchanged when path, size, mtime_ns and a usable filesystem identity (device, inode, status-change time) equal its stored record and both timestamps are older than the stored index write time minus 2 s. Unknown identity never takes the stat-only shortcut.
2. Otherwise the XXH3-128 content hash decides for files whose content may be read; an equal hash updates only the stat record. Sensitive and oversized files remain path-only.
3. New, changed and deleted files form the change set; an empty change set answers from the existing index without writes.
4. A format or extraction version mismatch triggers a full rebuild.

- Locking: writers hold an exclusive advisory lock on an unshared regular `<index>/lock` file, waiting at most 30 s before exit 1; symlinks and hard-linked lock files are rejected; readers never lock and map the files current at open time.
- Parallelism: walk, hashing, extraction and per-file resolution run on a work-stealing pool sized to the available cores; one parser and one reusable arena per worker, reset per file.
- IO: files up to 1 MiB are read into a per-worker reusable buffer; larger files are memory-mapped; index files are memory-mapped for reading.

## 5. Extraction

- Deep languages: tree-sitter grammar plus embedded query files (definitions, references, imports); query captures map to fact records; language code only for semantics a query cannot express (module rules, section 6).
- Structural languages: grammar or line patterns for symbols, includes and calls.
- `config`, `doc`, `asset` files: one format parser per file type; PDF text layer only, no OCR; DOCX and XLSX from their XML parts; audio and video from container headers, no transcription.

| fact | fields |
|------|--------|
| symbol | kind, name, parent symbol, line start, line end, signature (one line, whitespace-normalized, body removed, max 160 chars), doc (first sentence of the attached doc comment, max 120 chars), visibility (exported or private), flags (test, entry, generated) |
| import | module specifier, alias, imported names, line |
| ref | name (identifier or last two segments of a member chain), kind (call, type, inherit, implement), enclosing symbol, line |
| env | variable name, enclosing symbol, line |
| fields | owning type, field names (card content only, no graph nodes) |
| mention | doc heading, backticked identifier or relative path, line |
| link | doc heading, target path, line |
| manifest | module name; dependency name and version requirement; workspace members; source roots; include directories (`compile_commands.json`, `CMakeLists.txt` `include_directories`); entrypoints (package.json `bin` and `scripts`, pyproject `[project.scripts]`, Cargo `[[bin]]`, `src/main.rs`, `src/bin/*.rs`, Gradle `application.mainClass`, `.csproj` `OutputType` `Exe`, SwiftPM executable targets, Dart `bin/`) |
| config | key path (dotted, max depth 6), value kind, line; values are not stored |
| schema | GitHub Actions jobs and steps (`run`, `uses`); compose services (image, ports, `environment` names, `depends_on`); Kubernetes objects (kind, name, images, env names); OpenAPI operations (method, path, `operationId`); Dockerfile stages (base image, `ENV`, `ARG`, `EXPOSE`, `ENTRYPOINT`, `CMD`); Makefile targets and prerequisites; Protobuf messages, enums, services, rpcs; GraphQL types, interfaces, fields; SQL tables, views, columns, functions, procedures, triggers |
| env def | variable name (never its value), source (`.env.example`, compose, Dockerfile, CI workflow, Kubernetes), line |
| text | text segment with heading path and page, sheet or line, for the search index only |
| asset | format; width and height, or duration |
| asset ref | literal path from code, doc, template or config to an existing file of the root, line |

- Symbol kinds: `mod`, `struct`, `class`, `enum`, `iface`, `trait`, `alias`, `fn`, `method`, `const`, `var`, `macro`, `heading`, `table`, `view`, `task` (Makefile target, package script, CI job), `svc` (compose service, Kubernetes workload), `op` (OpenAPI operation; framework route `<kind> <route>` from Next.js App and Pages Router, SvelteKit route files, Next.js middleware, SvelteKit hooks and `"use server"` actions, calling its handler). Protobuf and GraphQL map to `struct`, `enum`, `iface` (service) and `method` (rpc, field resolver).
- Never emitted: locals, parameters, generic type parameters, anonymous functions (their calls count for the enclosing symbol), fields as nodes, symbols without source location.
- Env reads: Go `os.Getenv`, `os.LookupEnv`; Rust `std::env::var`, `std::env::var_os`, `env!`, `option_env!`; TypeScript/JavaScript `process.env.X`, `process.env["X"]`, `import.meta.env.X`; Python `os.environ[...]`, `os.environ.get`, `os.getenv`; Java, Kotlin, Scala, Groovy `System.getenv`; C#, VB.NET `Environment.GetEnvironmentVariable`; C, C++, Objective-C `getenv`; Swift `ProcessInfo.processInfo.environment[...]`; Ruby `ENV[...]`, `ENV.fetch`; PHP `getenv`, `$_ENV[...]`; Lua `os.getenv`; Zig `std.process.getEnvVarOwned`, `std.posix.getenv`; Elixir `System.get_env`, `System.fetch_env`; Erlang `os:getenv`; Dart `Platform.environment[...]`; Julia `ENV[...]`; R `Sys.getenv`; PowerShell `$env:X`; Bash `$X` and `${X}` of variables never assigned in the file.

## 6. Resolution and derived data

| language | module unit | import resolution |
|----------|-------------|-------------------|
| Go | package directory | `go.mod` and `go.work` module paths plus import path -> directory |
| Rust | module tree from `mod` declarations and file layout; crate per `Cargo.toml` target | `use` paths with `crate::`, `super::`, `self::`, workspace crates, `pub use` re-exports |
| TypeScript, JavaScript, Vue, Svelte, Astro | file | relative specifiers with extension and index probing; `tsconfig.json` and `jsconfig.json` `paths` and `baseUrl`; `package.json` `exports` and workspaces; CommonJS `require` |
| Python | module file or package; source roots: root, `src/`, pyproject package directories | absolute and relative imports, namespace packages, `__init__.py` re-exports |
| Java, Kotlin, Scala, Groovy | package declaration; source roots `src/{main,test}/<language>/` or the path suffix matching the package | single-type, wildcard and static imports; same package without import |
| C#, Visual Basic .NET | namespace; project from `.csproj`, `.vbproj` and `.sln` | `using` and `Imports`, global usings, enclosing and same namespace; partial types merged |
| C, C++, Objective-C | file; header and implementation paired by stem | `#include` and `#import`: relative to the file, then manifest include directories, then `include/` and `src/` probing; C++ namespaces and `using` |
| Swift | SwiftPM target directory from `Package.swift`, else top-level directory | `import` module; whole-module visibility; extensions merged into the extended type |
| Ruby | file; constants by Zeitwerk path convention | `require`, `require_relative`, `load`; autoload paths <-> constant names; reopened classes merged |
| PHP | namespace; PSR-4 roots from `composer.json` | `use` for classes, functions, constants; `require` and `include` with literal paths |
| Lua | file | `require("a.b")` -> `a/b.lua`, `a/b/init.lua`; `.toc` file lists |
| Zig | file; named modules from `build.zig` | `@import` of paths and named modules |
| Elixir | `defmodule` name | `alias`, `import`, `use`, `require`; fully qualified calls |
| Erlang | module = file name | `Module:fun` remote calls, `-import`, `-include` |
| Dart | file; package root from `pubspec.yaml` | `package:` and relative imports, `part` and `part of` |
| Julia | `module` block or file | `include`, `using`, `import` |
| R | file; package from `DESCRIPTION` and `NAMESPACE` | `source` with literal paths, `library`, `pkg::name` |
| OCaml | file module (capitalized file name); libraries from `dune` | `Module.name`, `open`, `include` |
| Fortran | `module` units | `use`, `include` |
| Pascal, Delphi | `unit` | `uses` clauses; forms linked to the unit of the same name |
| Common Lisp | `defpackage` and `in-package`; systems from `.asd` | `package:symbol`, `:use` lists, `.asd` components |
| Solidity | file | `import` paths with `remappings.txt` and `foundry.toml` remappings |
| Verilog, SystemVerilog | `module`, `package` | instantiation by module name, `` `include ``, `import pkg::*` |
| PowerShell | file; module manifests | dot-sourcing and `Import-Module` with literal paths |
| Bash | file | `source` and `.` with literal relative paths |
| SQL | schema object | object names in DDL and DML |
| Terraform, HCL | module directory | `module` blocks with local `source`; `var.`, `local.`, `module.`, `data.` and resource address references |
| Robot Framework | file | `Resource` and `Library` imports; keyword calls |
| DM | type path tree | `#include` order from `.dme`; type paths |
| Markdown and other docs | file | relative links; backticked identifiers |

Resolution order for a reference, first hit wins:

1. Lexical scope of the file: enclosing and nested definitions -> exact.
2. Name bound by an import, use, include or require -> exported symbol of the resolved module -> exact.
3. Qualified name (`pkg.Name`, `Type::name`, `Namespace.Type`, `Module:fun`) through the module map -> exact.
4. Same module unit visible without import (Go package, Java package, C# namespace, Swift module, Ruby and PHP autoload roots) -> exact.
5. Member call `x.m()`: receiver type from `self`, `this`, `super`; declared types of parameters, fields and variables; constructor and factory expressions (`new T`, `T(...)`, `T::new`, `T.init`, functions with a declared return type) -> method of the type or its supertypes; exact for declared types, inferred for factory return types.
6. Method of an interface, trait, protocol or abstract type -> edges to every implementation, inferred, max 10 targets, else one ambiguity notice.
7. Unique name among the modules the file imports, then unique name in the root (non-test preferred) -> inferred.
8. Otherwise no edge; counted per file as ambiguous.

- Implementation edges: `impl Trait for Type`, `implements`, `extends`, base and conformance lists, categories and extensions: exact; Go: type method set contains every interface method name with equal parameter count: inferred.
- Contract edges: OpenAPI `operationId`, Protobuf rpc and GraphQL field -> handler whose name matches within the implementing type or module: `implements`, inferred.
- Edge kinds: `calls`, `uses` (type reference), `inherits`, `implements`, `imports` (file -> file or module), `mentions` (doc -> symbol or file), `tests` (test symbol -> symbol it calls), `references` (code, doc, template or config -> file or asset), `runs` (task -> script, entrypoint or task), `queries` (symbol -> SQL object named in a string literal with an SQL keyword).
- Docs: backticked identifier with a unique symbol name: inferred; relative path mention or link: exact.
- Confidence: `exact` or `inferred`. Targets outside the root: external dependency per module (name, use count), no node.
- Edges are unique per (source, target, kind); exact wins over inferred.

| derived item | definition |
|--------------|------------|
| symbol graph | CSR forward and reverse adjacency over resolved edges |
| rank | PageRank, damping 0.85, 30 iterations, edge weight exact 1.0 and inferred 0.5; `test` and `gen` symbols excluded |
| modules | language module units; module graph = aggregated cross-module edges with counts |
| cycles | strongly connected components with more than one module in the module import graph |
| layers | longest-path level per module on the condensed module DAG |
| entrypoints | Go `main` in package `main`; Rust `main` of bin targets; `main` of Java, Kotlin, Scala, C#, C, C++, Dart and Zig; Swift `@main` and `main.swift`; Python `if __name__ == "__main__"` blocks; manifest entrypoints; executable files with shebang |
| env surface | per variable: definition sites (env def facts) and read sites |
| tasks | Makefile targets, package scripts, CI jobs and compose services with what they run (`runs`) |
| contracts | API operations (OpenAPI, Protobuf, GraphQL) with their implementing handlers |
| asset refs | referenced files and assets with their referrers |
| test map | symbol -> tests reaching it within 2 `calls` hops |
| dead candidates | private, non-entry, non-test symbols without incoming edges, excluding interface and trait implementations and init hooks; unused exported symbols listed separately |
| stale doc references | doc mentions of identifiers matching no symbol and of relative paths matching no file |
| env drift | variables read without a definition site; variables defined without a read site |
| unreferenced assets | `asset` files without incoming `references` |
| search index | token -> symbol postings from name, parent chain, path, signature, doc, the names of its callers and users (max 16) and body words (tokens of the identifiers, strings, comments and markup of the symbol's syntax-tree byte range, else its lines, its doc comment included, outside nested symbols; CSS literals skipped only in exact `class`/`className` attributes, bound class expressions and recognized class-composition calls (`cn`, `clsx`, `classnames`, `classNames`, `cx`, `twMerge`, `twJoin`, `cva`, `tv`); dynamic identifiers, comments, unrelated literals and nested ordinary-call strings kept; max 64, source role only; one-line and minified symbols included, neighbors on the same line never share words); token -> file postings from text segments, config key paths and words of top-level code |

- Tokenization: split on non-alphanumerics, camelCase and PascalCase boundaries and letter-digit boundaries; lowercase; light English suffix stripping (`-s`, `-es` after sibilants, `-ies`, `-ed`, `-ing`); common code abbreviations and their spelled-out words map to one form (`dir`, `config`, `repo`, `env`, `error`, `context`); an English stopword list applies to query terms only. File nodes index their leading comment (module documentation) in the doc field.

Git data requires `git` on PATH and a git work tree; otherwise git sections print `! git unavailable`. Cached in the `git` store file keyed by HEAD.

| git item | definition |
|----------|------------|
| diff | `git diff --unified=0 REV` (default `HEAD`, includes the working tree) -> changed line ranges |
| churn | commits touching each file within the last 1000 non-merge commits |
| co-change | file pairs changed together in at least 3 commits with Jaccard >= 0.3; commits touching more than 50 files ignored |
| hidden coupling | co-change pairs without a static edge between the two files |
| hotspots | churn x (1 + max symbol rank in the file), top 10 |

## 7. Query semantics

Detail ladder: `find` (one line per hit) -> `ctx` (bundle with signatures) -> `show` (full card) -> source read at the cited lines.

### 7.1 map

Sections in priority order: root line (name, files per language, symbol count) -> modules (tree by path, depth max 3; files, symbols, top 3 symbols by rank; max 40 lines) -> entrypoints -> tasks -> hubs (top 10 by rank) -> api contracts -> cycles -> layers -> external dependencies (top 15 by use count) -> env vars (name, read count, definition sources, first read location). Sections other than modules hold max 10 lines; the budget cuts from the lowest-priority section upward.

### 7.2 find

1. Tokenize the query; look up exact names for the raw query and each term. Query-only expansions: `-ization` -> `-iz`, `-ation` -> `-at`, `retriev`/`retrieval` -> `get`, other ASCII terms of at least 4 bytes -> the term plus `iz`; retain the original term.
2. Candidates = union of the postings.
3. Score = BM25F (k1 1.2, b 0.75; field weights name 3, body 1.8, parent 1.5, path 1, doc 1, signature 0.5, used by 0.5; tokens stemmed, code abbreviations and synonyms canonicalized: `remove` and `delete`, `sign in` and `login`, `invalidate` and `refresh`; per term use the higher score of its original and 0.5 x its expansion) times the share of query terms the node matches, + exact-name bonus 5 when the whole query names the symbol, multiplied by the role weight (src 1, test 0.3, gen 0.2, doc 0.5, config 0.5, manifest 0.5, asset 0.3), + 2 x normalized log rank.
4. Order: score descending, then path, line and node ID; files matched by path tokens appear as file lines, unless one of their symbols ranks higher; each further symbol of a file matching the same set of query terms counts 0.4 times the one before (diversity); queries over 64 distinct terms use file-wide diversity including file hits; output keeps the score order, adjacent hits of one file share its path line.

### 7.3 ctx

1. Seeds = top 5 `find` results with score >= 0.25 x top score.
2. Personalized PageRank from the seeds over the symbol graph in both directions, restart probability 0.3, 15 iterations.
3. Order: seeds first, then the highest-ranked result whose symbol name covers a query term absent from the seed names, then PPR score x role weight; queries over 64 distinct terms omit the extra result.
4. Packing: group by file in first-appearance order; seeds with signature and doc, others with signature; stop at the budget.

### 7.4 show

Card order: header (path:start-end, kind, qualified name) -> signature -> doc -> members (methods, field names) -> supertypes -> implementations -> incoming edges by rank (callers, referrers, `runs`, `queries`) -> outgoing edges -> tests -> env reads -> ambiguity notices. Each env read appears only on its recorded owner: the symbol card, or the file card when it has no enclosing symbol. Incoming and outgoing edges follow `--depth` hops, indented per hop. Test confidence includes every caller and test edge in the displayed path; an exact alternative wins. `--exact` requires every edge in that path to be exact. Each section max 10 lines unless `--all`.

### 7.5 impact

1. Targets: NAME symbols, or symbols whose line ranges intersect the `--diff` ranges (`git diff --no-renames`, NUL-delimited raw paths and decoded patch paths, deleted files kept). Untracked and historical paths are NUL-delimited; non-UTF-8 paths are skipped. Compare REV and current definitions by file, package, kind, lexical parents and receiver; match complete declaration tokens first, then a unique remaining pair. Ignore formatting and comments, retain literal contents; verify current source against the index. Incomplete current or revision parsing emits a budgeted path-and-side notice, also for excluded paths; incomplete current files are excluded from graph results, and unmatched identities in the same language are uncertain. Unpaired REV definitions are `removed` with REV path and line; ambiguous pairings or unavailable identities are `uncertain?`. A unique cross-file match is `moved?` when a same-directory package, Git rename detection, identical deleted/added content or similar deleted/added files in a file-scoped language with the same nonempty signature support it; similarity requires the longer file to be at least 32 bytes and >=75% byte-weighted matching lines at the same positions, excluding lines shorter than 3 bytes. Cite both REV and current paths and lines. `--exact` omits inferred move and uncertainty records. Exported signature changes are `sig`, or `sig?` on an inferred move. Current symbols whose unresolved call, type, inheritance or value reference names a removed definition or a move across scope are inferred depth-1 dependents (fact shards, no revision index). Sensitive paths are excluded before revision content is read.
2. Affected: reverse BFS over `calls`, `uses`, `inherits`, `implements`, `references`, `queries` up to `--depth`, ordered by depth, then rank. A dependent is exact only when a shortest chain to a target consists of exact edges; otherwise it is inferred, also on every later hop.
3. Tests: tests reaching any target or affected symbol (test map); confidence includes the full test-to-target chain and the final test edge, with an exact alternative winning. `--exact` drops inferred test paths and their run commands. Run lines get their budget reserved first (package-level form, at most half the budget); a run line is precise (test names) only while every later run line still fits in its package-level form (`go test ./pkg`, `cargo test -p crate`, `./gradlew test`, `mvn test`, `swift test`, `dotnet test`, the bare runner); a line fitting in neither form is counted in `+<n> more`.
4. Run lines: Go `go test ./<pkg> -run '^(<names>)$'` per package; Rust `cargo test -p <crate> -- <names>` per crate; Python `pytest <path>::<name> ...`; Java, Kotlin, Scala, Groovy with Gradle `./gradlew test --tests <class>`, with Maven `mvn test -Dtest=<class>`; C#, VB.NET `dotnet test --filter FullyQualifiedName~<name>`; Ruby `bundle exec rspec <path>` or `bundle exec ruby -Itest <path>`; PHP `vendor/bin/phpunit --filter <name>`; Swift `swift test --filter <name>`; Elixir `mix test <path>`; Dart `dart test <path>`; Zig `zig build test`; other languages list test files only.

### 7.6 path

Shortest path by layered BFS over forward `calls`, `uses`, `inherits`, `implements`, `imports` edges; among equally short paths the one with the fewest inferred hops wins, ties by adjacency order. Without a directed path it falls back to undirected search (not under `--exact`). One line per hop; a hop is inferred when its edge is inferred or taken against edge direction.

### 7.7 audit

Sections: dead candidates -> unused exports -> cycles -> stale doc references -> env drift -> unreferenced assets -> hidden coupling -> hotspots; each max 15 lines.

## 8. Output format

Text:

- Paths are root-relative with `/`. Symbol lines are grouped under their file line and indented two spaces: `<line> <signature>`, or `<line> <kind> <name>` when a symbol has no signature.
- No headers, banners or empty fields. Truncation emits one final line `+<n> more` (JSON `{"t":"more","n":<n>}`), whose space is reserved in the budget; when the full footer cannot fit, text uses `+<n>` or `+`, JSON uses `{"t":"more"}` or `{}`. An empty JSON object is exclusively a truncation marker. Only complete lines and complete JSON records are emitted.
- `map` and `audit` open each section with one keyword line: `modules`, `entry`, `tasks`, `hubs`, `api`, `cycles`, `layers`, `ext`, `env`, `dead`, `unused`, `stale`, `drift`, `assets`, `coupling`, `hotspots`.

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
| `+<n> more`, `+<n>`, `+` | truncated remainder; count omitted only when it cannot fit |

JSON Lines (`--json`): one object per line; absent fields omitted.

| field | type | description |
|-------|------|-------------|
| `t` | string | record type: `sym`, `file`, `edge`, `sec`, `note`, `more` |
| `path` | string | root-relative path |
| `line`, `end` | int | 1-based line range |
| `kind` | string | symbol kind or edge kind |
| `name` | string | qualified name |
| `sig` | string | signature |
| `new_path`, `new_line` | string, int | current location of an inferred moved definition |
| `sig_change` | string | `changed` when an inferred move also changed its exported signature |
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
| `files` | per file: path, size, mtime_ns, optional device/inode/status-change time, content hash, role, language, shard | file set or stat records change |
| `facts/NN` (64 shards by path hash) | per-file source hash and fact records, shard-local string table | a file of the shard changes or its stored facts do not match the file table |
| `derived` | interned strings, symbol table, CSR graph, ranks, modules, search index, test map, audit data | derived content changes |
| `used` | empty; its mtime is the last use for `ls` | a writing query when older than an hour |
| `git` | churn, co-change, hotspots keyed by HEAD and analysis version | key changes and git data is requested |
| `lock` | writer lock | never |

- Encoding: little-endian fixed-size records; strings as (offset, length) into string tables; 64-byte aligned sections; header with magic, format version and XXH3-64 checksum; format 9 fact records include the source XXH3-128 hash; fact shard sections deflated; search postings as delta-varint (node, term frequencies) pairs.
- Writes: new exclusively created temp file in the same directory (`O_CREAT|O_EXCL`; an existing entry or symlink is skipped, never opened) -> rename; no fsync; unchanged files untouched; install and update use the same primitive. A checksum mismatch or truncation on open rebuilds that file.
- Reads: memory-mapped, zero-copy.
- Listing and safe cleanup also recognize format-7 and format-8 containers; queries require the current format and writable queries rebuild older indexes.

## 10. Skill, install, update

| item | value |
|------|-------|
| source | `skills/xcalygraph/SKILL.md`, embedded in the binary |
| content | purpose; when to use it and when `rg` or direct reads are better; command table; output markers; exit codes; rules (verify against source, `--read-only` in read-only tasks, budget) |
| size | max 1500 tokens |
| install | running binary -> `~/.local/bin/xcalygraph` (`$XCALYGRAPH_INSTALL_DIR`) by temp file plus rename unless it runs from there; skill -> `~/.agents/skills/xcalygraph/SKILL.md` plus `.version` stamp, written atomically; symlink `~/.claude/skills/xcalygraph` -> skill directory when `~/.claude` exists and the link is absent (`--no-link` skips it; foreign entries are reported, never replaced); `--skill-only` skips the binary; PATH problems (directory missing from PATH, another binary first) reported, shell files never edited |
| update source | GitHub Releases of the project repository; assets `xcalygraph-<version>-<os>-<arch>.tar.gz` and `SHA256SUMS` |
| release build | stage version/platform archives; preserve unrelated destination entries and refuse occupied outputs; publish each output exclusively; checksum only this build's archives |
| update flow | latest release metadata -> download asset -> verify SHA-256 -> smoke test (`version` reports the release) -> atomic replace of the running binary -> `install --skill-only` with the new binary (failure: exit 1 naming the replaced binary and `xcalygraph install` as the remaining step); when already current, the skill is rewritten if its stamp or content differs |
| progress | phases on stderr when it is a terminal (spinner, download bar with bytes, speed and remaining time, one line per finished phase); off a terminal only the result lines on stdout |
| installer | `install.sh` attached to every release (`releases/latest/download/install.sh`): latest or `--version X.Y.Z`, SHA-256 check, smoke test, then the binary installs itself, the skill and the link; `--no-skill` installs only the binary using an exclusive temporary file beside it and rejects directory targets |
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
| refresh after a single-file change | max 250 ms |
| index size | max 20 MB |
| bytes written per single-file change | changed fact shard + file table + `derived`; max 15 MB |
| peak RSS, full build | max 400 MB |
| skill size | max 1500 tokens |

## 12. Evaluation

Reference corpora: xcalygraph itself, the upstream Graphify clone at the pinned commit (Python), one public repository per deep language pinned by commit under `research/corpora/`, private corpora evaluated locally. Edge precision targets per deep language: exact at least 95 %, inferred at least 85 %; structural languages: at least 85 %.

- Tool comparisons share one verified source snapshot identified by paths, content hashes and executable bits. Upstream graph reuse additionally validates package source/version, extraction mode and graph checksum.
- Every evaluation retains source, question and binary identities plus raw observations in an isolated run directory. Failed extraction or queries invalidate the run; existing evidence is preserved.

| metric | definition |
|--------|------------|
| hit@5 | share of questions whose expected symbol or file appears in the first 5 results of `find` or `ctx` |
| tokens per answer | output tokens of the command that answers the question |
| rg baseline | tokens of `rg` output plus the file reads an agent needs to reach the expected location |
| upstream baseline | hit@5 and tokens of `graphify query` on the same corpus |
| edge precision | share of emitted edges confirmed by source inspection, 50 samples per edge kind |
| agent success | share of questions a headless Claude Code run (`claude -p`) answers with the expected location; arms: no tool (`rg` and reads), Graphify skill, xcalygraph skill; 3 runs per question and arm |
| agent cost | total tokens, turns and wall time per agent run, from `--output-format json` |

Question sets: `scripts/benchmarks/questions/<corpus>.tsv` with columns question, expected location (`path:line` or qualified name); sets for private corpora under `scripts/benchmarks/questions/private/` (gitignored).

Sealed evaluation: `scripts/benchmarks/questions/sealed/manifest.json` records corpus revisions/content hashes, case hashes, prior tuning exposure and answer budgets. JSON Lines cases carry id, question, family and required locator groups; each group contains acceptable source alternatives with path, declaration line, name and file/evidence hashes. Multi-location success requires every group; absent-symbol negatives count returned locators as false positives. File hit@5, exact locator coverage and final agent-answer correctness remain separate metrics. Same-author exposure and question correlation are explicit limitations; inspected sealed sets become development data.

## 13. Deferred features - See docs/deferred.md
