---
title: Did the 15 field-run asks improve the engine? Measured
date: 2026-11-08
valid_until: 2027-05-08
lane: engine quality
verdict: "No accuracy gain is measured, and none was lost: the call-edge gold, the mapping gold, the conformance fixtures and the store hash are identical before and after. The asks add abilities and remove failure modes (lock waits, comma-split names, silent comment citations). One cost regression was found by this measurement (reach made a claim batch about 100 times slower) and fixed before the report; what remains is 0.15 s per claim with reach (0.01 s before), 0.14 s for a 30-row batch (0.02 s before), and a map about 5 to 10 percent slower."
status: measured
---

# Did the 15 field-run asks improve the engine?

## What is compared

- **Before:** commit `ef556eb` (main before any of the field-run work), release build.
- **After:** `main` at the merge of `fix/reach-cost` (the 15 asks plus the reach-cost fix), release build.
- Host: Linux x86-64, shared machine. Timings are three runs each on a fresh workspace; the table gives the range.
- Clones: the pinned study copies in `../comparegraph-study`. Gold files: `fixtures/gold` and `fixtures/gold-map`.
- Tools: `scripts/gold-score.sh`, `scripts/map-score.sh`, `comparegraph conform reference --fixtures fixtures`, wall time by
  `date +%s.%N`. The 30 claim rows are `has` claims on 30 real `fn` lines of codanna (generated; not the field run's data).

## Accuracy: no difference

| check | before | after |
|---|---|---|
| call-edge gold, 5 projects (caddy, hono, graphmind, code-review-graph, srclight): precision and recall | 1.0/1.0, 1.0/0.667, 0.973/0.75, 1.0/0.925, 1.0/0.78 | identical, output byte-identical |
| mapping gold `map` p5 / r5, 5 clones (graphmind, srclight, code-review-graph, codanna, ripwire) | 0.167/0.588, 0.173/0.543, 0.148/0.472, 0.05/0.2, 0.05/0.2 | identical |
| absent-check on the gold (graphmind) | precision 1.0, recall 0.125 | identical |
| conformance fixtures (`conform reference`) | all fixtures | identical |
| `build` store hash on codanna (797 files) | `a6f3b565078576f5` | identical (determinism holds) |

`CodeGraph` is not in the mapping run: its clone folder is lower-case and resolves no gold rows by name, so five of six
clones are scored.

## `map` does change, and the gold cannot see it

On codanna, over the 30 features of the code-graph-tools family, the top 5 candidates are the same in the same order for 14
features and differ for 16; the top candidate differs for 7. The gold labels only a few locations per feature, so a change of
ordering inside the top 5 or of what fills it does not move p5 or r5. This report therefore cannot say whether those 16
changes are better or worse. The reach signal (entry-point reached or not, x0.5 for an unreached called symbol) is a
structural prior; the one fixture where it is shown to help (`run_loop` over `_get_running_loop`) is synthetic. The real
openai-agents, OpenHands, goose and langgraph cases from the field run were not re-run (no clones here).

The community signal was tried as a score and made the gold worse (pooled r5 0.451 to 0.416), so it is reported, not scored
(`docs/decisions.md`).

## Cost

| step on codanna | before | after the fix | note |
|---|---|---|---|
| `build` (797 files) | 6.99 / 7.17 / 7.05 s | 6.93 / 7.06 / 6.87 s | unchanged |
| `claim --batch`, 30 rows | 0.024 / 0.019 / 0.022 s | 0.142 / 0.167 / 0.180 s | reach on every has |
| `claim --batch`, 300 rows | not measured on the baseline (the merged-before-fix build took 22 s) | 0.39 to 0.41 s | measured by the fix lane |
| single `claim` | 0.009 / 0.011 / 0.010 s | 0.159 / 0.143 / 0.150 s | reach built once per process |
| single `claim --no-reach` | n/a | about 0.01 s | escape hatch |
| `map`, 30 features | 0.576 / 0.507 / 0.568 s | 0.571 / 0.582 / 0.585 s | within noise to about +10 percent |

Found by this measurement: the first merged version computed reach per claim and took 2.5 to 2.7 s for the 30 rows (about
90 ms per claim; 22 s for 300 rows). At the field run's scale of 4,400 claims that is minutes added to a verb that was made
fast on purpose (`docs/reports/2026-10-14-answer-size.md`). The fix builds the reach state once per run for a batch
(`docs/reports/2026-11-08-reach-cost.md`); the reach of every row is byte-identical to the per-row build.

## What the asks change, by kind

- **Failure modes removed (no number measured, shown by tests that were seen red):** lock waits (3), comma/space/newline
  names in `--read-hits` (2), a refusal that carried its list only in prose (4), a bare `path:1` or a comment cited as
  evidence (14), a claim with no way to carry its commit, anchor or out-of-subset path (7, 8, 15).
- **Abilities added:** `--include/--exclude` and `--moved-to` (6), `family propose --from-claims` (9), `--expect` (11),
  licence/lane overrides (13), `reach` and `matrix --reach` (5), shim marking in `map` (10).
- **No change needed:** 12 (no truncation site in the engine).

## What this does not show

- Whether a reader on the field run's task now finishes with fewer dead ends. That needs the same task run again with the
  new binary; it was not done.
- `reach` quality: codanna reports 1,950 entry points for 797 files, a generous heuristic; 26 of 30 test claims came out
  `reached`. A tool registered through an uncalled helper reads as reached.
- Shim marking on a real ADK clone, and JS/TS barrel files (the provider writes no edge for them).
- The hand-written regex subset behind `--anchor` and `--expect` against real patterns (no `regex` dependency without a
  ruling).

## Verdict

Keep all of it: nothing measured regressed accuracy or determinism, and the abilities close real gaps. Do not claim an
accuracy gain: there is none in the gold. The next measurement worth doing is the field task re-run with the new binary,
which is what would show whether readers lose less time.
