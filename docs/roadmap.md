# Roadmap

What comes next, in the maintainer's words and in what each plan left behind, each item with the measurement that
motivates it. Nothing here is promised; an item becomes a lane when it has a stop condition and a way to measure it. The
measurements are in [`reports/`](reports/README.md); the choices already made are in [`decisions.md`](decisions.md).

## Plan 7 candidates (2026-10-13)

## From the maintainer (2026-10-13)

"Since we are using this as standardization, do we need to include metrics, structure prioritization, etc.? Does this relate
to our structure?" — answered: metrics yes, prioritization no, interoperability as exports.

- **Feature stats export (metrics as data).** Per feature: how many comparators carry it, files / nodes / edges behind it,
  `tests-for` coverage, the comparator's licence. Pure counts over the store; no model. A prioritization formula does NOT go in
  the tool — the weights (tenant pull, maintainer calls, what a port displaces) belong to the user's own judging process.
- **Deterministic taxonomy baseline.** `family propose` already drafts families without a model; make the accepted rows carry
  their provenance (`drafted` / `edited`) so a taxonomy can be audited for where a person or an agent changed the draft.

## From a downstream consumer (a project that scores feature cells from our graph records)

1. `claim import <csv> --author <file>`: our-side claims fed from a tracker CSV.
2. A graph-backed `absent` verdict: no node of the named kind in the clone, with the searched terms recorded.
3. The parity script per family (a bake-off per family, not per project).
4. `family import/export` for a shared taxonomy format (`_rows-<family>.md`).

## From a user's run (2026-10-14: ten packs, 116 clones, about 4,400 claims through the CLI)

Most costly first, as the user ranked them; the measured numbers are theirs.

1. `claim --batch` and `decide --batch` (JSON lines or stdin): one store open and one transaction per batch instead of one
   process per verdict over a 5 GB store.
2. The absent check speaks too rarely (recall 0.05 on the gold; in a large tree every `none` became `unknown` because the
   clone carries words like `task` and `channel`): drop terms common in the clone and say so, scope by the kinds a feature
   names, and let a pack declare synonyms so `--searched` can name them.
3. Rules stricter than the README: `--root` only before the verb, `repos` without `--limit`, the `<group>.<slug>` id rule
   undocumented in the family section.
4. `impact`, `tests-for`, `traverse` answer 90 to 312 KB prefixes; `show` caps callers at 50: counts and the top few first,
   a cursor, every cap in `limitations[]`.
5. `build` is silent for minutes: `--progress`.
6. The ranked `map` matches `multi` to `multiplier`: whole-token matching for short terms, synonyms from the pack.

Kept, by the same user: the refusal of a `has` without a resolving citation, `none` needing proof, the envelope, the
deterministic store hash, the pack format, the build speed per clone, one binary with no model calls.

Status, 2026-10-14 (Plan 8): 1, 3, 4 and 5 shipped as asked (`claim --batch` about twenty times faster than single
processes on 2,000 rows; summary-first answers with a cursor; `build --progress`). 2 shipped as the family-word rule
(a term shared by at least half of a family's features is set aside and named; zero hits for the rest then refuses
rather than records); a clone-frequency rule and kinds scoping were tried and withdrawn after review, each having
recorded a `none` on a clone that had the feature. 6 shipped as the whole-token rule for short tokens plus the pack's
`synonyms` column. On the gold the absent check still records one absence in nineteen, and `map` on the held-out clones
is unchanged, so the mapping stop condition stays open (`docs/reports/2026-10-14-mapping-gold-first-measurement.md`).
Follow-up from the same run (2026-10-14): a re-run `claim --batch` skips rows equal to the live claim instead of
writing them again, so an interrupted batch resumes without doubling its rows.

## Navigation parity with codegraph (the maintainer, 2026-10-13: "why not do what codegraph and other OSS do")

Graph-derived answers that codegraph serves and we do not, all plain queries over the store, no model:
- **Circular dependencies**: cycles in the file-level `imports` graph (and in `extends`), with the cycle listed.
- **Dead imports**: an `imports` edge whose imported names are never referenced in the importing file.
- **Complexity and hot paths**: per function the call fan-in / fan-out and the longest path from an entry point; a ranked list.
- **Architecture summary**: communities with their labels, sizes, cohesion and the edges between communities, as markdown.
- **Edit context**: for a symbol, the callers, callees, tests-for and the containing community in one answer (the agent's
  "what do I need to read before I change this").
Parked, with the reason: markdown indexing and search, a memory store, PR context — not graph features, and the consumer
projects keep those in their own vault and ledgers. Reopen only on a user's pull.
The bar for replacing codegraph in a consumer's navigation: the head-to-head on the gold set at or above codegraph's
recall on that consumer's languages with precision ≥ 0.96 (the same bar as Plan 6), recorded in the parity report.

## Left behind by Plan 6 lanes

- **B4, per-file analysis cache.** B2's incremental build equals a full build but saves little at 2,600 files because the per-file
  analyses (py_ctx, decl_types, rust_ctx, module calls) and the cross-file indexes are rebuilt over every file; cache them per
  file by content hash, and copy unchanged node/edge rows run-to-run instead of re-inserting (store.insert_* was 0.95 s of 3.3 s).
