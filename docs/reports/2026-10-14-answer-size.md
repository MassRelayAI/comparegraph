---
title: Answer size on six study copies
date: 2026-10-14
valid_until: 2027-04-14
lane: readme front door
verdict: "For 30 features on six code-graph tools, the `map` answers total 335,755 bytes (about 84 k tokens, an estimate) against 106,057,965 bytes of source (about 26.5 M tokens): 316 to 1. The answers are ranked places to read, not verdicts."
status: measured
---

# Answer size on six study copies

## What is measured
The size of what an agent reads to start a comparison, against the source it would otherwise open. Binary `comparegraph 0.1.1`
(debug build), family `families/code-graph-tools` (30 features), six study copies. Source bytes are files with a source
extension (rs py ts tsx js mjs go c cc cpp h hpp java kt swift rb php) outside `.git`, `node_modules`, `vendor`,
`third_party` and `target`. Answer bytes are the JSON envelope each verb prints. Tokens are bytes / 4, an estimate.

| clone | source bytes | `map` answer bytes | `map` seconds |
|---|---|---|---|
| graphmind | 1,554,166 | 57,558 | 0.76 |
| srclight | 1,379,465 | 47,671 | 0.64 |
| code-review-graph | 6,465,145 | 55,441 | 2.08 |
| codanna | 7,764,215 | 57,483 | 3.09 |
| CodeGraph | 71,696,190 | 64,496 | 3.73 |
| ripwire | 17,198,784 | 53,106 | 3.90 |
| **total** | **106,057,965** | **335,755** | |

`matrix code-graph-tools` is 5,703 bytes with no claims recorded; `relate` is 5,789 bytes. The ratio of source to `map`
answers is 316 : 1 (106,057,965 / 335,755). The raw numbers are in [`docs/assets/cost.json`](../assets/cost.json); the
picture in the README is rendered from that file.

## The caveat
The `map` answer is a ranked list of places to read, not a verdict. Held-out recall@5 is 0.149
([mapping gold, first measurement](2026-10-14-mapping-gold-first-measurement.md)), so the reader still opens files. What the
numbers show is the size of what an agent reads to start. What the workflow keeps is a matrix whose cells cite lines that
`check` re-verifies. Source bytes count every source file, including tests and generated code; an agent would not read all of
it, so the ratio is an upper bound on the saving, not a typical one.

## Reproduce
```bash
scripts/answer-size.sh <workspace> families/code-graph-tools <clone>... > docs/assets/cost.json
node scripts/readme-assets.mjs
```
Map answers and source bytes are deterministic for one binary and are equal on a re-run; timings vary by a few percent.
`node scripts/readme-assets.mjs --check` (and CI) fails when `cost.svg` is not the render of `cost.json`.
