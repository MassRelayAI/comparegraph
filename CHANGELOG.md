# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- cargo-deny in CI (`deny.toml`): licence allowlist, advisories, bans and sources, run as the `deny` job on every push and pull request.
- Build provenance attestations on the release binaries and `SHA256SUMS` (`actions/attest-build-provenance`); verify with `gh attestation verify`.
- `dependabot-automerge.yml`: a Dependabot patch or minor bump merges itself once CI is green on it; majors wait for the maintainer.
- `docs/assets/family.svg`: a family, its groups and features, six clones, 121 agreed cells from the mapping gold; `scripts/family-asset.mjs` derives its JSON.
- `docs/assets/accuracy.svg` and `scale.svg` (gold precision/recall per engine; files → nodes, edges, seconds), rendered from committed JSON by `scripts/readme-assets.mjs`.
- `scripts/answer-size.sh` and `docs/reports/2026-10-14-answer-size.md`: the size of what an agent reads (map answers for 30 features on six study copies) against the source it replaces; `docs/assets/cost.svg` rendered from `docs/assets/cost.json` by `scripts/readme-assets.mjs`, checked by a test.
- npm packaging: `@massrelay-ai/comparegraph` wraps the release binaries (five platform packages); published by `release.yml` once `NPM_PUBLISH` is set; `mcpName` for the MCP Registry.
- `server.json` lists the npm package (`registryType` npm, `runtimeHint` npx, positional `mcp`), so registry-aware clients can install the server; the README has the npm route and badge.
- `setup`/`doctor` state `plugin`: with the Claude Code plugin enabled, `--client all` leaves Claude Code's server and hook to the plugin.
- A bare `comparegraph` command that resolves on `PATH` to the registered binary reads as aligned.
- `setup` writes a CompareGraph rules block (between `<!-- comparegraph -->` markers) into the workspace's agent instruction file (only once `.comparegraph/` exists; otherwise the row is listed and skipped): `CLAUDE.md`, `AGENTS.md`, `GEMINI.md` or `.github/instructions/comparegraph.instructions.md`, one row per file. An older block is refreshed; a hand-edited one is a `conflict` unless `--force`. `--no-rules` skips it.
- `setup` registers a Claude Code `SessionStart` hook (`.claude/settings.json`, or `~/.claude/settings.json` at user scope) that runs `comparegraph doctor --brief`. `--no-hook` skips it.
- `doctor --brief`: plain text for the hook (nothing without a store; else the repo count and each detected client's registration, with a `--force` hint when one is stale or conflicting).
- `setup` and `doctor` know six more clients: Windsurf, Antigravity, Kiro, GitHub Copilot (VS Code), GitHub Copilot CLI and Qwen Code.
- `hooks/hooks.json`: the plugin ships the same session-start hook.

### Changed
- README in the inline style: centred HTML header with a doc-links row, rules between parts, a picture per part, numbers folded in details; CLI reference, MCP tools, configuration, vault, library usage, FAQ and troubleshooting moved to pages under `docs/` (`usage.md`, `commands.md`, `mcp-tools.md`, `faq.md`, `troubleshooting.md`).
- README front: centred header with platform and agent badges, Get started in four numbered steps with the other routes folded, Install merged into it; the cost illustration opens Why.
- README: Get started (three commands) and the cost illustration before Why; alternative install routes folded; the workflow diagram moved to How it works.
- README Install: A (recommended) / B / C per step; step 2 is `comparegraph setup --apply` for every agent found, the plugin, or the agent's own `mcp add` line.
- The install scripts hint `setup --apply`.
- `claim --batch` skips a row equal to the live claim (same run, verdict, author, note and locations): it writes nothing, reports `skipped: true` with the existing claim, and the summary gains `skipped`. An interrupted batch can be re-run without doubling its rows. `decide --batch` summaries report `skipped: 0`.
- `setup` and `doctor` rows carry `kind` (`config`, `rules` or `hook`; `doctor` also reports the `rules` and `hook` rows of detected clients); a `rules` row's `client` lists the clients that read the file, comma-joined.
- The `using-comparegraph` skill no longer points to npm for the binary.

## [0.1.1] - 2026-10-14

### Added
- `claim --batch <jsonl|->` and `decide --batch <jsonl|->`: many rows in one process, one store open and one transaction (a failing row writes nothing, the rest proceed); one envelope line per input row with its `line`, printed as each row is decided, then a `{rows, ok, failed, committed}` summary (an error envelope as the last line means nothing landed); exit 1 if any row failed. Store connections wait up to 5 s for a lock instead of failing at once.
- `--root` is global: it may follow the verb (`comparegraph repos --root ws`).
- `repos` takes `--limit` (default 50, maximum 500) and `--cursor`, and returns a `page`.
- `impact`, `tests-for` and `traverse` take `--limit` and `--cursor` (and the MCP tools `limit` and `cursor`).
- `build --progress` (or `COMPAREGRAPH_PROGRESS=1`) prints `[progress]` lines to stderr: one per 500 files parsed and one per pass.
- README: the `<group>.<slug>` feature-id rule in the families section.
- Family packs take an optional eighth column `synonyms` (comma-separated). They are terms of `map` and of the absent check, and `claim none --searched` accepts a synonym as naming the feature. Stored in a new `feature.synonyms` column, added to an existing store when it opens (schema version unchanged).
- `absent-check`: `data.common` and a `common=<terms>` note segment list the family words dropped from the check (a term in the term set of at least half of the family's features and of at least 4).

### Changed
- The absent check sets family words aside before deciding (see above): a hit on a remaining term refuses as before; zero hits with a family word set aside refuses too, naming the uncounted words (the absence is then the author's to record with `--searched`); a feature whose every term is a family word is refused ("every term of X is a family word in F (...); pass --searched with specific terms"). A graph-backed absence is recorded only when no term was set aside. The symbol channel counts every node whose name carries a term whatever its kind; a named kind feeds only the kinds channel. (A clone-frequency common-term rule and kinds scoping of the symbol channel were tried and withdrawn after review: both recorded a `none` on clones that have the feature.)
- `map` matches a short token (under 6 characters) only whole, or the longer is the shorter plus `s`, `es`, `ed`, `ing`, `er`, `ers`, `or`, `ors` (with a final `e` dropped before `ed`, `ing`, `er`, `ers`, a final `y` before `ies`, `ied`, `ier`): `multi` no longer matches `multiplier`, `task` still matches `tasks`. Longer tokens keep the prefix rule. Terms that were stop words only because of the old over-matching can now carry weight.
- `--pretty` is global like `--root`: both may come before or after the verb.
- `impact`, `tests-for` and `traverse` answer summary-first: `data.summary` (`total`, `truncated`, `max_nodes`, `by_hop`,
  `by_kind`), then `items` paged and ordered by `path_confidence` (high first), hop, name. `impact` no longer returns
  `by_hop[].nodes`; its `items` is the flat list with `hop` on each, and its `tests` list is capped at the page size with a
  `tests_capped` limitation when cut (`summary.tests_total` is always present). `total` and `truncated` stay at the top of `data`; `truncated` stays in `limitations[]`.

## [0.1.0] - 2026-10-13

The first public release.

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
- Plan 5: module-level calls with a file-node caller, declared return and field types, per-case `it:` test nodes, Python
  re-exports through `__init__`, no-op builds reuse the latest run, `setup`/`doctor` for six MCP clients,
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
- Install routes, as the Rust tools in the field do it: `scripts/install.sh` and `install.ps1` fetch the release binary
  (linux-x64, linux-arm64, darwin-arm64, darwin-x64, win32-x64) and check it against `SHA256SUMS`; a Homebrew tap; the
  Claude Code plugin (`/plugin install comparegraph --marketplace massrelay-ai/comparegraph`) carries the skills and the
  MCP server over the installed binary; an MCP Registry manifest (`server.json`) published by the release workflow;
  `cargo install --locked` from source.

### Fixed
- A `kill -KILL -<pid>` shell-out in a test became `kill(-1)` on hosts with 7-digit pids (every process of the user);
  signals now go through `libc::kill` and `tests/no_kill_shellout.rs` forbids any kill shell-out.
- A name-only call target no longer binds at 1.0 through a shadowed import, a class without the method, a type alias or
  interface, a benchmark or example copy, or another file's data binding.

[Unreleased]: https://github.com/massrelay-ai/comparegraph/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/massrelay-ai/comparegraph/releases/tag/v0.1.1
[0.1.0]: https://github.com/massrelay-ai/comparegraph/releases/tag/v0.1.0
