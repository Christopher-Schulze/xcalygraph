# xcalygraph Deferred Features

Considered and deliberately not in `docs/spec.md`. Each entry names the evidence that moves it into the spec.

| feature | trigger |
|---------|---------|
| MCP server | an agent host that cannot run CLI commands, or an evaluation showing better results than CLI plus skill |
| Query daemon / resident memory index | freshness or startup cost misses the latency targets on reference corpora |
| Leiden communities, surprising cross-community edges | evaluation shows module structure insufficient for orientation questions |
| Multi-repository graph | an owner workflow that needs cross-repository queries |
| Filesystem and server mode (config keys, path references between config files) | a server administration use case with an evaluation question set |
| Full type inference (generic instantiation, return types beyond declared ones) or SCIP ingest | inferred-edge precision below 85 % on a reference corpus |
| Audio and video transcription, image OCR and captioning | a local engine without runtime dependencies plus an evaluation showing agent value |
| Live database schema introspection | an owner workflow where schema files in the tree are insufficient |
| Google Workspace and MCP server ingest | an owner workflow that needs them |
| Framework entrypoints beyond file-based routes (code-registered HTTP routes, CLI frameworks, job handlers) | evaluation questions about service entrypoints that `map` and `find` miss |
| Go `//go:embed` pattern resolution to files (directive facts are extracted) | agent questions about embedded assets that `show` on the directive's variable misses |
| Rust source files declared under several `#[path]` module names | a reference corpus where this pattern produces wrong or missing answers |
| TypeScript type-system depth beyond declared types (declaration merging across scripts and namespaces, generic substitution, literal key canonicalization) | inferred-edge precision below 85 % or agent-benchmark misses traced to it |
| Bulk directory attribute walker (`getattrlistbulk` on macOS) replacing the `ignore` crate's walk; measured limit about 13 ms below today's freshness check | agent-visible latency caused by the freshness check, or an owner request |
| Hand-written tree walks instead of tree-sitter queries for the most used languages (query walk and compile cost, about 30 % of a build's CPU) | a measured refresh or build target that query cost alone misses |
| Typo-tolerant search | evaluation misses caused by misspelled terms |
| Search text for text-format documents (Markdown, text, reStructuredText, Org, AsciiDoc, LaTeX, HTML) | evaluation questions about documentation content that `rg` answers worse; indexing it exceeded the index size target and slowed single-file refresh with no gain on code questions |
| Media durations of WebM, Matroska, MP3 and Ogg | agent questions that need these durations (format is reported) |
