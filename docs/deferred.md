# xcalygraph Deferred Features

Considered and deliberately not in `docs/spec.md`. Each entry names the evidence that moves it into the spec.

| feature | trigger |
|---------|---------|
| MCP server | an agent host that cannot run CLI commands, or an evaluation showing better results than CLI plus skill |
| Query daemon / resident memory index | freshness or startup cost misses the latency targets on reference corpora |
| Leiden communities, surprising cross-community edges | evaluation shows module structure insufficient for orientation questions |
| Multi-repository graph | an owner workflow that needs cross-repository queries |
| Filesystem and server mode (config keys, path references between config files) | a server administration use case with an evaluation question set |
| Further languages (Swift, Kotlin, Java, C, C++, C#, PHP, Ruby, Zig, SQL, Terraform) | an owner corpus that needs them |
| Type-aware resolution (receiver types, generics) or SCIP ingest | inferred-edge precision below 85 % on a reference corpus |
| Framework entrypoints (HTTP routes, CLI frameworks, job handlers) | evaluation questions about service entrypoints that `map` and `find` miss |
| Typo-tolerant search | evaluation misses caused by misspelled terms |
