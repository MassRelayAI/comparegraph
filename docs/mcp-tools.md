# MCP tools

The tools `comparegraph mcp` serves. [`catalog.json`](catalog.json) is the machine-readable source for the tool list and the error codes; `comparegraph catalog --check` fails when it drifts from the code.

`comparegraph mcp` speaks JSON-RPC 2.0 over stdio, one object per line, protocol `2025-06-18`. Every tool returns the same
envelope as the CLI. 37 tools: 36 read-only, one write (`comparegraph_claim`). The list is generated into
[`docs/catalog.json`](catalog.json) and checked in CI by `comparegraph catalog --check`.

| Tool | Answers | Key arguments |
|---|---|---|
| `comparegraph_repos` | Which repos, with licence, lane, role, head and latest run; archived repos are left out unless asked; `no_port` lists the repos whose code must not be ported, each with why (`lane study`, `licence AGPL-3.0`, `licence unread`) | `limit`, `cursor`, `archived`, `all`, `no_port` |
| `comparegraph_search` | Which symbols match these words | `query`, `repo`, `kind`, `limit`, `cursor` |
| `comparegraph_show` | One symbol: callers, callees, community, licence lane | `repo`, `qualified_name`, `detail`, `max_callers` |
| `comparegraph_communities` | Module clusters of a repo | `repo`, `limit`, `cursor` |
| `comparegraph_check` | Is the store sound: schema, search index, citation locations, the action log. A true read: it writes no flag; a clone that is missing or at another head than its run read is `clone_unavailable` and its locations are not read | none |
| `comparegraph_relevance` | Per repo: factor counts and, with the user's `relevance-policy/1` file, a score and a class (`unrated` without one); `explain` gives every term of one repo | `policy`, `family`, `explain`, `class`, `as_of`, `limit`, `cursor` |
| `comparegraph_governance_verify` | Does the action log beside the store verify: each hash, a gapless seq, the head kept in the store, each archive flag against its manifest | none |
| `comparegraph_families` | Which families, with feature counts | none |
| `comparegraph_map` | Candidate symbols and communities per feature | `repo`, `family`, `feature`, `limit` |
| `comparegraph_absent_check` | Four-channel absent check for one feature, plus the ranked `read_set` (top 10 candidates) a `none` must read (reads; it cannot record a claim) | `repo`, `family`, `feature`, `searched` (optional) |
| `comparegraph_relate` | Stored relatedness band per clone and family (reads, does not compute) | `repo`, `family` |
| `comparegraph_claims` | Latest claim per clone and feature, with citations | `repo`, `family`, `feature`, `history` |
| `comparegraph_matrix` | Features against clones, prevalence, gaps, differentiators; `relatedness` is `not run` until relate has scored the clone | `family`; optional `relate` (bool, default false: score unscored comparators first) |
| `comparegraph_ui_products` | UI evidence: products with screens, evidence files, states, flows, menus | optional `ui_root`, `max_nodes` |
| `comparegraph_ui_screens` | UI evidence: the screens of a product with archetype and captures | `product` |
| `comparegraph_ui_show` | UI evidence: one capture with E1..E12 pass/fail, layout, style, image path | `product`, `screen`; optional `state`, `viewport`, `seat` |
| `comparegraph_ui_diff` | UI evidence: two products on the same screen, state and viewport, in words and numbers | `target`, `ours`, `ref`; optional `state`, `viewport`, `seat`, `ref_seat` |
| `comparegraph_ui_matrix` | UI evidence: family rows x products, cell = pass / 12 or `-`; `net` also lists captures with failed network calls | `family`; optional `state`, `viewport`, `net` |
| `comparegraph_ui_system` | UI evidence: a product's design system against the targets | `product` |
| `comparegraph_ui_flow` | UI evidence: one flow's steps and dead ends | `product`, `task` |
| `comparegraph_ui_nav` | UI evidence: a product's menu tree and derived numbers | `product`; optional `viewport`, `seat` |
| `comparegraph_ui_graph` | UI evidence: a product's interaction graph (ui-graph/1): depth, orphans, dead actions, duplicate paths, cycles | `product`; optional `viewport`, `seat` |
| `comparegraph_ui_path` | UI evidence: the shortest click path between two screens, bounded | `product`, `from`, `to`; optional `viewport`, `seat`, `max_len` |
| `comparegraph_ui_graph_diff` | UI evidence: two interaction graphs (or `target`) compared by depth to each screen, orphans, dead actions | `ours`, `ref`; optional `viewport`, `seat`, `ref_seat`, `tree` |
| `comparegraph_ui_nav_diff` | UI evidence: two menu trees compared in words | `ours`, `ref`; optional `viewport`, `seat`, `ref_seat` |
| `comparegraph_ui_net_diff` | UI evidence: the requests two products make on the same screen, state and device: added, removed, changed (outcome or response shape). Reads the probe's `net.behaviour` / `net.shapes` / `net.completeness`; a truncated capture is never `unchanged` | `ours`, `ref`; optional `screen`, `state`, `device` |
| `comparegraph_ui_journey` | UI evidence: a journey per product (completed / partial / n/a) | `journey`; optional `products` |
| `comparegraph_ui_coverage` | UI evidence: coverage table of a run, blank cells reported | `run` |
| `comparegraph_ui_verify` | UI evidence: the determinism gate between two runs (also `net.failed`) | `run_a` + `run_b`, or `root_a` + `root_b` (one run per ui root); `integrity` re-hashes every evidence file and blob |
| `comparegraph_ui_limits` | UI evidence: every cap the ui tools apply (max_nodes, file size, page size, path length), its unit and what is reported when it is hit | none |
| `comparegraph_claim` | **Write.** Record a claim | `repo`, `feature`, `family`, `verdict`, `author`, `at[]`, `searched`, `read_hits[]` (the `read_set.candidates` names, after opening each), `note` |
| `comparegraph_traverse` | Reachable nodes, summary first, then one page by path confidence | `repo`, `qualified_name`, `direction`, `depth` (max 10), `max_nodes` (max 2000), `kinds`, `min_confidence`, `limit`, `cursor` |
| `comparegraph_impact` | Blast radius: summary (`by_hop`, `by_kind`, `tests_total`), then one page, plus the tests that reach it (capped at `limit`) | `repo`, `qualified_name` or `files[]`, `depth`, `max_nodes`, `min_confidence`, `limit`, `cursor` |
| `comparegraph_tests_for` | Tests linked to a symbol, summary first, then one page | `repo`, `qualified_name`, `max_nodes`, `min_confidence`, `limit`, `cursor` |
| `comparegraph_entry_points` | Exported, handler or main symbols nothing calls | `repo`, `limit`, `cursor`, `min_confidence` |
| `comparegraph_dead_code` | Dead-code candidates (never proof) | `repo`, `limit`, `cursor`, `min_confidence` |
| `comparegraph_doctor` | Store and provider status | none |

