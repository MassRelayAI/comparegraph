# Language fixtures

`fixtures/languages/<language>/` holds a small hand-written source file for each of the 32 language ids the walker knows
(29 languages; `tsx`, `jsx` and `mjs` are separate ids). `fixtures/languages/DIGESTS.json` records, per id, the file, node and
edge counts, the counts by kind, and a SHA-256 digest of the sorted canonical lines of the graph:

- node: `N|kind|qualified_name|path|line_start-line_end`
- edge: `E|kind|source_qn|target_qn|confidence`

`cargo test -p comparegraph --test languages` builds the directory as one repo, recomputes the digests and fails with the language,
the counts that moved and both digests. It also fails with `no fixture for <language>` when an id has no directory, and when the
walker's extension table drifts from the list in the test. `html` and `sql` yield only the file node; their entries carry a `note`.

Why: the five resolved languages have golden graphs (`conform`), but the other grammars would otherwise change under a
dependency bump unnoticed. Re-gold on purpose, in the bump's own PR:

```bash
UPDATE_LANGUAGE_DIGESTS=1 cargo test --locked -p comparegraph --test languages
git diff fixtures/languages/DIGESTS.json   # put the count diff in the commit message
```

Adding a language to the walker means adding its directory, an entry in the test's probe list, and re-golding.
