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
| Framework entrypoints (HTTP routes, CLI frameworks, job handlers) | evaluation questions about service entrypoints that `map` and `find` miss |
| Typo-tolerant search | evaluation misses caused by misspelled terms |
| Search text for text-format documents (Markdown, text, reStructuredText, Org, AsciiDoc, LaTeX, HTML) | evaluation questions about documentation content that `rg` answers worse; indexing it exceeded the index size target and slowed single-file refresh with no gain on code questions |
| Media durations of WebM, Matroska, MP3 and Ogg | agent questions that need these durations (format is reported) |
