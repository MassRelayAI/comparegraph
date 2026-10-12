---
title: Proof on a real product repo as root with three harness comparators
date: 2026-10-13
valid_until: 2027-01-13
lane: plan-5-P
verdict: the graph read and the claim and matrix machinery work on a 4.4k-file private monorepo and on goose, but two of three comparator builds crash on a UTF-8 slicing bug, the product build takes 3.5 minutes, and the head-to-head with codegraph-mcp could not be measured because the incumbent hung
status: decided
---

Subject: a private product monorepo (Rust services + a TypeScript API + a React web app, ~2k source files; 4,370 files
parsed including scripts, SQL, CSS and config), added with `--as-root`. Comparators, by upstream URL:
github.com/NousResearch/hermes-agent, github.com/openclaw/openclaw, github.com/block/goose (remotes verified with
`git remote get-url origin`). The product was only read; all writes went to a scratch workspace. Binary: release build
of main (1dba411). Everything below was run with `nice -n 10`.

## 1. The product graph read

| step | result |
|---|---|
| `build` | 211 s (wall 3 m 31 s), 4,370 files parsed, 0 cache hits, 51,806 nodes, 144,649 edges (calls 74,976, contains 47,436, tests 17,594, imports 4,591, implements 46, extends 6), 1,132 communities |
| nodes by language | rust 23,249; typescript 18,183; mjs 7,556; tsx 1,130; sql 440; bash 277; css 274; python 265 |
| build limitations (37) | `clamped_line_end` x34 (TOML/YAML table spans end one line past the file), `skipped_dir` (2 dirs), `skipped_minified` (3 files), `entry_file_heuristic` |
| `search record_audit` / `recordAudit` | 1 and 5 hits, 15-19 ms, 250 B and 1.5 KB |
| `show` (Rust `record_audit`) | 0.36 s, 3.3 KB; 50 callers listed (capped), 6 callees (`chain_lock_key`, `last_hash`, `now_iso`, `canonical_stringify`, `compute_entry_hash`, `compute_payload_hash`) |
| callers by SQL on the store | 397 distinct callers at confidence >= 0.5, so `show` lists 50 of 397 and says nothing about the cap in its limitations |
| `traverse --depth 2` | 0.50 s, 63 KB, 200 nodes, **truncated** (max-nodes 200), confidence `derived` |
| `impact` Rust `record_audit` | 0.50 s, 312 KB, 500 nodes in 2 hops, 658 tests reached, **truncated** at 500 |
| `impact` TS `recordAudit` | 0.63 s, 296 KB, 500 nodes in 3 hops, 266 tests, **truncated**, plus `name_guessed_hops` (all 500 reached through a 0.5-confidence name-guessed edge) |
| `tests-for` | 0.54 s, 90 KB, 392 tests, truncated at 200 |
| `entry-points` | 0.59 s, 6,705 candidates (20 shown); confidence `inferred`, limitation `exported_heuristic` |
| `dead-code` | 0.68 s, 3,469 candidates; confidence `unknown`, limitation `dynamic_dispatch_invisible` |
| `communities` | 1,132 total; the largest, labelled `crates`, holds 14,849 nodes (28.7% of the repo), then `apps` 7,721 and `tools` 2,228; cohesion of the largest is 0.02 and the labels are top-level directories, so the clusters mostly mirror the folder layout |

Reading: queries are fast (under 0.7 s), but the answers to the widest symbols are prefixes: every impact and
tests-for answer was truncated and returned 90-312 KB, which is a poor fit for an agent's context. The truncation
is declared in the envelope; the `show` cap is not.

## 2. Head-to-head with codegraph-mcp

| query | CompareGraph found / callers / time / bytes | codegraph-mcp |
|---|---|---|
| symbol search `record_audit` | yes / - / 0.015 s / 412 B | no answer |
| symbol search `recordAudit` | yes / - / 0.019 s / 1,617 B | not run |
| callers of `record_audit` | 50 listed of 397 / 0.36 s / 3.3 KB (`show`) | no answer |

Verdict. This comparison could not be made. The one-shot form (`--run-tool codegraph_symbol_search`) was not warm:
it re-scanned the workspace for about 4.5 minutes (its log says "Indexed 2907 files, persisted 21,507 nodes and
84,313 edges"), then produced no output for a further 10 minutes until it was killed. The MCP tool of the long-running
server in the same session then returned nothing for 30 minutes and was aborted by the host. The incumbent shares one store
under the home directory with other live codegraph servers on this host, which is the likely cause (a lock or a
corrupted-store recovery path; the store directory holds earlier `.corrupt` copies). Nothing here shows
codegraph-mcp is slower or less accurate when healthy; it shows it was unavailable. On what was measured,
CompareGraph answers in milliseconds from a store it owns, which is a real operational advantage over a shared
store that can hang, but its cold build (3.5 minutes) is slower than the incumbent's incremental reindex would be.
Index sizes differ: codegraph-mcp stored 21.5k nodes (it skips docs, large files and parses 44 of 2,907 changed
files incrementally) against CompareGraph's 51.8k nodes, which include config, SQL and test cases as nodes.

## 3. The comparison run

Pack `harness-agents`: the 12 basics rows of the product's own harness taxonomy converted by script (id kept with
the dot, group = prefix, tier basics, weight 2 because the source has none); `family add` accepted all 12.

