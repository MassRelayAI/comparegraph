---
title: Time to a first result, on the sample and on real clones
date: 2026-10-24
valid_until: 2027-04-24
lane: readme front door
verdict: "The README's sample walk (add, build, family add, map, matrix on fixtures/typescript) takes under 0.3 s of tool time; on real clones the build dominates, 0.82 s for 85 parsed files (srclight) to 6.20 s for 797 (codanna), and a three-feature `map` answers in under 0.1 s. `family propose` drafts 60 shared words in 0.05 s, which are words, not features: a person still writes the pack."
status: measured
---

# Time to a first result

## What is measured

How long a reader waits, after installing, before CompareGraph prints a first answer: on the shipped sample, and on clones
of real projects. Wall time of each command as `date +%s.%N` around it. Binary: a release build (`cargo build --release -p
comparegraph`) of commit `9e0ab8d`, which reports `comparegraph 0.1.1`. Host: Linux x86-64, 40 cores, load average about 4,
shared. Three runs each; the table gives the range, and the median where one number is quoted. Network time is not
included: the clone of this repository was made from a local copy (0.26 s to 0.28 s), and the study clones were already on
disk.

## On the shipped sample

The commands of the README's "See it work on the sample", in a fresh workspace each run.

| step | run 1 | run 2 | run 3 |
|---|---|---|---|
| `add ../comparegraph/fixtures/typescript --name tsfix` | 0.04 s | 0.04 s | 0.04 s |
| `build tsfix` (3 files) | 0.03 s | 0.16 s | 0.03 s |
| `family add ../comparegraph/families/example-code-graph-tools` | 0.01 s | 0.01 s | 0.01 s |
| `map tsfix example-code-graph-tools` | 0.01 s | 0.01 s | 0.01 s |
| `matrix example-code-graph-tools` | 0.01 s | 0.01 s | 0.01 s |

The `claim` step was run once, with the same output as in [Usage](../usage.md#quick-start-on-the-shipped-sample); the
matrix then shows `has` for `symbols.extraction` and `-` for the other nine features.

## On real clones

Study copies at their pinned heads, each built once into a fresh store per run (`build`; `build --full` for hono and codanna).

| clone (head) | language | files parsed | nodes | build, three runs | median |
|---|---|---|---|---|---|
| srclight (`23924ff`) | Python | 85 | 1,789 | 0.90 s, 0.78 s, 0.82 s | 0.82 s |
| code-review-graph (`6b12d11`) | Python | 332 | 8,183 | 3.62 s, 3.27 s, 3.31 s | 3.31 s |
| hono (`552ea02`) | TypeScript | 434 | 7,052 | 4.76 s, 4.49 s, 4.37 s | 4.49 s |
| codanna (`639917d`) | Rust | 797 | 10,011 | 6.55 s, 6.20 s, 6.16 s | 6.20 s |

The build's own `seconds` field reads a few hundredths of a second lower than the wall time; the report quotes the wall time.

After the build, a hand-written pack of three features (full-text search, embedding search, module communities) loads with
`family add` and `map` answers in 0.09 s on codanna (5,810 bytes) and 0.07 s on hono (4,687 bytes). On code-review-graph the
top three `map` candidates for module communities include `code_review_graph/communities.py::detect_communities`, the
location the mapping gold also cites; candidates are places to read, not verdicts (held-out recall@5 0.149 in the
[mapping gold report](2026-10-14-mapping-gold-first-measurement.md)).

## `family propose`

`family propose tools --repos crg,srclight` on the two Python clones takes 0.04 s to 0.05 s and writes
`families/tools/rows.draft.md` with 60 rows (the `--max` default). The rows are the words both clones share most, such as
`name`, `file`, `extract`, `resolve`, `path`, with `draft: define` in place of a definition. It is a list to pick from, not a
pack: the person who knows the field keeps a few, defines them and saves the file as `rows.md`.

## What this does not say

The time a person spends writing the pack and reading candidates is not measured; it is the larger part of a real
comparison. Larger trees take longer: the build table in the README (hermes-agent 93.2 s, openclaw 499.6 s) comes from the
[Plan 6 report](2026-10-13-plan-6-recall-and-scale.md). One host, at low load, three runs.

Reproduce: the commands above with `time`, on clones of the same heads.
