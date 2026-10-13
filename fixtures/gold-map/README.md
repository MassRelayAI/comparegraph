# Gold feature-mapping labels (Plan 7, lane 1)

For each (feature, clone) pair: does the clone implement the feature? Features come from `families/code-graph-tools/rows.md`
(30 features; the node kinds that would implement each are in the `[kinds: ...]` suffix of its definition). Clones are the six
study copies of code-graph tools, at pinned heads: graphmind, srclight, code-review-graph, CodeGraph, codanna, ripwire.
`pairs.jsonl` holds the 180 pairs, one per line, in a fixed order (clone, then pack row); regenerate it with
`scripts/gold-map-pairs.sh` (deterministic). Row shape:
`{id: "map-<clone>-<nnn>", clone, feature_id, title, definition, counts_as_has, kinds}`.
Labels land in `fixtures/gold-map/<clone>/features.jsonl` as
`{id, reader1, reader2, final}` where each reader is `{verdict, locations, searched, note}` and `final` is `null` where they disagree.

## Verdict

- `has`: the `counts_as_has` test is met by code in the clone.
- `partial`: a clear part of it is met; the note says which part and which part is missing.
- `none`: searched and absent. `searched` lists the terms and paths searched (grep terms, directories); a bare "not found" is invalid.
- `unknown`: cannot be decided from source (generated code, a vendored binary, behaviour that exists only at run time).

## Locations

- Form `path::symbol` (preferred) or `path:line`; paths relative to the clone root at the pinned head.
- At most 3 per pair, for `has` and `partial` only; `none` and `unknown` carry none.
- Each location points at the implementation (the function, struct or module that does the work), not at a mention in the README,
  a comment, a test name or a config key that nothing reads.

## Method

- Readers work independently, from the clone's source, and never from the engine's output (`map`, `relate`, search hits, claims).
  Reader 2 does not see reader 1's labels.
- Apply `counts_as_has` literally; do not credit a plan, a TODO or a stub. A feature behind a build flag that is off by default
  still counts as `has`, with the flag named in the note.
- Docs and the README may tell where to look, never settle the verdict.

## Final label

`final` is the agreement of both readers on the verdict, and, for `has` and `partial`, on at least one location (same
`path::symbol`, or the same file and lines within 10 of each other). Anything else is `final: null`, kept in the file and left out
of the scores. Disagreements are not adjudicated into the gold.

## Scoring

The engine's mapping is scored against `final` per pair (verdict match, location hit for has/partial). Floors are in `FLOORS.json`,
set after the first measured run and raised, never lowered.
