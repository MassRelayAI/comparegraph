---
name: writing-a-claim
description: Use to record what a clone does or does not have for one feature, with citations that resolve now. Claims are append-only; the latest wins.
---
# Writing a claim

## When
You have read the cited lines yourself and can say has, partial, none or unknown for one feature in one clone.

## Commands
```
comparegraph --root <ws> claim <repo> <feature> has|partial --author <who> --at <path>:<start>-<end> [--at ...] [--note "..."] [--family <f>]
comparegraph --root <ws> absent-check <repo> <family> <feature>
comparegraph --root <ws> claim <repo> <feature> none --author <who> [--note "..."]
comparegraph --root <ws> claim <repo> <feature> none --author <who> --searched "<terms>" [--note "..."]
comparegraph --root <ws> claim <repo> <feature> unknown --author <who>
comparegraph --root <ws> claims [--repo <r>] [--family <f>] [--feature <id>] [--history]
```
Paths are relative to the clone. `<path>:<line>` cites one line. Over MCP, `comparegraph_claim` takes the same fields (`at` is an array, `family` optional).

## What the engine refuses (`invalid_arguments` unless noted)
- no `--author`; `has`/`partial` without `--at`
- a path that is absolute, contains `..`, is missing, is not a file, or resolves outside the clone (symlink)
- lines out of range (need 1 <= start <= end <= file length)
- Run `comparegraph check` first: a location becomes stale only when `check` runs, so a claim read before it can look current.
- `--searched` matches whole identifier tokens by prefix (`retry` finds `retry_with_backoff`, `backoff` does not); search several stems. Text with no searchable token is refused.
- `none` without `--searched` when the graph has candidates on any of four channels (symbols, paths, community labels/keywords, node kinds the feature names); the message lists them. Read them, then claim has/partial, or record a manual absence with `--searched`
- `none` over MCP without `searched` (the MCP server never records a graph-backed absence)
- `none` with `--searched` when the engine's own search for those terms finds a hit in that repo (the message lists examples)
- a retired feature ("feature <id> is retired")
- an id present in several families without `--family`
- unknown repo or feature: `not_found`

## Verify
- `data.verdict`, `data.author`, `data.locations[]` with `path`, `line_start`, `line_end`, `content_hash`, `node_id`.
- `evidence.confidence` is `derived` (a located claim) or `unknown`; never `observed`.
- A second claim for the same repo and feature has `supersedes_id`; `claims --history` shows both.
- If a cited file changes, `comparegraph check` marks the location stale and the matrix shows `unknown*` for that cell.
Never cite lines you have not read.

## Graph-absent or manual
`none` without `--searched` is the default: the engine searches the feature's terms itself and, only if all four channels
are empty, records `derived` with the terms and the four counts in the note (`graph-absent: terms=...; channels=symbols:0,paths:0,communities:0,kinds:0`).
Pass `--searched` when you disagree with the graph (it refuses on a path substring like `add` in `address`) or know a
synonym the terms lack: that records a manual absence with your terms. There is no force flag. Absence is only as good as the terms: a clone that names the thing differently still reads as absent.
