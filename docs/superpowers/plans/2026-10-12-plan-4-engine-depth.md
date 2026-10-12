# CompareGraph Plan 4 — engine depth (parity with mature code graphs, measured)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans or subagent-driven-development. Steps use `- [ ]`.

**Goal:** Close the measured gaps of the reference engine against mature code graphs (code-review-graph as the yardstick):
inheritance and test edges, import-aware call resolution, multi-hop traversal, impact (blast radius), related tests, entry points
and dead-code candidates, incremental builds, more conformance languages — then MEASURE parity against code-review-graph on the
same three study projects and publish the numbers. Nothing is claimed "on par" without that measurement.

**Baseline (measured 2026-10-12 on code-review-graph's own source, 333 files):** edge kinds produced 3 (contains 8,824 · imports 758 ·
calls 16,051); 24 % of call edges are name-only guesses (confidence 0.5); relative imports only; one-hop queries only; full rebuild
every time; conformance fixtures for TypeScript and Python only.

**Architecture:** all new extraction lives in OUR layer (`providers/reference/{inherit,resolve,tests_link}.rs`), reading the
tree-sitter trees and the vendored `ParsedFile` output — the vendored files stay byte-identical. New queries live in
`application/graph.rs` over the stored edges (SQLite recursive CTEs or in-memory BFS over one run's edges, bounded).
Edge kinds `implements` and `tests` already exist in the schema; `extends` is added to `EdgeKind` (schema stays v1, unreleased).

## Global Constraints
- Determinism holds: same clone + same version → same store_hash; every new list sorted.
- Every query is bounded: `--depth` (default 3, max 10) and `--max-nodes` (default 200, max 2,000); a truncated answer says so in
  `evidence.limitations` (`truncated`), never silently.
- Confidence on every new edge: 1.0 when resolved through an import or the same file/class; 0.5 when by name across files; edges
  below 0.5 are not written.
- Every new verb answers with the envelope and is exposed over MCP; MCP tool total rises from 12 to 17.
- Vendored files untouched; clippy `-D warnings`; tests green; trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus
1. A call cycle (a → b → a) and self-recursion: traversal and impact terminate, each node once → Task 4.
2. An impact query on a symbol with thousands of transitive callers: answer truncated at `max-nodes` with `truncated` recorded, in < 1 s on the study projects → Task 4.
3. A class inheriting from an imported base class in another file, and from an external library class: the first gets an `extends` edge to the base's node, the second records nothing (no dangling edge) → Task 1.
4. An incremental build after editing one file: results identical (store_hash) to a full build of the same tree → Task 5.
5. Two functions named `run` in different files, one imported by the caller: the call resolves to the imported one at 1.0, not 0.5 → Task 2.

---

### Task 1: Inheritance edges (`extends`, `implements`)
Our own tree-sitter pass per file (re-parse in `providers/reference/inherit.rs` with the same grammar the vendored parser picked;
reuse `vendor::parser` language selection by calling the grammar crates directly — do not edit vendor files):
- Python: `class_definition` → `superclasses` argument list identifiers → `extends`.
- TypeScript/JavaScript: `class_declaration` → `class_heritage` → `extends_clause` (→ `extends`), `implements_clause` (→ `implements`).
- Java: `superclass` (→ extends), `super_interfaces` (→ implements). C#: `base_list` (first class-like → extends, rest → implements, P).
- Rust: `impl_item` with `trait` field → `implements` from the type's node to the trait's node.
- Go: none (structural typing; recorded as a limitation in capabilities).
Targets resolve like calls: same file → imported name → unique name across the repo (0.5) → else no edge.
- [ ] Tests first: per language a two-file fixture snippet (base in one file, subclass importing it) → the edge with 1.0; an external base → no edge (Review Focus 3); Rust trait impl → implements.
- [ ] Commit `feat(reference): extends and implements edges for Python, TS/JS, Java, C#, Rust`.

### Task 2: Import-aware call resolution
In `resolve.rs`, before the name-only fallback: (1) same class: a call with receiver `self`/`this` resolves to a method of the
caller's enclosing class (needs the class span: the class node whose lines contain the caller); (2) imported names: when the
caller's file imports a specifier equal to the callee name and that import resolves to file F (existing resolver, now also Python
absolute dotted modules and TS `index` files — already there — plus Go: an import path whose last segment matches a directory in
the repo), resolve to `F::callee` at 1.0; (3) module-qualified calls `mod.func()` where `mod` is an imported module alias → F::func
at 1.0. Only then the existing same-file / unique-name / 0.5 fallback.
- [ ] Tests first: Review Focus 5; `self.helper()` → the class's own `helper`; `import utils; utils.parse()` (Python) and
  `import * as u from './u'; u.parse()` (TS) → 1.0; measure on the study projects: share of 0.5 call edges before vs after
  (record in the commit body; target: below 10 %).
