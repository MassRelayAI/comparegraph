---
title: Mapping gold, first measurement
date: 2026-10-14
valid_until: 2027-04-14
lane: plan 7 lane 1
verdict: "On 94 gold-labelled features across 6 code-graph tools, map scores pooled precision@5 0.046 and recall@5 0.141, and the absent check scores precision 1.00 (1 call) and recall 0.05 (1 of 19 true-absent pairs)."
status: measured — baseline before the improvement task
---

# Mapping gold, first measurement

## The gold
180 pairs (30 features of `families/code-graph-tools` x 6 clones: graphmind, srclight, code-review-graph, CodeGraph, codanna, ripwire),
labelled by two independent readers, merged by `scripts/gold-map-merge.sh` under the rule in `fixtures/gold-map/README.md`.
Disagreements stay in the files with `final: null` and are not adjudicated.

| clone | final non-null | final null | final has / partial / none | kappa (verdict, 4 classes) | unresolved locations |
|---|---|---|---|---|---|
| graphmind | 20 | 10 | 12 / 0 / 8 | 0.848 | 2 |
| srclight | 26 | 4 | 20 / 3 / 3 | 0.828 | 2 |
| code-review-graph | 25 | 5 | 23 / 1 / 1 | 0.643 | 3 |
| CodeGraph | 18 | 12 | 14 / 4 / 0 | 0.808 | 1 |
| codanna | 18 | 12 | 12 / 0 / 6 | 0.651 | 0 |
| ripwire | 14 | 16 | 12 / 1 / 1 | 0.873 | 5 |
| pooled | 121 | 59 | 93 / 9 / 19 | 0.788 | 13 |

Final is null when verdicts differ or, for has/partial, no location is shared (same `path::symbol`, or same path within 10 lines).
Final locations are the shared ones only (at most 3). Unresolved = a gold location with no node of that path and name (or span) in the engine graph;
the scorer leaves them out. Reasons for the 13: 7 module-level constants or SQL strings (`SCHEMA_SQL`, `FTS_SQL`, `DEFAULT_IGNORE`,
`EXTENSION_TO_LANGUAGE`, `_EDGE_TARGET_KINDS`; the engine makes no node for a constant), 5 C++ header declarations in ripwire (`buildGraph`,
`collectSources`, `packTaskBundleText`, enum `Lang`; no node from the `.h` file), 1 tool handler registered through a macro/dispatch table
(`codegraph_pr_context`). No normalisation in the merge could fix any of them (class prefixes were already reduced to the bare name), so none was changed.

## Method
Readers worked from the clone source alone and never from engine output (`map`, `relate`, search hits, claims); reader 2 did not see reader 1.
Scoring: `scripts/map-score.sh` with the release binary at HEAD 0102e81, reference provider, one scratch workspace per clone, `family add families/code-graph-tools`.

## Measurement
| clone | map n | unres | P@5 | R@5 | relate (want) | absent P (calls) | absent R (none pairs) | absent n |
|---|---|---|---|---|---|---|---|---|
| graphmind | 12 | 2 | 0.086 | 0.294 | related (related) | 1.00 (1) | 0.125 (1/8) | 20 |
| srclight | 22 | 2 | 0.065 | 0.143 | related (related) | none (0) | 0.00 (0/3) | 26 |
| code-review-graph | 23 | 3 | 0.050 | 0.139 | related (related) | none (0) | 0.00 (0/1) | 25 |
| CodeGraph | 17 | 1 | 0.012 | 0.045 | related (related) | none (0) | none (no none pairs) | 18 |
| codanna | 12 | 0 | 0.038 | 0.133 | related (related) | none (0) | 0.00 (0/6) | 18 |
| ripwire | 8 | 5 | 0.028 | 0.100 | related (related) | none (0) | 0.00 (0/1) | 14 |
| pooled | 94 | 13 | 0.046 | 0.141 | 6/6 ok | 1.00 (1) | 0.053 (1/19) | 121 |

Floors: `fixtures/gold-map/FLOORS.json`, measured - 0.01 (omitted where the measure is undefined). `cargo test -p comparegraph --test map_gold`: 2 passed.

## Worst features for map (recall@5, pooled over clones that have it; all 0)
- `update.ignore-rules` 0/8: the gold points at `DEFAULT_IGNORE`, `_should_ignore`, `collect_files`; map returns update/updater commands and test files.
- `symbols.signatures` 0/7: gold is `_extract_signature`, `build_signature`; map returns generic test and CLI functions whose words overlap.
- `graph.inheritance` 0/6: gold `_build_inheritance_edges`, `get_type_hierarchy`; map returns graph-named files and commands instead.
- `symbols.extraction` 0/5: gold `_extract_symbols`, `insert_symbol`; map ranks tests, CLI and DB methods above the extractor.
- `graph.call-graph` 0/5: gold `_build_edges`; map returns community/db functions. Also 0 for `symbols.multi-language`, `graph.call-resolution`, `serving.http-api`.
Common cause: ranking by title word overlap pulls tests, CLI entry points and `file` nodes; the implementing function is named differently from the feature words.

## Absent false-absent cases
None: the check called absent only once in 121 pairs (graphmind, correct). The failure is the opposite: 18 of 19 true-absent pairs were not called absent
(recall 0.05), because the lexical channels hit on common words (e.g. `code`, `analysis` for `analysis.dead-code` on ripwire) and the check then declines. Feeds the improvement task.

## Limits
6 clones of one family and 30 features; 94 map rows. Readers were agents, kappa 0.64 to 0.87, and 59 pairs (33%) are excluded as disagreements, so the gold
leans to easy pairs. CodeGraph has no `none` rows. Absent precision rests on one call.

## Next
IMP (map ranking: kind and name weighting, demote tests and file nodes; absent check: stop word and kind-aware terms), H2H baselines on the same gold, PROD.
