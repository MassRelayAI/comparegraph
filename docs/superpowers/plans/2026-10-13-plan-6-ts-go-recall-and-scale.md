# CompareGraph Plan 6 — TypeScript and Go recall, resolver scale, and a second engine

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use `- [ ]`. Lanes own disjoint files
> and run in parallel worktrees; every task gets a reviewer.

**Goal:** Close the two measured recall gaps (gold recall TypeScript 0.31 vs 0.56, Go 0.25 vs 0.65 — `docs/reports/2026-10-12-engine-parity.md`,
"After Plan 5"), make the resolver scale (openclaw, 38k files: 31 min with every parse a cache hit — `docs/reports/2026-10-13-proof-on-a-real-product.md`),
cut limitation noise, and ship the first external engine adapter (code-review-graph) behind the plugin protocol so "engines are
pluggable" is true in practice. Every recall change is re-scored against `fixtures/gold/` (the committed two-reader gold standard) —
no claim without that number.

**Baseline to beat (pooled gold, both readers agree):** ours P 0.98 / R 0.63; code-review-graph P 0.95 / R 0.71. Targets: TS recall ≥ 0.55,
Go recall ≥ 0.60, pooled recall ≥ 0.72 with precision ≥ 0.96; openclaw full build < 8 min; unchanged rebuild unchanged.

## Global Constraints
- Lane A (recall) owns `providers/reference/**` (never `vendor/`), `fixtures/**` except `fixtures/gold/**` (read-only truth), `tests/{recall,gold}.rs`.
  Lane B (scale) owns `application/{build,graph,communities}.rs`, `store/**` (schema stays v1 unless a ruling says otherwise), `tests/{incremental,perf}.rs`, `benches/**`.
  Lane C (engine 2) owns `providers/{mod,external}.rs` + `providers/crg/**`, `application/conform.rs`, `docs/plugin-protocol.md`, `tests/{external,crg}.rs`.
  A need to cross the line is reported, not done. Shared: nothing. `Cargo.lock` changes only by ruling.
