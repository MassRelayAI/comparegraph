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
- `none` without `--searched`, or when the engine's own search for those terms finds a hit in that repo (the message lists examples)
- a retired feature ("feature <id> is retired")
- an id present in several families without `--family`
- unknown repo or feature: `not_found`

## Verify
- `data.verdict`, `data.author`, `data.locations[]` with `path`, `line_start`, `line_end`, `content_hash`, `node_id`.
- `evidence.confidence` is `derived` (a located claim) or `unknown`; never `observed`.
- A second claim for the same repo and feature has `supersedes_id`; `claims --history` shows both.
- If a cited file changes, `comparegraph check` marks the location stale and the matrix shows `unknown*` for that cell.
Never cite lines you have not read.
