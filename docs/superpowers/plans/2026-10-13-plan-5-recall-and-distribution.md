# CompareGraph Plan 5 — the remaining recall classes, and distribution (two independent lanes)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use `- [ ]`. Lane A and Lane B
> own disjoint files and run in parallel worktrees; each task gets a reviewer.

**Goal:** Lane A closes the measured "behind" classes of the engine (module-level calls, return-typed locals, TS/JSX field types,
per-case test nodes, same-family test links, Python re-exports, no-op builds) and re-measures. Lane B makes CompareGraph installable
and discoverable by anyone: `setup`/`doctor` for agent clients, an npm shell with prebuilt binaries, a Claude Code plugin with an
entry skill, CSV export, a generated tool catalog, CI.

**Spec:** `docs/superpowers/specs/2026-10-12-codecross-design.md` §6b (plugins), §9 (verbs incl. `setup`), §9b (envelope), §10 (agent surface), §15 (standards). **Baseline:** `docs/reports/2026-10-12-engine-parity.md` (after the review fixes).

## Global Constraints
- Lane A owns: `crates/comparegraph/src/providers/reference/**` (never `vendor/`), `application/{graph,build,communities}.rs`, `fixtures/**`, `tests/{graph,incremental,recall}.rs`, `scripts/compare-engines.sh`, the parity report. Lane B owns: `adapters/**`, `application/{export,setup,catalog}.rs`, `packages/**`, `.claude-plugin/**`, `skills/**`, `method/**`, `.github/**`, `README.md`, `AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`, `CHANGELOG.md`, `docs/catalog.json`. Neither touches `Cargo.lock` (no new deps without a ruling), `store/schema.sql`, or the other lane's files. A need to cross the line is reported, not done.
- Precision invariants from Plan 4 stand: no name-only 1.0 edge; unknown-receiver calls bind nothing; builtin denylists first.
- Determinism, envelope on every verb, vendored files byte-identical, clippy `-D warnings`, tests green, trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Lane B writes nothing outside the repo by default: `setup` prints its plan; `--apply` writes, additively, with a timestamped backup beside the file; a dry run is the default. Tests exercise `--apply` only against files under a temp HOME (`HOME`/`XDG_CONFIG_HOME` overridden).

## Review Focus
1. Module-level callers must not turn every module-level-called symbol into "called": `entry-points` and `dead-code` treat a file-node caller as an entry (a symbol called only at module level of its own file is still a dead-code candidate unless the file is an entry file such as `main.py`, `index.ts`, `__main__.py`, `cli.py`) → Lane A Task 1.
2. A return-typed local must come from a declared return type (`-> Foo`, `: Foo`, Rust `-> Foo`), never inferred from the body → A2.
3. `setup --apply` on a client config that already has a `comparegraph` entry pointing elsewhere: do not duplicate, do not silently overwrite; report `conflict` and leave the file untouched unless `--force` → B1.
4. The npm shell on an unsupported platform: a clear error naming the platform and the source-build command, exit 1, no partial install → B2.
5. `doctor` registrations: `aligned` only when the command vector equals what `setup` would write; a different binary path is `stale` → B1.

---

## Lane A — recall and build

### A1: Module-level calls (file node as caller)
Our own pass per language (Python, TS/JS/JSX, Go `init`/package-level `var`, Rust `static`/`const`/`lazy_static!` bodies, Java static initialisers), modelled on `js_ctx::test_case_calls` and its `CALLER_FILE` convention: a call outside any function body yields a `CallSite` whose caller is the file; resolution follows the existing steps (import-aware, declared types, builtins denylist, 0.5 cap) with the file node as source. Decorator calls (`@app.route(...)`) and module tables (`HANDLERS = {"x": handle_x}` — a bare identifier reference to a repo function at module level) count, the latter as `calls` at 1.0 when the name is imported or same-file, else dropped (never 0.5 for references). Review Focus 1: `graph.rs` entry-points/dead-code treat file-sourced calls as "called from an entry" only when the file is an entry file (list above, plus any file with `if __name__ == "__main__"`), else they do not rescue a candidate.
- [ ] Tests first (tests/recall.rs): Python `if __name__ == "__main__": main()` → `app.py` → `app.py::main` at 1.0; a decorator call; a module table reference; dead-code: a function referenced only from a module table in a non-entry file stays a candidate; TS top-level `app.use(router())`.
- [ ] Commit `feat(reference): module-level calls with the file as caller; entry files rescue dead-code candidates`.

