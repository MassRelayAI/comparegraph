---
name: adding-a-clone
description: Use when a repository should become a comparator or the root in a CompareGraph workspace - register it, parse it, verify the build. Clones are read, never run.
---
# Adding a clone

## When
A cloned repo is to be compared against a taxonomy. The clone must already be on disk; CompareGraph never fetches or executes it.

## Commands
```
comparegraph --root <ws> add <path-to-clone> --name <name> [--as-root]
comparegraph --root <ws> build <name>
comparegraph --root <ws> check
comparegraph --root <ws> repos
```
`--as-root` marks your own product (the baseline); every other clone is a comparator. Root is excluded from prevalence.

## What the engine refuses
- a path that does not exist: `io_error`. A folder that is not a git clone is accepted with null head and licence and lane `study`; check `data.head` is set
- `build` on a name never added: `not_found`
- a build that parses zero files: `empty`, and the run is marked failed (nothing half-written is served)
- every later verb on an unbuilt repo (`map`, `relate --repo`, `claim`): `not_found` "has no finished run; build it first"

## Verify
- `add`: `data.licence` and `data.lane` (take|study|...), `data.head` is the commit read. Check the lane before porting any code.
- `build`: `data.files_parsed`, `data.edges`, `data.communities` are all non-zero; `data.limitations` lists what the provider skipped.
- `check`: `data.schema_ok` true, `data.problems` empty, `data.stale_locations` 0.
- `repos`: the clone shows a latest run with status `done`.

## Notes
Re-running `build` after the clone moves on creates a new run; claims keep citing the run they were written against and `check` marks drifted locations stale.
Name clones by their upstream URL in any document you publish, never by a local path.
