---
name: defining-a-family
description: Use when a taxonomy of features is needed - write a pack (rows.md), seed one from the corpus with family propose, load it with family add, and re-load it safely after edits.
---
# Defining a family

## When
Before mapping or claiming anything: a family is the list of features every clone is judged against.

## Commands
```
comparegraph --root <ws> family propose <name> --repos a,b,c [--out <dir>] [--min-repos 2] [--max 60]
comparegraph --root <ws> family add <pack-dir>
comparegraph --root <ws> family list
comparegraph --root <ws> family show <name>
```
`propose` writes only `rows.draft.md` (terms shared by at least `--min-repos` clones, with counts). It never writes `rows.md`
and never touches the store. A person edits the draft into `rows.md`: merge terms into real features, write definitions.

## Pack format
`<pack-dir>/rows.md`: front matter (`family:`, `description:`) then a table with columns
`id | group | feature | definition | counts_as_has | tier | weight`. `id` is `<group>.<slug>` (lowercase `[a-z0-9-]`, one dot);
`tier` is basics|common|extra|candidate; `weight` 1..3. See `families/example-code-graph-tools/rows.md`.
`counts_as_has` is the test a citation must pass; write it so two readers would agree.

## What the engine refuses
`family add` writes nothing (one transaction) and names the row when: a feature id repeats, a definition, title or
`counts_as_has` is empty, the id is malformed, the tier or weight is out of range, the table is missing or has the wrong header.
`propose` refuses an empty `--repos`, an unbuilt repo, or a name outside `[a-z0-9-]`.

## Verify
- `family add`: `data.added` lists new ids; `data.retired` lists features the pack no longer has. Retired features keep
  their claims, leave the matrix, and refuse new claims.
- `family show <name>`: features grouped, tiers as written.
- `propose`: `data.candidates` (count), `data.draft_path`, `data.rows_md_exists` (true means you already have a pack there).