### A2: Return-typed and field-typed locals (declared only)
- Python: `x = f()` where `f` (same file or imported, resolved) has a declared `-> T` return annotation → `x: T`; `a, b = f()` only when `-> tuple[A, B]`/`-> Tuple[A, B]`; class attributes `self.x: T` and `self.x = T(...)` in `__init__` → `self.x.m()` resolves to `T.m`.
- TS/JS: `const x = f()` with `function f(): T`; class fields `private reader: Reader;` / `reader = new Reader()` → `this.reader.m()`; constructor parameter properties `constructor(private reader: Reader)`.
- Rust: `let x = f()` with `fn f() -> T` (same crate, resolved by rust_ctx); struct fields `pub reader: Reader` → `self.reader.m()`.
- Review Focus 2: no inference from bodies; a function without a declared return type types nothing.
- [ ] Tests first for each language shape, plus the negative (no annotation → no edge).
- [ ] Commit `feat(resolve): return-typed and field-typed receivers from declarations`.

### A3: Per-case test nodes and same-family test links
`it("…")`/`test("…")`/`describe` (JS/TS), Python `def test_*` already are symbols; Rust `#[test]` fns are. Add a node per JS/TS case: kind `Test`? — no: the `Test` kind is the file; add nodes of kind `function` named `it: <title>` (slug-safe) contained by the test file, so `tests-for` can name the case; their calls attribute to the case node, not the file. Naming-convention links (`tests_link.rs` (b)) restricted to the same language family. Rust inline `mod tests` fns link by call like any test.
- [ ] Tests first: an `it` case node with a `tests` edge to what it calls; `test_x.py` does not link `x.ts`.
- [ ] Commit `feat(tests): per-case test nodes for it/test/describe; naming links within a language family`.

### A4: Python re-exports and the `__init__` rule
`from .impl import run` inside `pkg/__init__.py` makes `pkg.run` resolve to `pkg/impl.py::run`; `import pkg; pkg.run()` follows it. Also `__all__` is ignored (not needed). A name used as an object (`from m import ring; ring.since()`) binds a class or an instance assignment in `m`, never a function of that name (design-note item).
- [ ] Tests first; commit `feat(resolve): Python re-exports through __init__ and object-vs-function binding`.

### A5: No-op builds and changed-rows-only writes
`build` on a repo whose head and every file hash are unchanged since the latest done run (all cache hits, same provider version) returns that run (`reused: true` in BuildReport, no new rows) unless `--full`. When some files changed, still a new run (correctness first), but communities/edges computed once — no change here beyond the no-op path. Time it on code-review-graph's source: unchanged rebuild should fall from ~3.9 s to the walk time (< 0.5 s).
- [ ] Tests first (tests/incremental.rs): no-op returns the same run id and hash; a changed file produces a new run; `--full` always a new run.
- [ ] Commit `feat(build): a no-op build reuses the latest run`.

### A6: Re-measure
Run `scripts/compare-engines.sh` as before (fresh copies, code-review-graph 2.3.9, seed 7, 20 crg-only + 10 ours-only per project, hand-classified by reading). Append "After Plan 5 (2026-10-13)" to the parity report with before→after and refreshed per-area verdicts. Expect: call recall on Rust/TS up (module-level + field types), test links up (per-case nodes).
- [ ] Commit `docs: parity re-measured after plan 5`.

## Lane B — distribution

### B1: `setup` and `doctor` for agent clients
`application/setup.rs` + CLI `setup [--client claude|codex|cursor|gemini|opencode|hermes|all] [--root <ws>] [--apply] [--force] [--scope user|project]`. Client config shapes (read each client's documented location; write only the `comparegraph` server entry): Claude Code — project `.mcp.json` (`mcpServers.comparegraph = {command, args:["--root",<abs ws>,"mcp"]}`) or user `~/.claude.json` `mcpServers`; Codex — `~/.codex/config.toml` `[mcp_servers.comparegraph] command = … args = […]`; Cursor — `~/.cursor/mcp.json`; Gemini CLI — `~/.gemini/settings.json` `mcpServers`; OpenCode — `opencode.json` `mcp.comparegraph = {type:"local", command:[…]}`; Hermes — `~/.hermes/config.yaml` under `mcp_servers` (document the shape you verified; if a client's shape cannot be verified from its docs/source in this environment, mark it `unverified` in the envelope and skip it by default). JSON written with 2-space indent preserving other keys; TOML/YAML appended as a block when absent (never rewrite the file). Backup `<file>.bak-<unix-seconds>` on apply. `doctor` gains `clients: [{client, path, state: aligned|stale|missing|conflict|unverified}]` (Review Focus 5).
- [ ] Tests first under a temp HOME (Review Focus 3 and 5; dry run writes nothing; apply writes once, idempotent twice).
- [ ] Commit `feat(setup): register the MCP server with agent clients, plan first, additive and backed up; doctor reports registrations`.

