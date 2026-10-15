# MCP tools

The tools `comparegraph mcp` serves. [`catalog.json`](catalog.json) is the machine-readable source for the tool list and the error codes; `comparegraph catalog --check` fails when it drifts from the code.

`comparegraph mcp` speaks JSON-RPC 2.0 over stdio, one object per line, protocol `2025-06-18`. Every tool returns the same
envelope as the CLI. Eighteen tools: seventeen read-only, one write (`comparegraph_claim`). The list is generated into
[`docs/catalog.json`](catalog.json) and checked in CI by `comparegraph catalog --check`.

| Tool | Answers | Key arguments |
|---|---|---|
| `comparegraph_repos` | Which repos, with licence, lane, role, head and latest run | `limit`, `cursor` |
| `comparegraph_search` | Which symbols match these words | `query`, `repo`, `kind`, `limit`, `cursor` |
| `comparegraph_show` | One symbol: callers, callees, community, licence lane | `repo`, `qualified_name`, `detail`, `max_callers` |
| `comparegraph_communities` | Module clusters of a repo | `repo`, `limit`, `cursor` |
| `comparegraph_check` | Is the store sound | none |
| `comparegraph_families` | Which families, with feature counts | none |
| `comparegraph_map` | Candidate symbols and communities per feature | `repo`, `family`, `feature`, `limit` |
| `comparegraph_absent_check` | Four-channel absent check for one feature (reads; it cannot record a claim) | `repo`, `family`, `feature` |
| `comparegraph_relate` | Stored relatedness band per clone and family (reads, does not compute) | `repo`, `family` |
| `comparegraph_claims` | Latest claim per clone and feature, with citations | `repo`, `family`, `feature`, `history` |
| `comparegraph_matrix` | Features against clones, prevalence, gaps, differentiators; `relatedness` is `not run` until relate has scored the clone | `family`; optional `relate` (bool, default false: score unscored comparators first) |
| `comparegraph_claim` | **Write.** Record a claim | `repo`, `feature`, `family`, `verdict`, `author`, `at[]`, `searched`, `read_hits[]`, `note` |
| `comparegraph_traverse` | Reachable nodes, summary first, then one page by path confidence | `repo`, `qualified_name`, `direction`, `depth` (max 10), `max_nodes` (max 2000), `kinds`, `min_confidence`, `limit`, `cursor` |
| `comparegraph_impact` | Blast radius: summary (`by_hop`, `by_kind`, `tests_total`), then one page, plus the tests that reach it (capped at `limit`) | `repo`, `qualified_name` or `files[]`, `depth`, `max_nodes`, `min_confidence`, `limit`, `cursor` |
| `comparegraph_tests_for` | Tests linked to a symbol, summary first, then one page | `repo`, `qualified_name`, `max_nodes`, `min_confidence`, `limit`, `cursor` |
| `comparegraph_entry_points` | Exported, handler or main symbols nothing calls | `repo`, `limit`, `cursor`, `min_confidence` |
| `comparegraph_dead_code` | Dead-code candidates (never proof) | `repo`, `limit`, `cursor`, `min_confidence` |
| `comparegraph_doctor` | Store and provider status | none |

There is no `decide` tool: adopting or rejecting a clone is a human act on the CLI. `min_confidence` is `0.5` (default,
includes guessed hops, labelled in `evidence`) or `1.0`. A name-only match is capped at `0.5` and never binds onto a method.
Truncation (`depth`, `max_nodes`) is recorded in `evidence.limitations`. By hand, add this to `.mcp.json`:

```json
{"mcpServers":{"comparegraph":{"command":"comparegraph","args":["--root","/path/to/workspace","mcp"]}}}
```


[Back to README](../README.md)
