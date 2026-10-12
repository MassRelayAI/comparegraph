---
name: using-comparegraph
description: Use first when comparing cloned repositories with CompareGraph - it says which of the method skills applies, the order of the workflow, and the two habits that keep verdicts honest.
---
# Using CompareGraph

CompareGraph parses clones into one SQLite code graph and compares them against taxonomy packs. The engine never calls a
model and never decides: you read the code, you write the claim, a person records the decision.

## Once per machine
```
comparegraph setup --client claude --apply
```
Without `--apply` it only prints its plan. It writes additively and leaves a timestamped backup. `comparegraph doctor`
reports whether the registration is `aligned` or `stale`.

## Which skill, when
| situation | skill |
|---|---|
| a repo should become a comparator or the root | `adding-a-clone` |
| no taxonomy yet, or a pack needs editing | `defining-a-family` |
| where to read for a feature; is this clone even in the family | `mapping-features` |
| you have read the lines and can say has / partial / none / unknown | `writing-a-claim` |
| someone needs the whole result, gaps and differentiators | `reading-the-vault` |
| a person said adopt, reject or park | `recording-a-decision` |

## Workflow
1. relate: `relate <repo> <family>` - is the clone related at all (a band, not a verdict).
2. map: `map <repo> <family>` - candidate places to read per feature. Candidates are leads, never evidence.
3. claim: read the code, then `claim` with citations that resolve now.
4. matrix: `matrix <family>` - prevalence, gaps, differentiators.
5. decide: only on a person's instruction, with their name and reason.

## Habits
- Run `comparegraph check` before trusting a verdict; stale locations show as `unknown*` in the matrix.
- Compact answers carry ids and `file:line` only. Read a body with `show <repo> "<qualified_name>" --detail full`.
- Read every answer's `limitations`: dynamic dispatch, config-driven wiring and unsupported languages are invisible.
- A `none` needs a `--searched` stem; try several. Clones are read, never run.