### B2: npm shell with prebuilt binaries
`packages/comparegraph/` (name `comparegraph`, `bin: {comparegraph: bin/comparegraph.js}`): the bin script resolves `@comparegraph/<platform>-<arch>` from `optionalDependencies` (`linux-x64`, `darwin-arm64`, `darwin-x64`, `win32-x64`), execs the binary with the same args and exit code, and on a missing platform prints `comparegraph: no prebuilt binary for <platform>-<arch>; build from source: cargo install --git <repo> comparegraph` and exits 1 (Review Focus 4). `packages/platforms/<name>/package.json` templates + `scripts/package-npm.sh <version> <platform>` that copies `target/release/comparegraph` (stripped) into the platform package. No `postinstall`. A smoke test (`node packages/comparegraph/test/bin.test.mjs`): with a fake platform package on `NODE_PATH`, `--version` round-trips; with none, the error and exit 1. Document in README: `npx comparegraph …` and `npm i -g comparegraph`.
- [ ] Commit `feat(npm): the comparegraph package shell with per-platform binary packages`.

### B3: Claude Code plugin, entry skill, contributor files
`.claude-plugin/plugin.json` (name `comparegraph`, description, version from Cargo, author, repository, license Apache-2.0, keywords), `.claude-plugin/marketplace.json` (one plugin), `skills/` = the method skills (move `method/*` to `skills/*`, leave `method/README.md` pointing there — or symlink; pick one and say why) plus a new entry skill `skills/using-comparegraph/SKILL.md` (when to use which skill; the five-line workflow; "run `comparegraph setup --client claude --apply` once"). `AGENTS.md` and `CLAUDE.md` (= `@AGENTS.md`) for contributors: layout, the rings, the precision invariants, how to run tests, never touch vendor/. `CONTRIBUTING.md`, `CHANGELOG.md` (0.1.0 entry), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1, verbatim with attribution).
- [ ] Commit `feat(plugin): Claude Code plugin manifest, entry skill, contributor files`.

### B4: CSV export and the generated catalog
`export csv --out <dir>`: `nodes.csv`, `edges.csv`, `claims.csv`, `decisions.csv`, `matrix-<family>.csv` (RFC 4180, header row, deterministic order). `comparegraph catalog` prints the MCP tool catalog (name, description, inputSchema, the envelope error codes) as JSON; `docs/catalog.json` is committed and a test fails when it differs from the live catalog (`comparegraph catalog --check`).
- [ ] Commit `feat(export): CSV export; generated and checked tool catalog`.

### B5: CI and release workflows
`.github/workflows/ci.yml`: on push/PR — `cargo test`, `cargo clippy -D warnings`, `comparegraph conform reference --fixtures fixtures`, `comparegraph catalog --check`, the npm bin smoke test; Linux only. `.github/workflows/release.yml`: on tag `v*` — build release binaries for linux-x64, darwin-arm64, darwin-x64, win32-x64, attach to the GitHub release, and publish the npm packages only when `NPM_TOKEN` is present (manual approval environment). README: install section (npm, cargo, from source), the MCP client snippet replaced by `setup`.
- [ ] Commit `ci: test, lint, conformance and catalog checks; release binaries and npm on tags`.

## Self-review
Each "behind" row of the parity report has a Lane A task (module-level A1, locals/fields A2, tests A3, imports/re-exports A4, incremental A5). Spec §9 `setup`, §6b plugin packaging, §10 agent surface, §15 catalog → Lane B. Review Focus → A1, A2, B1, B2, B1. The two lanes share no file.
