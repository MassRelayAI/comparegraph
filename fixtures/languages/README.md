# Language fixtures

One small source file (or two) per parsed language id, written for this repo (Apache-2.0). `crates/comparegraph/tests/languages.rs`
builds this whole directory as one repo and compares, per language, the counts and a SHA-256 digest of the graph with
`DIGESTS.json`. A tree-sitter grammar bump that changes what the engine extracts turns that test red.

Rule: a grammar bump that changes a digest is re-golded on purpose, in the same PR as the bump, with the diff of counts in
the commit message.

Re-gold: `UPDATE_LANGUAGE_DIGESTS=1 cargo test --locked -p comparegraph --test languages`, then read `git diff
fixtures/languages/DIGESTS.json` and judge whether the change is an improvement or a regression before committing it.
See `docs/dev/language-fixtures.md`.
