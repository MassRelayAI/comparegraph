---
name: reading-the-vault
description: Use when you need the whole comparison in readable form - the comparison matrix, the markdown vault and the JSON export - and want to read gaps and differentiators correctly.
---
# Reading the vault

## When
Claims exist and someone needs to read or hand over the result; or you want to browse a clone's graph as linked notes.

## Commands
```
comparegraph --root <ws> matrix <family>
comparegraph --root <ws> export vault --out <dir>
comparegraph --root <ws> export json [--repo <r>]
comparegraph --root <ws> claims --history
comparegraph --root <ws> check
```

## Reading the matrix
- Rows are the active features of the family (retired ones are gone); columns are every clone, with its role, its relatedness
  band for the current pack and its latest repo-scope decision.
- A cell is the latest claim's verdict: has, partial, none, unknown, or `-` when nobody has claimed it.
- A location becomes stale only when `comparegraph check` runs: run `comparegraph check` first, then read.
- `unknown*` means the claim's cited lines changed since it was written (stale); re-read and re-claim.
- `prevalence` counts comparators only (the root is excluded): the share that has or partially has the feature.
- `gap` marks a feature comparators have and the root lacks; `differentiator` marks one the root has and comparators lack.
  Both are null when there is no root.

## What the engine refuses
`matrix` on an unknown family is `not_found`. `export vault` regenerates the directory whole: point `--out` at a directory
that holds nothing you wrote by hand.

## Verify
- `data.comparators`, `data.columns[]`, `data.rows[].cells`.
- Two vault exports of the same store are byte-identical; diff them to prove nothing drifted.
- The vault has `index.md`, `llms.txt`, `AGENTS.md` and the folders `repos`, `symbols`, `communities`, `families`, `features`.
- Run `check` first: `data.stale_locations` should be 0, or the matrix will show `unknown*` cells.
