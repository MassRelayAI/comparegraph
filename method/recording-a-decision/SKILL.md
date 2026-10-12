---
name: recording-a-decision
description: Use only when a person has said to adopt, reject or park a clone or one of its features - record their decision with their name and reason. The engine never decides on its own.
---
# Recording a decision

## When
A human gave an instruction ("adopt their retry loop", "reject this repo"). An agent records it; it does not originate it.
There is no MCP decide tool by design: a decision is a CLI act.

## Commands
```
comparegraph --root <ws> decide <repo> adopt|reject|park --why "<reason>" --by <person>
comparegraph --root <ws> decide <repo> adopt|reject|park --why "<reason>" --by <person> --feature <id> [--family <f>]
comparegraph --root <ws> decisions [--repo <r>] [--history]
```
No `--feature` records a decision about the whole clone (scope `repo`); with it, about one feature of that clone (scope `feature`).
`--family` is needed only when the feature id exists in several families.

## What the engine refuses
- a disposition other than adopt|reject|park, an empty `--why`, an empty `--by`: `invalid_arguments`
- an unknown repo or feature: `not_found`
- an ambiguous feature id without `--family`: `invalid_arguments`

## Verify
- `data.scope`, `data.disposition`, `data.decided_by`, `data.supersedes_id`.
- Deciding again on the same scope (same repo, or same repo and feature) keeps both rows; the new one points at the old via
  `supersedes_id`. `decisions` shows the latest per scope, `decisions --history` shows every row.
- The matrix column for the repo shows the latest repo-scope decision.
Quote the person's words in `--why` when you have them.
