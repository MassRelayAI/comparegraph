# Plan 7 candidates — 2026-10-13

Not a plan yet: the scope that waits on Plan 6's re-measure. A plan file follows a brainstorm; this note records what the
maintainer asked and what the lanes left behind, so nothing is re-derived from a transcript.

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
