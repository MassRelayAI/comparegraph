---
family: example-code-graph-tools
description: Features of tools that index source code into a graph for search and navigation (an example pack)
---
| id | group | feature | definition | counts_as_has | tier | weight |
|---|---|---|---|---|---|---|
| symbols.extraction | symbols | Symbol extraction | Functions, classes and methods are parsed out of source files into named nodes with a kind and a line span | a parser pass (tree-sitter or AST walk) that stores one record per symbol with its file and lines | basics | 3 |
| graph.call-graph | graph | Call graph | Which symbol calls which, kept as edges that can be queried both ways | stored caller-callee edges plus a query for callers or callees | basics | 3 |
| graph.import-graph | graph | Import graph | Which file or module imports which other | stored import edges between files or modules | common | 2 |
| graph.communities | graph | Community detection | Symbols or files are clustered into modules from the graph alone | a clustering step (connected components, Leiden or similar) whose result is stored and labelled | extra | 1 |
| search.fulltext | search | Full-text search | Symbol names and text are searchable by keyword with ranking | an inverted index (FTS5 or similar) queried by a search command or tool | basics | 2 |
| search.embeddings | search | Embedding search | Symbols are searched by meaning through vector embeddings | embeddings computed for symbols or chunks and a nearest-neighbour query over them | extra | 2 |
| serving.mcp-server | serving | MCP server | The index is served to coding agents over the Model Context Protocol | a command that starts an MCP server exposing query tools | common | 2 |
| registry.multi-repo | registry | Multi-repo registry | Several repositories are indexed side by side and queried by name | a registry or workspace of repos with per-repo selection in queries | common | 2 |
| update.incremental | update | Incremental update | Re-indexing touches only files that changed | change detection by hash, mtime or git diff that skips unchanged files | common | 2 |
| export.formats | export | Export formats | The graph can be written out for other tools | a command that writes JSON, markdown or another documented format | candidate | 1 |