There is no `decide` tool: adopting or rejecting a clone is a human act on the CLI. For the same reason the store-safety
writes (`backup`, `restore-store`, `gc`, `archive`, `restore`, `check --mark`) have no tool: they are CLI only, dry-run by
default where they change the store, and each appends one entry to the action log. `min_confidence` is `0.5` (default,
includes guessed hops, labelled in `evidence`) or `1.0`. A name-only match is capped at `0.5` and never binds onto a method.
Truncation (`depth`, `max_nodes`) is recorded in `evidence.limitations`. By hand, add this to `.mcp.json`:

```json
{"mcpServers":{"comparegraph":{"command":"comparegraph","args":["--root","/path/to/workspace","mcp"]}}}
```


[Back to README](../README.md)

## Output schemas and structured content

Every tool advertises a compact `outputSchema` next to its `inputSchema` (JSON Schema 2020-12): the envelope (`ok`, `verb`, `schema`, `data`, `evidence`,
`page`, `cost`, `error`) with this tool's `verb` and the `data` keys a caller can rely on (typed; `!` keys are always present in a successful answer;
other keys stay open, so the schema is always a widening of the real answer). `tools/call` returns the same envelope as `structuredContent` and as
the text content. A failed call has `ok: false`, `data: null` and an `error` with `code`, `message`, optional `hint` and a machine-readable `recovery`
(`{kind, operation?, retryable}`; `kind` is `fix_arguments`, `retry`, `run_command`, `inspect`, `choose_candidate` or `none`). The per-tool part is
one line of `crates/comparegraph/src/application/output_schema.rs`; `tests/output_schema.rs` calls every tool on the fixtures and validates the real
answer against the advertised schema. Together the 37 schemas are about 50 KB on the wire (the input schemas are about 27 KB).
