# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Plan 7 lane 1: a feature-mapping gold standard (30 features, six clones, two readers, `fixtures/gold-map/` with floors and
  `scripts/map-score.sh`), feature-mapping baselines (`scripts/map-baselines.sh`), the graph-backed absent check
  (`absent-check`, and `claim ... none` without `--searched` runs it and records the channels), `map` ranking by idf-weighted
  terms with kind and graph priors (held-out precision@5 0.038 / recall@5 0.149, pooled 0.109 / 0.378 from 0.046 / 0.141),
  a `matched` field on map candidates, unit tests for the ranking, and `relate` counting a feature only when its top candidate
  is named for it.
- Documentation for the public repository: `docs/design.md`, `docs/decisions.md`, `docs/roadmap.md`, `docs/reports/README.md`,
  `docs/dev/` (plugin protocol, PID namespaces, scripts); plan files are no longer tracked.
- Claude Code plugin manifest and marketplace listing (`.claude-plugin/`), the `using-comparegraph` entry skill, and the
  contributor files (`AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`).
- Plan 5: module-level calls with a file-node caller, declared return and field types, per-case `it:` test nodes, Python
  re-exports through `__init__`, no-op builds reuse the latest run, `setup`/`doctor` for six MCP clients, the npm shell,
  `export csv`, `catalog`, CI and release workflows, a gold standard of 150 labelled call sites per project under
  `fixtures/gold/`, and a proof on a real product.
- Plan 6, recall: a reusable gold scorer (`scripts/gold-score.sh`, `tests/gold.rs` with floors); TypeScript `describe` nodes,
  JSX element calls, class fields bound to functions as methods, class-method resolution through imports and `extends`, barrel
  re-exports followed, shadow-aware receivers; Go receiver methods through declared types, package-qualified calls through
  `go.mod`, same-package calls. Gold recall: TypeScript 0.31 → 0.67, Go 0.25 → 1.00, pooled 0.63 → 0.83 at precision 0.99.
- Plan 6, scale: indexed resolver lookups and a bounded thread pool for per-file analyses (store hash identical); incremental
  edges and communities proven hash-identical to a full build (fallbacks named in a limitation); bounded adjacency loading
  for `impact`, `traverse`, `tests-for`, `entry-points`, `dead-code` (flat memory on 434k nodes), with edge indexes added on
  open (traverse 1.7 s → 0.28 s on 1.3M edges); repeated limitations collapsed with a count; per-pass profiling with
  `COMPAREGRAPH_PROFILE=1`; `COMPAREGRAPH_THREADS`; every test binary runs in its own PID namespace where the host allows
  it (`tools/pidns-run.sh`).
- Plan 6, a second engine: the external plugin host over JSON lines (`docs/dev/plugin-protocol.md`), bounded reads and waits,
  validated plugin paths, process-group kill, a per-language conformance gate; `comparegraph-provider-crg` for
  code-review-graph and `build --providers a,b` with an agreement row per kind.

### Fixed
- A `kill -KILL -<pid>` shell-out in a test became `kill(-1)` on hosts with 7-digit pids (every process of the user);
  signals now go through `libc::kill` and `tests/no_kill_shellout.rs` forbids any kill shell-out.
- A name-only call target no longer binds at 1.0 through a shadowed import, a class without the method, a type alias or
  interface, a benchmark or example copy, or another file's data binding.

## [0.1.0]

### Added
- Core engine (Plan 1): register a clone, parse it with our own tree-sitter engine into the CompareGraph Graph (schema v1) in
  one SQLite file, verify it (locations resolve, deterministic builds, conformance against golden fixtures), export JSON.
- Surfaces (Plan 2): one response envelope for every verb, communities per run, full-text symbol search, `show` with code and
  licence lane, an Obsidian-style vault export, and an MCP stdio server over the read verbs. Deterministic; no LLM.
- Analysis layer (Plan 3): taxonomy packs (family, group, feature), corpus-seeded `family propose`, relatedness per clone and
  family, candidate mapping, claims with resolving citations, append-only adopt/reject/park decisions, and a comparison matrix
  with gaps and differentiators.
- Engine depth (Plan 4): inheritance and test edges, import-aware call resolution, multi-hop traversal, impact, related tests,
  entry points and dead-code candidates, incremental builds, more conformance languages, and a measured parity report
  against code-review-graph (`docs/reports/2026-10-12-engine-parity.md`).

[Unreleased]: https://github.com/massrelay-ai/comparegraph/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/massrelay-ai/comparegraph/releases/tag/v0.1.0
