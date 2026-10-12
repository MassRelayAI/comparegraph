# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Claude Code plugin manifest and marketplace listing (`.claude-plugin/`), the `using-comparegraph` entry skill, and the
  contributor files (`AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`).

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

[Unreleased]: https://github.com/nicholasandreasujitno/comparegraph/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/nicholasandreasujitno/comparegraph/releases/tag/v0.1.0