- **Edge index.** `CREATE INDEX IF NOT EXISTS` on `edge(run_id, source_node_id)` and `edge(run_id, target_node_id)` at store
  open; schema version stays 1 (row shape unchanged; `store_hash` hashes rows). Takes traverse on a 1.3M-edge graph from 3.4 s
  toward the 2 s target. Ruled after B2 merges (store owner).
- **COMPAREGRAPH_THREADS** override for `par_map` (one line).
- **Go.** Calls on a method result, range variables, generic instantiations, `go.mod` `replace` and vendored modules bind nothing
  today; the same-named-method caller fix covers Go only (Python and TS still pick the first method of that name).
- **TS.** An arrow component bound to a const is not a function node in the vendored extractor; `describe` inside a plain function
  gets no node; two describes opening on one line parent each other.
- **crg adapter.** A failed later provider drops the agreement table of earlier pairs; `files_parsed` is crg's own stdout count;
  a conform result from a partial fixtures directory passes the gate for the missing languages (document or guard).
- **Port kit export.** For an adopted feature: the files, line ranges, licence text and attribution header a coding agent needs
  to take the module whole. This is the "systematic adopt" the maintainer asked about — a feature, not a skill: the method skill
  `using-comparegraph` already carries the workflow, and the adoption criteria stay the user's.

## Left behind by Plan 7 lane 1 (the maintainer, 2026-10-13: "we can do those as well — nice to have")

- **A text channel in the store.** The mapping-gold improvement task (IMP) doubled `map` recall but stalled on the held-out
  copies at precision@5 0.038 / recall@5 0.149 (target 0.30 / 0.50; `docs/reports/2026-10-14-mapping-gold-first-measurement.md`,
  "After IMP"). Diagnosis measured on the held-out copies: a gold location is inside the top 200 candidates for only 30 to
  47 percent of features, because the store holds only symbol names, paths and community keywords. A feature word that is in no
  symbol name cannot be found by any ranking. The lane: the reference provider also records the tokens of a symbol's doc
  comment and signature (a `doc_tokens` column or table, per node), the build hash gate re-baselined once with the reason
  recorded, `map` and `absent-check` gain a `doc` channel with its own weight tuned on the TUNE split and measured once on
  HOLD-OUT. Stop condition: held-out recall@5 ≥ 0.50 at precision@5 ≥ 0.30, or the measured ceiling written down.
- **A synonym channel.** Absent recall is held at 1/19 by spelling and synonym gaps (`visualisation`/`visualization`,
  `lookup` for `fuzzy-name`): every stricter cutoff that added an absent call added a wrong one. A small, committed synonym
  table per family pack (data, reviewed like the rows) rather than an embedding model, so the verdict stays computable
  without a model. Measured on the same gold; absent precision must stay ≥ 0.95.
- **A larger tuning set.** Three small clones (about 90 gold locations) chose the IMP parameters; the formula sits on a
  plateau on them but that is not proof of transfer. Two more labelled clones in the TUNE split before the next tuning round.
- **Ranking cost on large runs.** The IMP ranking loads every node of a run with its tokens: on a 454k-node run `map` took
  61 s and 677 MB where the FTS path took 12 s and 27 MB. A fix round of lane 1 addresses it (inverted token index, candidate
  pool per term, one index per run shared by `relate`); if the cost cannot be brought near the FTS path it becomes a lane.
- **Short-term prefix match.** The ranking's token rule (a shared prefix of at least four characters that leaves at most two
  of the shorter token) lets `multi` match `multiplier`, `collab` match `collect`, `audit` match `audio` and `recovers` match
  `record`; on the real-product check this one rule explains every wrong top-three hit. Candidate fix: a term shorter than
  six characters matches only whole tokens or tokens that are an inflection of it (the absent check's suffix list); measure on
  the mapping gold before and after.
- **Test detection by name.** Functions named `test_…` inside a non-test file (Rust `#[cfg(test)]` modules) and files named
  `*.test-helpers.ts` or `*.test-support.ts` rank as implementers; the kind prior only sees `test` nodes and test paths.


## Distribution

Measured against the twelve study projects in `docs/reports/2026-10-13-install-surfaces.md`, and following the Rust
tools among them (the maintainer, 2026-10-14: "they don't use npm, so skip; just follow others"). Shipped: release
binaries for five platforms with `SHA256SUMS`, `scripts/install.sh` and `install.ps1` with the checksum check, a Homebrew
tap, the Claude Code plugin (skills plus the MCP server over the installed binary), `setup`/`doctor` for twelve clients
(with a rules block in the agent's instruction file and a Claude Code session-start hook), the MCP Registry entry,
`cargo install --locked` from source (`docs/dev/release.md`). Next:

1. More clients (Zed, Continue) once their config shapes are verified.

Not taken: npm (dropped 2026-10-14), crates.io (nobody asked; the from-source line covers it), an MCPB bundle for the
registry entry, Docker, a VS Code extension, a desktop app, a GitHub Action, Windows package managers.
