# Reports

Every measurement, proof and bake-off ends in a dated file here, with `date`, `valid_until`, `verdict` and `status` in its
front matter. The README quotes numbers only from these files. A report past its `valid_until` is re-run or marked stale,
never silently kept.

| date | report | verdict in one line | status |
|---|---|---|---|
| 2026-10-14 | [Mapping gold, first measurement](2026-10-14-mapping-gold-first-measurement.md) | `map` started at the level of plain full-text search (pooled precision@5 0.046, recall@5 0.141); the improvement task raised it to 0.109 / 0.378 pooled, but on the three held-out clones only 0.038 / 0.149 against the target 0.30 / 0.50; the absent check scores precision 1.00 on one call and recall 1 of 19 | measured, target open |
| 2026-10-13 | [Install surfaces across the study projects](2026-10-13-install-surfaces.md) | npm is one registry of three in use; the recurring surfaces are an install script, a Homebrew tap, an MCP Registry manifest, a one-command agent setup with skills, hooks and a rules line, and a Claude Code plugin; we lack the script, the manifest, the tap, hooks and the rules line | decided; registry manifest and install scripts shipped 2026-10-14, crates.io dropped |
| 2026-10-13 | [Proof on a real product](2026-10-13-proof-on-a-real-product.md) | the pipeline as root over a private product monorepo with three harness comparators: builds (103 s to 436 s), claims, matrix, and after Plan 7 lane 1 the ranked `map` read against the source (8 of 12 product features right in the top three, one noise rule) and the join-order defect it found | measured, re-run 2026-10-13 |
| 2026-10-13 | [Plan 6: recall and scale](2026-10-13-plan-6-recall-and-scale.md) | call recall on the gold standard 0.667 TypeScript, 1.000 Go, 0.834 pooled at precision 0.995; a 38k-file repository builds in 390.6 s | done |
| 2026-10-12 | [Engine parity with code-review-graph](2026-10-12-engine-parity.md) | the head-to-head: Jaccards by kind, the gold standard, the codegraph comparison, the feature-mapping baselines; sections are appended after each plan | decided |
| 2026-10-12 | [Plan 1 proof on three real clones](2026-10-12-plan-1-proof.md) | the reference provider builds three real clones deterministically with a clean check and a faithful export | decided |

Reproduce a number: the scripts are described in [`../dev/scripts.md`](../dev/scripts.md); the labelled data is under
`fixtures/gold/` (call edges) and `fixtures/gold-map/` (feature mapping), each with its own README and `FLOORS.json`.
