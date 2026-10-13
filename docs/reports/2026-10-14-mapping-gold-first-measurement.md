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

## After IMP (2026-10-13, Plan 7 lane 1 task IMP)
Split: TUNE = graphmind, srclight, code-review-graph (every change designed on these); HOLD-OUT = CodeGraph, codanna, ripwire (measured once per kept round, never inspected per feature, never tuned on).
Same scorer, same gold, same release build path. Pooled micro p5 / r5.

| round | change | TUNE p5 / r5 | HOLD-OUT p5 / r5 | absent P / R (TUNE) | kept |
|---|---|---|---|---|---|
| 0 | baseline (FTS OR over group+slug+title) | 0.064 / 0.170 | 0.023 / 0.085 | 1.00 (1) / 0.083 (1/12) | - |
| 1 | IDF terms (share > 0.10 of node names or path tokens weighs 0), definition tokens at 0.5, slug compound form, stem-prefix match; candidate sum of idf over name/path/community matches | 0.053 / 0.170 | 0.022 / 0.085 | same | yes (base of 2) |
| 2 | kind prior from `[kinds: ...]` (test node or test path x0.15, file/module x0.5, class/type x0.8 unless wanted) and graph prior `1 + 0.3 ln(1+callers)` | 0.151 / 0.489 | 0.032 / 0.128 | same | yes |
| 3 | one-hop expansion: callees in the same file or community that name a term, boosted by the best 8 parents | 0.140 / 0.455 | 0.032 / 0.128 | same | no, reverted (TUNE worse, HOLD-OUT flat) |
| 4 | name-coverage scaling `0.5 + 0.5 x (matched name tokens / name tokens)` | 0.154 / 0.500 | 0.038 / 0.149 | same | yes |
| 5 | slug-tail weight 1.5, group weight 0.5 | 0.126 / 0.409 | not measured (TUNE regressed) | - | no, reverted |
| 6 | same-file discount (0.8 per earlier pick) | 0.126 / 0.409 | not measured (TUNE regressed) | - | no, reverted |
| 6b | class-container tokens of the qualified name as a channel (0.6) | 0.147 / 0.477 | not measured | - | no, reverted |
| 7 | absent check searches with IDF-informative terms only (stricter cutoffs 0.1, 0.03, 0.02, 0.01, 0.007 tried) | 0.154 / 0.500 | 0.038 / 0.149 | 0.1 to 0.02: unchanged 1.00 / 0.083; 0.01 and 0.007: P 0.33 (3 calls), R unchanged | no, reverted (no recall gain; the extra calls were wrong) |

Single-parameter sweeps on TUNE (cutoff 0.03 to 0.15, definition weight 0.25 to 0.8, path weight 0.2 to 0.6, community weight, idf power, callers weight) moved p5 by at most 0.02; the chosen values sit on the plateau.

Final (all six, `scripts/map-score.sh`, release binary): TUNE p5 0.154 / r5 0.500; HOLD-OUT p5 0.038 / r5 0.149; pooled p5 0.109 / r5 0.378 (baseline 0.046 / 0.141). Absent unchanged: precision 1.00 (1 call), recall 1/19.
relate 6/6 ok (but see "relate after the fix": that line checked nothing). Two `map` runs byte-identical. store_hash of `fixtures/typescript` unchanged (8a64f321...). Floors in `FLOORS.json` raised to measured minus 0.01 (pooled 0.0985 / 0.3678).

### Verdict
The stop condition (HOLD-OUT p5 >= 0.30 and r5 >= 0.50) was NOT met; the floors were not lowered, the target stays open. Gains transfer weakly: x3.3 recall on TUNE, x1.75 on HOLD-OUT, and HOLD-OUT is far below TUNE.
Why, from aggregate diagnostics (no per-feature reading of HOLD-OUT): the gold is reachable lexically on TUNE (a gold location is within the top 200 for 65 to 77 percent of locations) but not on HOLD-OUT (top 200: CodeGraph 32, codanna 47, ripwire 30 percent). The HOLD-OUT clones are 2 to 6 times larger (up to 16k functions, 2k test nodes in ripwire) and their implementers are named by words the feature text does not carry (types and classes are 4 of 10 gold locations in ripwire and 4 of 15 in codanna). Names, paths and community keywords are the only text the store holds; docstrings, comments, signatures and call-site neighbours are not stored, so a feature word that is in no node name cannot be found. Next levers sit outside this lane: a text channel (doc/comment tokens) in the store, a synonym or embedding channel, and a larger TUNE set so the formula is not fitted on three small repos.
Absent recall: lexical channels cannot separate `none` from `has` when the implementer uses another word (`visualisation`/`visualization`, `lookup` for `fuzzy-name`): every cutoff that added a call added a wrong one.
Five worst features after (pooled recall@5, all clones): search.embeddings 0/7, graph.type-info 0/5, serving.cli 0/5, symbols.visibility 0/4, registry.multi-repo 0/3 (symbols.test-detection 0/3, graph.inheritance 1/7, search.fuzzy-name 1/6 follow).

### relate after the fix (2026-10-13, fix round)
Reviewers found that `relate` could not say no: a term is informative only when some node token matches it, so every feature with an informative term had a candidate and counted, and "relate 6/6 ok" held for any code base. New rule: a feature counts only when its top candidate's NAME carries a term of source weight 1. `relate --repo X --family code-graph-tools`, score (features counted of 30), fresh workspace per binary:

| clone | main | IMP before fix | after fix (rule 1) | after fix (stricter: name share <= 0.1) |
|---|---|---|---|---|
| codanna | 1.000 | 1.000 | 1.000 | 1.000 |
| CodeGraph | 1.000 | 1.000 | 0.967 | 0.967 |
| code-review-graph | 0.867 | 1.000 | 0.967 | 0.967 |
| graphmind | 0.933 | 1.000 | 0.967 | 0.967 |
| ripwire | 1.000 | 1.000 | 1.000 | 1.000 |
| srclight | 0.833 | 1.000 | 0.967 | 0.967 |
| hono (control, web framework) | 0.867 | 1.000 | 0.933 | 0.933 |
| caddy (control, web server) | 0.733 | 1.000 | 0.900 | 0.900 |

All six gold clones stay `related`; so do both controls (want: below 0.5). The control is NOT separated: the matches are one common feature word that names some function (`code` in `parseIPv6HexCode`, `file` in `saveContentToFile`, `update` in `update`). The stricter variant changes nothing. Kept: rule 1. The map precision and recall are unchanged by this fix.
