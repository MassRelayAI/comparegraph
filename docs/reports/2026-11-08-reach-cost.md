---
title: The cost of reach in a claim batch
date: 2026-11-08
valid_until: 2027-05-08
lane: fix/reach-cost
verdict: "Sharing the run's reach state across a batch brings a 30-row `claim --batch` on codanna from 2.54-2.65 s back to 0.12-0.15 s and a 300-row batch from 22.1-22.5 s to 0.39-0.41 s, with every row's reach, limitations and confidence identical to the per-row build (630 rows compared (30 + 300 + 300), on ok, reach, limitations, confidence and message). A single claim is unchanged at 0.13-0.14 s."
status: measured
---

# The cost of reach in a claim batch

## What was wrong
The reach hook (`application/reach.rs`, from the field run's ask 5) rebuilt, for every has/partial claim, the run's
referenced set, its entry-point list and an empty walk cache, then walked up to 2,000 nodes from every entry point. A batch
paid that once per row: about 85 ms a row on codanna (797 files, 1,931 entry points).

## The change
`graph::ReachCache`, keyed by (run, depth, max-nodes, min-confidence), holds the sorted entry points, the walk's node and
adjacency caches, and the walk from every entry point. `claim --batch` keeps one for all its rows (the batch holds its write
transaction, so no build lands between rows; a new run is a new key). A claim whose target is an entry point only because
nothing calls it leaves that target out of the starts, as before: it walks again, on the cached adjacency. A single `claim`
builds the state once. `--no-reach` (batch rows `no_reach: true`) skips the walk and records no reach.

## Numbers
Release builds, `origin/main` at the branch point ("old") against `fix/reach-cost` ("new"), each on a fresh workspace with
`codanna` built and `families/code-graph-tools` added. Wall time from `/usr/bin/time`, three runs each.

| run | old | new |
|---|---|---|
| `claim --batch`, 30 has rows (30 ok) | 2.65 / 2.53 / 2.54 s | 0.13 / 0.15 / 0.12 s |
| `claim --batch`, 300 rows (the 30 with `at` lines shifted 0..9; 258 ok, 42 refused as comment or blank lines, 30 skipped as equal) | 22.53 / 22.21 / 22.14 s | 0.41 / 0.39 / 0.40 s |
| single `claim ... has` | 0.14 / 0.13 / 0.13 s | 0.14 / 0.13 / 0.14 s |
| single `claim ... has --no-reach` | (no flag) | 0.00 / 0.00 / 0.00 s |

Every row of the 30-row and both 300-row outputs was compared between old and new on `ok`, `reach`, the limitations,
`confidence` and the error message: 0 differences (26 reached and 4 unreached in the 30; 226 and 32 in the 300).

## Guards
`tests/claim_reach_cost.rs`: a counter of reach-state builds per thread must read 2 (two repos) for both a 10-row and a
40-row batch; it read 9 for 10 rows on the old code and with the cache forced off. The cached batch must equal the
cache-off batch row by row over a fixture that reaches every outcome (reached, unreached, uncalled target, name-guessed,
file scope, truncated at 2,000 nodes); reusing the all-entries walk for an uncalled target turned it red. A rebuild
between two batches gives a new run and a new reach.
