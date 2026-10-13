# Plan 7, lane 1 — a gold standard for feature mapping, and a graph-backed absent verdict

**As of 2026-10-13.** The maintainer: "do the mapping gold + absent verdict first, push when done (auto improve as
necessary); test them with all cases and head to head with other OSS, and a real product at the end to check the outputs."

## Why first
The engine (nodes, edges) has a committed gold standard, floors and a head-to-head. The layer users read — `relate`, `map`,
claims, the matrix — has none: the proof on a real product found `relate` scoring everything `related` and `map` carrying
substring noise. Nothing above the engine can be called good until it is measured the same way.

## Targets
- A committed gold set `fixtures/gold-map/<clone>/features.jsonl`: for each (feature, clone) pair a verdict `has | partial |
  none` and, for `has`/`partial`, the locations (`path::symbol` or `path:line`) that implement it; two independent readers,
  rubric in `fixtures/gold-map/README.md`, `final` only where both agree, disagreements kept but excluded.
- A scorer `scripts/map-score.sh` (+ `scripts/lib/goldmap.py`) and a floor test `tests/map_gold.rs` (skips loudly without
  `COMPAREGRAPH_STUDY`): `map` precision@5 and recall@5 of gold locations per feature; `relate` band vs gold `has` per
  (clone, family); the absent verdict's precision (never "absent" where the gold says `has`) and recall over gold `none`.
- Measured floors committed in `fixtures/gold-map/FLOORS.json` after the first honest run; raised by later tasks, never lowered.
- Targets after the improvement task: map recall@5 ≥ 0.70 at precision@5 ≥ 0.60; absent precision ≥ 0.95 (a wrong "absent"
  is the costly error) at recall ≥ 0.60.

## Lanes
- **G0 — the feature list and rubric.** Extend `families/example-code-graph-tools/rows.md` (or a new pack
  `families/code-graph-tools/`) to ~30 features a code-graph tool can have, each with a one-line definition and the node kinds
  that would implement it (function, class, config key, test, CLI command, MCP tool). The five study copies used by the call
  gold are the clones. A sampling script lists the (feature, clone) pairs in a fixed order with ids `map-<clone>-<nnn>`.
- **G1 — two readers.** Two independent agents read each clone's source (never the engine's output) and label every pair per
  the rubric; `final` = agreement; kappa recorded. Locations must resolve (`comparegraph show` on a fresh build), or the row
  is `unresolved` and excluded.
- **ABS — the absent verdict.** `claim … none` gains evidence from the graph: the engine searches the feature's terms over
  symbol names, paths, community labels and keywords, and the kinds the feature names; zero hits on every channel → evidence
  class `derived` with note `graph-absent: <channels searched>`; any hit → the claim is refused as `none` and the hits are
  shown (the reader must look). `--searched` stays as the manual fallback and is recorded as `searched`. MCP exposes the
  check read-only (`absent-check`), never the claim.
- **IMP — improve map/relate against the gold.** After the first measurement: rank candidates by call-graph proximity and
  community co-membership in addition to term hits; path-segment terms; test names as evidence; `relate` bands from the share
  of features with a top-5 hit, not from an OR over all terms. Every change is measured before→after on the gold.
- **H2H — head to head.** No OSS graph tool maps features to code; the fair baselines are (a) plain full-text symbol search over
  the feature terms (our own FTS, no ranking), (b) codegraph's `codegraph_symbol_search` / `search_by_pattern` fed the same
  terms, scored with the same scorer. Table in the parity report.
- **PROD — the real product.** Build the private product monorepo as root with the proof's comparators, run `map` and the
  absent check for the proof's family, and read every output against the source: a dated block in the proof report
  (wording: "a private product monorepo"; no paths).

## Review focus
1. A gold location that the engine cannot resolve is excluded, never counted as a miss of the engine.
2. The absent verdict must never be computable from a model: channels, counts and the searched terms are in the note.
3. `relate`/`map` changes keep store_hash identical (they read, never write the graph).
4. The scorer's rule is symmetric for every baseline (same enclosing/location normalisation as `gold.py`).

## Rollback
Every lane is its own worktree and branch; the gold files are data; the absent verdict is a new code path behind the
`none` verdict; `map`/`relate` changes are measured and reverted if the gold falls.