Comparator builds:

| clone | result |
|---|---|
| goose | 33 s, 1,379 files, 19,429 nodes, 44,328 edges, 332 communities, 88 limitations |
| hermes-agent | **panic after 356 s**: `resolve.rs:270` "start byte index 2410 is not a char boundary; it is inside '×'" |
| openclaw | **panic after 822 s**: same line, "inside '─'" |

Cause (read from the source): in the declared-type resolver, `before.rfind(|c| !is_id(c)).map_or(0, |k| k + 1)`
adds one byte to the index of a non-identifier character, which is wrong when that character is multi-byte. Any
clone with a non-ASCII character in that position kills the whole build, with no partial result.
Consequences: the hermes-agent and openclaw cells are empty, and `claim` refuses them ("has no finished run").

`relate`: product `related` (1.0) and goose `related` (1.0) for `harness-agents`; hermes-agent and openclaw have no
run, so no band. All 12 rows were listed as the basis for both, which says the band is easy to reach.

`map`: product 0.75 s, goose 0.64 s. Real hits: `crates/mr-control/src/agentloop/guards.rs` for loop guards,
`crates/mr-gateway/src/model_gate.rs` for model hops, goose `agents/mcp_client.rs` for MCP tools and
`platform_extensions/orchestrator.rs` for the orchestrator. Noise: `multi-channel-bridges` matched `multiple...` test
names and `multiplier`, and several goose rows matched test functions. The candidates are leads, not evidence.

Claims (author `proof-p`, all read at the cited lines before recording):

| clone | feature | verdict | citation |
|---|---|---|---|
| product | agents.audit-append-only | has | crates/mr-kernel/src/audit.rs:402-425 |
| product | agents.loop-guards | has | crates/mr-control/src/agentloop/guards.rs:22-70 |
| goose | agents.mcp-tools | has | crates/goose/src/agents/mcp_client.rs:108-125 |
| goose | agents.subagents | has | crates/goose/src/agents/subagent_handler.rs:36-50 |
| goose | agents.tool-approval-hitl | has | crates/goose/src/agents/state_machine/ops_tool_approval.rs:164-170 |
| goose | agents.audit-append-only | none | `--searched` (see below) |

The two requested hermes-agent claims could not be recorded (no run); goose took four of the six instead.
The `none` guard worked: a first attempt searching "audit hash chain tamper" was refused with 54 hits in goose,
and only the exact terms `audit_log`, `append_only`, `hash_chain` (0 hits each) were accepted. Because I recorded
three single-term searches one after another, the claim has a three-row supersede chain (ids 4, 5, 6); only the
last counts in the matrix. Only one search term is stored per claim, so a `none` rests on a narrow term.

Matrix (`matrix harness-agents`, 16 ms; 12 rows x 4 columns):

| feature | product | goose | hermes-agent | openclaw | gap | differentiator |
|---|---|---|---|---|---|---|
| agents.audit-append-only | has | none | - | - | no | yes |
| agents.loop-guards | has | - | - | - | no | yes |
| agents.mcp-tools | - | has | - | - | yes | no |
| agents.subagents | - | has | - | - | yes | no |
| agents.tool-approval-hitl | - | has | - | - | yes | no |
| 7 other rows | - | - | - | - | no | no |

Totals: 3 gaps, 2 differentiators. The three gaps are artefacts of my not claiming those rows on the product (the
product has them; the matrix cannot know), so the matrix is only as good as the coverage of claims. `decide goose
park --why "proof run" --by proof-p` recorded (id 1).

## 4. Vault and exports

| step | result |
|---|---|
| `export vault` | 4.17 s, 66,970 files, 275 MB, 2 repos (the two built ones), 1,464 communities, 65,486 symbols |
| `export json` | 3.72 s, 63 MB printed to stdout in one object |
| `export csv` (needs `--out`) | 1.12 s, 42 MB: nodes 71,235, edges 188,977, claims 6, decisions 1, one matrix file |
| `check` | 1.3 s: 4 repos, 2 runs done, 0 stale locations, 0 drifted runs, 0 downgraded claims, schema ok, no problems |

The vault is one file per symbol; at this size it is too large to open in a note tool as one vault, and the 4-repo
store exports only the two repos that have a run.

