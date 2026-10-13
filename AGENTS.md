# AGENTS.md - contributing to CompareGraph

A Rust CLI + MCP server that compares cloned repositories against taxonomy packs. Users' skills live in `skills/`
(`method/` is a symlink to it, so older links still resolve); this file is for people and agents changing the engine.

## Layout and rings (dependencies flow inward)
```
crates/comparegraph/src/
  domain/       the standard: graph schema, taxonomy, claim, evidence, run - pure, no I/O
  providers/    one adapter per engine (reference = tree-sitter); the conformance runner
  application/  workflows: add, build, relate, map, claim, check, export, search, conform, doctor, setup
  adapters/     cli (clap, one-shot) and mcp (stdio) - thin
skills/ (method/ links here)  the markdown skills     families/  sample packs     fixtures/  golden repos per language
packages/  npm shell     docs/  specs, plans, reports, design notes, THIRD_PARTY_NOTICES.md
```
A module never imports outward. `domain` has no I/O; `providers` know engines, not workflows; `adapters` are thin.

## Precision invariants (Plans 4 and 5 Global Constraints; they stand for every plan after)
- Determinism holds: same clone + same version -> same store_hash; every new list sorted.
- Every query is bounded: `--depth` (default 3, max 10) and `--max-nodes` (default 200, max 2,000); a truncated answer says so in
  `evidence.limitations` (`truncated`), never silently.
- Confidence on every new edge: 1.0 when resolved through an import or the same file/class; 0.5 when by name across files; edges
  below 0.5 are not written.
- No name-only 1.0 edge; unknown-receiver calls bind nothing; builtin denylists first.
- Every verb answers with the envelope; read verbs are exposed over MCP.

## Commands
```
cargo test -p comparegraph
cargo clippy -p comparegraph --tests -- -D warnings
comparegraph conform reference --fixtures fixtures    # golden graphs per language
```
Toolchain is pinned in `rust-toolchain.toml`. Tests that use `--apply` run against a temp HOME only.

## Never
- Touch `providers/reference/vendor/`: vendored files stay byte-identical (see `docs/THIRD_PARTY_NOTICES.md`).
- Run `cargo fmt` over `providers/reference/`; format only files you wrote.
- Add an LLM call, network call or model dependency to the engine. It is deterministic and offline; `author`/`by` is always
  given by the caller, and the engine never writes a claim or decision itself.
- Add a dependency or edit `Cargo.lock` or `store/schema.sql` without a ruling.
- Execute code in a clone. Clones are read only.

## Where things are written down
Specs `docs/design.md`, plans `docs/roadmap.md`, measurements `docs/reports/`, every provisional choice
`docs/decisions.md`. Commit as conventional commits; stage named paths, never `git add -A`.
