# CompareGraph

CompareGraph maps many cloned repositories onto one taxonomy and puts evidence on
every cell. It reads clones and never runs them. A provider parses each clone
into a graph of nodes and edges (every node cites a file and line range), the
store is deterministic (same clone, same hash), and `check` verifies schema,
determinism and locations. Empty results are reported, never silent.

## Install

```
cargo install --path crates/comparegraph
```

## Commands

All commands take `--root <dir>` (the workspace, default `.`) and `--pretty`.

```
comparegraph add /path/to/clone --name myrepo     # register a clone
comparegraph build myrepo                         # parse it (provider: reference)
comparegraph check                                # schema, determinism, locations
comparegraph export json --repo myrepo            # export the graph as JSON
comparegraph export vault --out ./vault           # Obsidian-style vault (+ llms.txt, AGENTS.md)
comparegraph repos                                # registered repos with their latest run
comparegraph communities myrepo                   # module clusters of the latest run
comparegraph search "parse config" --repo myrepo  # full-text search over symbols
comparegraph show myrepo "src/a.ts::f"            # a symbol: callers, callees, community, licence lane
comparegraph show myrepo "src/a.ts::f" --detail full   # plus its current lines (read from the clone)
comparegraph doctor                               # store and every provider
comparegraph conform reference                    # run golden fixtures on a provider
comparegraph mcp                                  # MCP stdio server over the read verbs
```

The vault is organised for cheap triage: `index.md` is the entry and counts every note group, every note carries a one-line `description:` built from stored facts (no model), and `history/<repo>.md` lays out a repo's dated claim and decision trail, with superseded rows marked. Its `AGENTS.md` teaches the short workflow.

### Analysis verbs

```
comparegraph family add families/example-code-graph-tools   # load a taxonomy pack (rows.md)
comparegraph family propose myfam --repos a,b,c             # draft a pack from terms the clones share (no model)
comparegraph family list | family show <name>
comparegraph map myrepo myfam [--feature id]      # candidate symbols and communities per feature; writes nothing
comparegraph relate [--repo r] [--family f]       # related | adjacent | unrelated; --stored reads without computing
comparegraph claim myrepo group.slug has --author me --at src/a.py:10-20   # evidence that resolves now
comparegraph claims [--history]                   # latest claim per clone and feature
comparegraph decide myrepo adopt --why "..." --by me [--feature id --family f]   # a person's decision, append-only
comparegraph decisions [--history]
comparegraph matrix myfam                         # features x clones, prevalence, gaps, differentiators
```

The engine proposes and records; it never writes a claim or a decision on its own. `claim` needs an author and, for
has/partial, citations that resolve now. Provisional choices are listed in `docs/design-notes-for-review.md`.

## Method

`method/` holds six plain-markdown skills (`adding-a-clone`, `defining-a-family`, `mapping-features`, `writing-a-claim`,
`recording-a-decision`, `reading-the-vault`): when to use each verb, the exact commands, what the engine refuses, and how to
verify. `families/example-code-graph-tools/` is a small example pack.

Every answer is one JSON envelope (`ok`, `verb`, `schema`, `data`, `evidence`, `page`, `cost`, `error`).
Exit code 0 when `ok`, 1 otherwise. List verbs take `--limit` (default 50, max 500) and `--cursor`.
Error codes: `store_error`, `provider_unavailable`, `not_found`, `empty`, `below_floor`, `check_failed`, `io_error`.

## MCP

`comparegraph mcp` speaks JSON-RPC 2.0 over stdio (one object per line, protocol `2025-06-18`) and exposes
the read verbs (repos, search, show, communities, check, doctor, families, map, relate, claims, matrix) and one write tool, `comparegraph_claim` (author required). Decisions are not exposed. Client configuration:

```json
{"mcpServers":{"comparegraph":{"command":"comparegraph","args":["--root","/path/to/workspace","mcp"]}}}
```

## Schema

The store schema (v1) is specified in section 4 of
`docs/superpowers/specs/2026-10-12-comparegraph-design.md`.

## Taken from others

Code and ideas adapted from other projects are listed, with licences, in
`THIRD_PARTY_NOTICES.md`.

## Licence

Apache-2.0. See `LICENSE`.