- [ ] Commit `feat(resolve): import-aware and receiver-aware call resolution`.

### Task 3: Test links
`tests` edges from a test node to the code it exercises: (a) every non-test symbol called (directly) from inside a test file;
(b) naming convention: `test_foo` / `foo.test.ts` / `foo_test.go` / `FooTest` → the symbol or file `foo`/`Foo` when unique.
Confidence 1.0 for (a), 0.5 for (b). Test symbols are the symbols contained in test files (node kind `test` is the file).
- [ ] Tests first: a Python test calling `parse` → tests edge; `foo.test.ts` → `foo.ts`; no edge to a test helper inside another test file.
- [ ] Commit `feat(reference): test links by call and by naming convention`.

### Task 4: Graph queries — traverse, impact, related tests, entry points, dead code
`application/graph.rs` + CLI + MCP:
- `traverse <repo> <qualified-name> --direction callers|callees|both [--depth 3] [--max-nodes 200] [--kinds calls,extends,implements]` → nodes with their hop distance and the edge path to them.
- `impact <repo> (<qualified-name> | --file <path>...) [--depth 5] [--max-nodes 500]` → everything that transitively depends on the target (reverse `calls`, `extends`, `implements`, `imports` for files), grouped by hop, each with its community; plus `tests` that reach any impacted node (the related tests).
- `tests-for <repo> <qualified-name>` → tests linked directly or through ≤ 3 call hops.
- `entry-points <repo>` → symbols with no incoming `calls` that are exported or named `main`/handlers (P: exported = name not starting with `_` in Python, `export`ed in TS from the vendored signature when available; else all top-level functions with no callers).
- `dead-code <repo>` → candidates: no incoming calls, not an entry point, not a test, not referenced by `extends`/`implements`; labelled `candidate` with confidence `unknown` (dynamic dispatch is invisible — say so in limitations).
MCP tools: `comparegraph_traverse`, `comparegraph_impact`, `comparegraph_tests_for`, `comparegraph_entry_points`, `comparegraph_dead_code` (17 total).
- [ ] Tests first: Review Focus 1 and 2; impact by file; tests-for through two hops; entry points and dead code on a small fixture; truncation recorded.
- [ ] Commit `feat(graph): traverse, impact, related tests, entry points and dead-code candidates — CLI and MCP`.

### Task 5: Incremental build
`build` reuses the previous done run when the provider version is unchanged: files whose content hash (the file node's
content_hash) is unchanged keep their parse output (re-hydrated from the stored nodes — or simpler and acceptable: cache the
vendored parse output per file hash in a new table `parse_cache(file_hash, language, provider_version, payload_json)`), only changed
or new files are parsed; edges and communities are always recomputed for the whole run (cheap). The run is still a new run with
its own rows. `build --full` bypasses the cache.
- [ ] Tests first: Review Focus 4 (edit one file → incremental store_hash == full-build store_hash of the same tree); a removed file disappears; timing on code-review-graph's source: second build with one changed file vs first build (record in the commit body).
- [ ] Commit `feat(build): incremental builds through a per-file parse cache`.

### Task 6: Conformance in more languages
Fixtures `fixtures/rust/`, `fixtures/go/`, `fixtures/java/` (small, hand-verified expected.json including extends/implements/tests edges
where the language has them); extend the TypeScript and Python fixtures with an inheritance pair, an imported-name call and a test
file. Floors unchanged (nodes ≥ 0.95, edges ≥ 0.85). Fix OUR layer when a floor fails; never lower a floor; a vendored-extractor
miss is recorded in the fixture's NOTES.md and in design notes.
- [ ] Commit `test(conform): Rust, Go and Java fixtures; inheritance, imports and tests in TS and Python`.

### Task 7: Measured head-to-head with code-review-graph
A proof script (`scripts/compare-engines.sh`, committed; no product dependency on Python): in a scratch dir, a Python venv with
`pip install code-review-graph==2.3.9`; for each study project, COPY the clone into scratch (code-review-graph writes its store
inside the repo — never run it in the study folder), run `code-review-graph build`, read its SQLite (`nodes`, `edges`), map its
node and edge kinds to ours (document the mapping in the script header), and compare with our latest run on the same commit:
per kind, Jaccard over (file, name) for nodes and over (kind, source, target) for edges; plus feature parity rows (which queries
each answers). Also time both builds. Write `docs/reports/2026-10-12-engine-parity.md` (front matter date, valid_until +90 days,
verdict, status) with the table and an honest verdict per area: on par / behind / ahead. Paths of study clones never committed.
- [ ] Commit `docs: measured parity with code-review-graph on three projects`.

## Self-review
Gaps from the baseline each have a task: edge kinds (1, 3), call accuracy (2), imports (2), one-hop (4), rebuild (5), languages (6).
Parity is a measurement (7), not a claim. Review Focus mapped to Tasks 4, 4, 1, 5, 2.