- Precision invariants stand (no name-only 1.0 edge; unknown receivers bind nothing; builtins first; a module-table reference binds only an imported or same-file function).
- `scripts/gold-score.sh <project-copy> <engine>` (Lane A builds it first, from `scripts/compare-engines.sh`'s scorer) scores an engine's run against `fixtures/gold/<project>/calls.jsonl`; every Lane A task reports before→after gold P/R on hono and caddy, and the pooled number.
- Determinism, the envelope, vendored files byte-identical, clippy `-D warnings`, tests green, the commit attribution trailer.

## Review Focus
1. A TS call inside a `describe()` body or at module level attributed to a new `describe`/file-scope node must not rescue dead-code candidates in non-entry files (same rule as A1 of Plan 5) → A1.
2. Go method resolution through a same-file receiver type must not bind `x.Get()` when `x` is a parameter of interface type (interface vs implementation ambiguity → no edge, confidence never 1.0) → A2.
3. The resolver's scale fix must not change any edge: the full-repo edge set on the five study projects is byte-identical before and after (store_hash equal) → B1.
4. An external engine process that hangs, prints garbage, or exits non-zero must fail the build with `provider_unavailable` within the timeout, never corrupt the store → C1.
5. The crg adapter's edges must obey our confidence rule: an edge crg marks 1.0 is stored at 1.0 only when its kind is one we also resolve from evidence (import/declared type); otherwise 0.5 — the conformance suite and the gold score must show crg-through-adapter ≥ the native crg numbers in the parity report → C2.

---

## Lane A — recall

### A0: `scripts/gold-score.sh` and `tests/gold.rs`
Extract the scorer from `scripts/compare-engines.sh` into a reusable script; a test that scores our engine on the committed gold labels for the five projects IF the study copies are present at `$COMPAREGRAPH_STUDY` (skipped otherwise) and asserts the current numbers as a floor (never lower; raised by later tasks).
- [ ] Commit `test(gold): a reusable gold scorer and a floor test on the committed labels`.

### A1: TypeScript — describe bodies, module-level test code, JSX elements, hooks
Gold: 18 of 44 hono repo-target sites sit in `describe()` bodies or at module level with no enclosing function node. Add a node per `describe` block (kind `function`, name `describe: <title>`, contained by the test file, nested describes contained by their parent) so calls inside attribute to it; module-level calls in TS test files attribute to the file (Plan 5 A1 already) — make sure they are counted by the gold scorer's "smallest enclosing node" rule (the scorer uses nodes; a file node qualifies). JSX: `<Foo />` → a `calls` edge to component `Foo` when `Foo` is imported or same-file, confidence 1.0, kind `calls` (document as a rendering call; the readers marked these ambiguous — record in design notes). Hooks `useX()` resolve like any call. Review Focus 1.
- [ ] Gold before/after on hono; commit `feat(ts): describe nodes, JSX element calls, module-level test code`.

### A2: Go — same-file and same-package receiver methods, package-qualified calls
Gold: caddy recall 0.25; misses are `d.Next()`, `p.addresses()` on receivers declared in the same file or package. Add `go_ctx.rs` (our layer): typed locals from `var x T`, `x := T{}`, `x := NewT()` with a declared return type, parameters `x T` / `x *T`; receiver methods `func (r *T) M()` indexed per package (directory); `pkg.F()` through the import path → the package directory (already) → the function; interface-typed receivers bind nothing (Review Focus 2). Type conversions `T(x)` are NOT calls (readers: ambiguous) — no edge.
- [ ] Gold before/after on caddy; commit `feat(go): receiver methods through declared types, package-qualified calls`.

### A3: The hono false-edge causes
`render()` bound to a benchmark copy (`benchmarks/jsx/src/hono.ts::render` instead of `src/jsx/dom/render.ts`): when a same-named function exists in more than one file and the caller's import does not resolve, prefer NOTHING (0.5 only if exactly one candidate outside `benchmarks/`, `examples/`, `test*/` dirs; else no edge). `new Response` bound to a type alias: `new X()` binds only to a class, never to a type alias or interface. A local `let` in another test file: a bare name never binds to a variable in a different file.
- [ ] Gold precision before/after on hono; commit `fix(resolve): no binds to type aliases, benchmark copies or other files' variables`.

### A4: Limitation noise
`clamped_line_end` ×611 on openclaw: collapse per run into one limitation with a count and the first five paths; same for any code that repeats > 20 times. Envelope size on a big build must stay under 8 KB.
- [ ] Commit `fix(build): repeated limitations collapsed with counts`.

## Lane B — scale

### B1: Profile, then fix the resolver's super-linear passes
Build a `--profile` flag (or `COMPAREGRAPH_PROFILE=1`) that prints per-pass wall time and node/edge counts; run on openclaw's copy (study clone; never commit the path) to find the passes that dominate (suspects: `by_name` rebuilt per file, `symbol_in` scanning all symbols per call, `file_syms` linear lookups, the re-export rounds, dedup over Vec). Replace each hot spot with indexed lookups (HashMap<name, Vec<idx>> built once per repo; per-file symbol maps; binary search over sorted spans for `caller_at`). Review Focus 3: store_hash identical on all five study projects before/after.
- [ ] Targets: openclaw full build < 8 min; the product repo < 90 s; commit `perf(resolve): indexed lookups; per-pass profiling`.

### B2: Communities and edge recomputation on incremental builds
When only K of N files changed, recompute edges whose source or target file changed, keep the rest (edges are stored with run_id — copy unchanged rows into the new run); communities recomputed only for affected components. Determinism: the result must equal a full build's store_hash (test on the study copies with one file edited).
- [ ] Commit `perf(build): incremental edges and communities`.

### B3: Query bounds on big graphs
`impact`/`traverse` on 400k-node graphs: load only the needed adjacency (SQL recursive CTE with depth and node caps, or lazy adjacency by node id) instead of the whole run's edges; `entry-points` and `dead-code` paginated; measure on openclaw's copy (< 2 s for impact at default bounds).
- [ ] Commit `perf(graph): bounded adjacency loading`.

## Lane C — the second engine

### C1: The external plugin host (spec §6b)
`providers/external.rs`: discover `comparegraph-provider-<name>` on PATH or a path in `comparegraph.toml`; JSON-lines protocol v1 (`capabilities`, `build`, progress lines, `ok`/`error` answers); a timeout (default 30 min, configurable), an output byte cap, stdout as data only; `doctor` lists external providers with their last conformance result; `conform <name>` runs the fixtures through them. `docs/plugin-protocol.md` documents the protocol with one example exchange. Review Focus 4 (hang / garbage / non-zero exit → `provider_unavailable`, store untouched).
- [ ] Commit `feat(providers): external engine host over JSON lines`.

### C2: `comparegraph-provider-crg`
A small Python script (shipped under `providers/crg/`, no product dependency — it needs `code-review-graph` installed in the user's environment; `doctor` says so when missing) that runs `code-review-graph build` on the clone (on a COPY if crg writes into the repo, as the parity script does), reads its SQLite and emits our node/edge kinds with the confidence rule of Review Focus 5. Conformance: must pass the five fixtures at the floors; gold: report crg-through-adapter P/R per project next to the native crg numbers. `build --providers reference,crg` then yields an `agreement` row per kind (Plan 4's deferred item): Jaccard over qualified names and edges, written into the run's limitations when < 0.8.
- java/rust: shape differences, gated per language (the conform gate refuses a repo only for a failed language it contains; `doctor` shows conformed and failed languages).
- [ ] Commit `feat(crg): the code-review-graph adapter and the agreement score`.

## After the lanes: the re-measure and the codegraph head-to-head
Re-run `scripts/compare-engines.sh` on the five projects and append "After Plan 6" with the gold table (both engines, and crg-through-adapter); re-run the codegraph-mcp head-to-head from the proof ONLY when no other codegraph process is alive on the host (`pgrep -af codegraph-mcp` empty — the maintainer's call on timing); append to the proof report.

## Self-review
Each "behind" from the Plan 5 re-measure has a task (TS A1/A3, Go A2); scale from the proof (B1–B3); noise (A4); the plugin promise of spec §6b (C1–C2); Review Focus mapped A1, A2, B1, C1, C2. Lanes share no file.
