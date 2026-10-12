---
name: mapping-features
description: Use to find candidate evidence for a family's features in a built clone and to score how related a clone is to a family. Candidates are leads, never verdicts.
---
# Mapping features

## When
A family is loaded and a clone is built; you want to know where to read, and whether the clone belongs to this family at all.

## Commands
```
comparegraph --root <ws> map <repo> <family> [--feature <id>] [--limit 5]
comparegraph --root <ws> relate [--repo <r>] [--family <f>]
comparegraph --root <ws> relate --stored [--repo <r>] [--family <f>]
comparegraph --root <ws> search "<terms>" --repo <r>
comparegraph --root <ws> show <repo> "<qualified_name>" --detail full
```
`map` writes nothing. For each active feature it returns `terms` (tokens of group, slug and title; the definition is not used),
`symbols` (full-text hits, each labelled `candidate`) and `communities` (clusters whose label or keywords carry a term).
`relate` scores the share of active features with at least one symbol or community candidate:
`related` at or above 0.5, `adjacent` at or above 0.2, else `unrelated`. It records a row keyed by repo, family, run and pack hash.

## What the engine refuses
- unknown family or repo: `not_found`; unbuilt repo: `not_found` "build it first"
- `map --feature <id>` where the id is retired or absent: `not_found`
- `relate --stored` computes nothing; pairs never scored are absent from `data.items`

## Verify
- `data.items[].band`, `.score`, `.basis` (ids that had a candidate). Editing the pack and running `relate` again adds a
  new row without a rebuild; `--stored` returns the row for the current pack.
- Treat every candidate as a place to read. Confirm with `show --detail full` before claiming (see writing-a-claim).
- A `related` band proves vocabulary overlap, not that a feature is present.
