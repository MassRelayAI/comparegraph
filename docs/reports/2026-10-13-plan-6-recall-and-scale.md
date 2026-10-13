---
title: Plan 6 re-measure — recall and scale after six lanes
date: 2026-10-13
valid_until: 2027-04-13
lane: plan 6
verdict: gold recall rose to 0.667 on TypeScript (hono), 1.000 on Go (caddy, 48 labelled sites) and 0.834 pooled with pooled precision 0.995 (all targets met), and after lanes D and E the openclaw full build takes 390.6 s against the 8 minute target (met; 499.6 s before them), with impact, traverse and dead-code on its 1.29M-edge graph in 0.75 s, 0.35 s and 2.6 to 4.2 s
status: done
---

Gold measured at main `feef80c`, builds and queries re-measured at `9190b9b` (release builds), 2026-10-13. Full tables: `docs/reports/2026-10-12-engine-parity.md`, section
"After Plan 6 (2026-10-13)". The codegraph-mcp head-to-head was run the same day on the same gold (`scripts/h2h-codegraph.sh`): codegraph-mcp scores 0.708 precision / 0.506 recall pooled against our 0.995 / 0.834, with a graph built in the same time on four of five projects (ours 1.1 to 1.3 times slower, 5.1 times on hono, 0.5 times on graphmind) once its fixed 12 s embedding-model load is excluded; the table, the method and the caveats are in `docs/reports/2026-10-12-engine-parity.md`, section "codegraph head-to-head (2026-10-13)".

## Targets vs measured

| target (plan 6) | measured | met |
|---|---|---|
| TypeScript gold recall >= 0.55 | hono 0.667 (P 1.000), was 0.306 | yes |
| Go gold recall >= 0.60 | caddy 1.000 (P 1.000), was 0.250; 48 labelled sites | yes (small gold) |
| pooled recall >= 0.72 with P >= 0.96 | 0.834 / 0.995 (196 of 197 answered, 196 of 235 sites), was 0.626 / 0.980 | yes |
| openclaw full build < 8 min | 390.6 s (6 m 31 s) at `9190b9b`, 38,258 files, load 11; 499.6 s at `feef80c` before lanes D and E | yes |
| unchanged rebuild unchanged | no-op reuse 2.0 s (hermes-agent), 7.5 s (openclaw); incremental store_hash equals `--full` on both trees | yes |
| product repo < 90 s | not re-measured (the private tree was not part of this run) | open |
| crg through the adapter >= native crg | equal on 3 projects, +0.055 / +0.083 recall on hono / caddy; pooled 0.956 / 0.736 vs 0.954 / 0.711 | yes |

## Per lane (from the commits and `docs/design-notes-for-review.md`)

- A0 `984057b`: `scripts/gold-score.sh` and `tests/gold.rs`, the floor test on the committed labels.
- A1 `3b0bda9`: `describe` blocks are function nodes, module-level test code belongs to the file, JSX `<Foo/>` is a 1.0 rendering call through an import or same-file definition, class-field arrows are method nodes, `foo.bar()` resolves through the declared class and its `extends` chain: hono 0.846 / 0.306 to 0.920 / 0.639.
- A2 `194466c`: Go `go_ctx.rs` (declared types, receivers, package table, `go.mod` imports), the 0.5 name-only Go edge removed: caddy recall 0.25 to 1.0; 40 of 40 sampled edges correct.
- A3 `e256ea3`: barrel re-exports followed, the name fallback binds only a sole candidate outside benchmark/example/test dirs, `new X()` binds only a class: hono precision 0.920 to 1.000.
- A4 / lane D `9aef51c`: repeated limitations collapse into one with a count (hermes-agent 233, openclaw 611 `clamped_line_end`, verified in the envelopes); `44d04a2` edge indexes on open; `c5c77e9` `COMPAREGRAPH_THREADS`.
- Lane E `57bc0f7`: the resolver's per-call class lookup fixed; openclaw resolve_edges 145 s to 59.7 s, hermes-agent 30.3 s to 11.5 s, store_hash unchanged.
- B1 `fb6e9e0`, `41adfa8`: per-pass profiling (`COMPAREGRAPH_PROFILE=1`), indexed lookups, parallel per-file analyses: hermes-agent 194 s to 64.5 s on the B1 binary.
- B2 `feef80c`: incremental edges and communities (affected-file closure, copy of the rest); hash equal to `--full`; at 2,600 files it saves little, on openclaw 289.7 s vs 499.2 s.
- B3 `ce8cb00`, `69f3b3b`, `6994e60`: bounded adjacency loading, deterministic tie-breaks; `impact`/`traverse`/`dead-code` on the 1.29M-edge openclaw graph in 2.5 / 3.6 / 1.6 s at `feef80c`, 0.75 / 0.35 / 2.6 to 4.2 s at `9190b9b` (12 to 15 MB RSS).
- C1 `43b9769`, `bdb0af0`, `fc69ddb`: the external engine host over JSON lines; timeouts kill the process group, a failed engine is a failed run row.
- C2 `a04cff7`, `a25f469`: the code-review-graph adapter, conformance gate per language, `build --providers a,b` with an agreement table (srclight: calls J 0.962, imports 0.904, tests 0.594).

## Method

Readers: two language-model agents labelled each gold site independently under one rubric; the committed `final` is the rows they agree on
(kappa table in the parity report, unchanged: labels did not change). Scorer: `scripts/gold-score.sh` with `scripts/lib/gold.py`, the
smallest enclosing function, method or (for module scope) file node, matching the callee's last segment; precision over answered sites,
recall over repo-target sites. Floors: `fixtures/gold/FLOORS.json`, last measured minus 0.01, raised by lanes and never lowered; all
hold. Hash gate: `store_hash` of an incremental build must equal `build --full` of the same tree (equal on both trees here, and in
18 tests). Timing: ratios between binaries measured back to back, on a shared host (load average 5 to 15 on 40 cores during this run,
37 to 73 on 2026-10-13 morning).

## Limits

- The gold has 150 sites per project (36 to 53 repo-target); caddy has 48, all drawn before the Go work, so 1.000 means "every labelled site", and per-project noise is about 0.1.
- Precision samples were 40 (Go) and 25 edges read by agents against the source, not by humans; the wrong-edge bound at 0 of 40 is under about 7 percent at 95 percent confidence.
- Two readers, both agents; the labelling and the engines were not run blind to each other's authors.
- The hono precision of 1.000 rests on 24 answered sites.
- Build timings ran under load 6 to 15, one run each; the `9190b9b` figures come from a later window than the back-to-back hermes-agent trio (194.4 s pre-Plan 6, 64.5 s B1, 90.5 s `feef80c`), so only the resolve_edges pass figures (30.3 s to 11.5 s) compare directly. The `dead-code` query was slower after lane D (4.2 s against 1.6 s) and is not diagnosed.
- The product-repo build (<90 s target), and the private tree were not measured here; the codegraph head-to-head ran afterwards (see above).
- Query bounds were measured on one hub symbol (3,156 callers), at defaults, one run each.

## What is next

Plan 7 candidates: `docs/superpowers/plans/2026-10-13-plan-7-candidates.md`. From this run: the resolve regression is fixed by lane E
(the 8 minute target is met), the per-file analysis cache (B4) remains the lever for incremental builds (openclaw incremental still
300 s of 390 s, read+parse 166 s), and the `dead-code` timing after the edge indexes should be looked at.
