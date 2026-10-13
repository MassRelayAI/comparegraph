# Contributing to CompareGraph

Read `AGENTS.md` first: the rings, the precision invariants and the never-list apply to every change.

## Proposing a change
1. Open an issue or a draft pull request saying what is wrong and how you saw it. A measurement beats an opinion.
2. Tests first. A bug fix starts with a test that fails; a new behaviour starts with the test that describes it.
3. Touching extraction (a provider, a language pass, call or type resolution)? Add or extend a conformance fixture under
   `fixtures/` with its expected graph, and run `comparegraph conform reference --fixtures fixtures`. Add the negative case:
   the shape that must not produce an edge.
4. A provisional choice (a threshold, a default, a heuristic you could not validate) gets one row in
   `docs/decisions.md`: the choice, the alternative, why it is provisional, what would change it.
5. Run `cargo test -p comparegraph` and `cargo clippy -p comparegraph --tests -- -D warnings`. Both must be clean.
6. Conventional commit messages; stage named paths.

## Out of bounds
Vendored code under `providers/reference/vendor/` is never edited. No LLM or network call enters the engine. New dependencies
need a ruling in the issue first. Code from copyleft or community-licensed projects is not accepted; record the origin and
licence of anything adapted in `docs/THIRD_PARTY_NOTICES.md`.

## Skills
The markdown skills live in `skills/` (`method/` is a symlink to it). Keep each skill short, command-first, and honest
about what the engine cannot see.

By taking part you agree to the `CODE_OF_CONDUCT.md`. Contributions are licensed under Apache-2.0.