## 5. What did not work or was slow

1. Build panic on non-ASCII text in two of three comparators (resolve.rs:270); the panic comes after 6 and 14 minutes of
   work and loses everything.
2. Cold builds are slow: 3.5 min for the product, 6 min (to a crash) for hermes-agent, 14 min (to a crash) for openclaw.
   Goose, similar in size to the product's Rust part, took 33 s, so the time is not linear in files; the resolver on
   large Python/TypeScript trees is the suspect, not measured here.
3. codegraph-mcp was unusable during the run (section 2).
4. `show` lists 50 callers of 397 without a limitation entry.
5. `relate` gave `related` to everything with all 12 rows as the basis; `map` has substring noise.
6. `export csv` requires `--out` while `export json` writes to stdout; the README example may not match.

## 6. Fit for the maintainer's use

Fit for the read side and the evidence discipline; not yet fit for the claim "compare against other open-source
projects" until the build crash is fixed. What held: fast graph queries with honest envelopes (truncation,
inferred or unknown confidence, guessed hops), claims refused without a resolving citation, a `none` refused when
the search finds hits, `check` clean, and exports that complete. What blocks: two of three comparators cannot be
built, and the head-to-head with the existing tool remains open.

Next steps, in order:

1. Fix `resolve.rs:270` (use `k + c.len_utf8()` via `char_indices`), add a regression fixture with `×` and `─`, and make a
   per-file parse panic a recorded limitation instead of a lost build; then rebuild hermes-agent and openclaw and
   record the two missing claims.
2. Re-run the codegraph-mcp comparison when no other codegraph server is running (stop the live servers first or give it a
   private store), with the same three queries, and add `show` caller-cap and impact-size limits (a `--limit` and a
   compact mode) so a widest-symbol answer fits an agent's context.
3. Extend the claims to all 12 basics for the product and goose, then re-read the matrix; consider an `unclaimed`
   cell state distinct from `-` so a missing claim is not mistaken for a gap.

## After the crash fix (2026-10-13)

Fix: the char search at `resolve.rs:269` now advances by `len_utf8`; an audit of every byte-index slice on source text in the
reference provider and the application layer found no other char-search arithmetic. A panic in any resolution pass now fails
the run with the file named (`resolution panicked while processing <file>`) instead of crashing the process. `show` reports
`truncated` (with `callers_total`) when a list hits its cap and takes `--max-callers` (default 50, max 2000).
Both comparators were rebuilt with the new release binary, parse results coming from the cache of the crashed attempts.

| comparator | seconds | files parsed | nodes | edges | communities | limitations |
|---|---|---|---|---|---|---|
| github.com/NousResearch/hermes-agent | 162 | 10,288 | 166,253 | 443,009 | 967 | 236 (233 clamped_line_end, skipped_dir 1, skipped_minified 1, entry_file_heuristic 1) |
| github.com/openclaw/openclaw | 1,866 | 38,258 | 433,887 | 1,321,920 | 4,254 | 614 (611 clamped_line_end, skipped_minified 1, entry_file_heuristic 1, no_inheritance_go 1) |

The openclaw build took 31 minutes with every file already parsed (38,258 cache hits, 0 misses), so the time is the
resolution and edge passes, not parsing. That is the next performance question; the product (51k nodes) took 3.5 minutes and
the 434k-node comparator 31, which is worse than linear in nodes.

`relate` now scores all four columns `related` (1.0) for `harness-agents`. Two claims were written on hermes-agent after reading
the cited lines: `agents.mcp-tools` has (`tools/mcp_tool_discovery.py:207-238`, connects a configured MCP server and registers its
tools) and `agents.subagents` has (`tools/delegate_tool.py:417-428`, `delegate_task` spawns child agents, single or batch, with
list, steer and stop control). `check`: 4 repos, 4 runs done, 0 stale locations, 0 drifted runs, no problems.

Matrix, `harness-agents`, all four columns (`-` is no claim, not an absence):

| feature | product | goose | hermes-agent | openclaw | gap | differentiator |
|---|---|---|---|---|---|---|
| agents.audit-append-only | has | none | - | - | no | yes |
| agents.loop-guards | has | - | - | - | no | yes |
| agents.mcp-tools | - | has | has | - | yes | no |
| agents.subagents | - | has | has | - | yes | no |
| agents.tool-approval-hitl | - | has | - | - | yes | no |
| the other 7 basics | - | - | - | - | no | no |

Prevalence of `mcp-tools` and `subagents` among comparators is now 2 of 3 claimed columns only in the sense of claims made;
openclaw has no claims yet, so the matrix still shows coverage of claims, not of the products. Next: claim openclaw, extend
claims to all 12 basics, and profile the resolution passes on the 434k-node graph.
