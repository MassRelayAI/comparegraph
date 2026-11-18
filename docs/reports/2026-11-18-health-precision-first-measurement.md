# health: first precision measurement (2026-11-18)

Measured with `comparegraph health` at commit 04c414a on a temp store (WSL, never the real store), repo `F:/repos/massrelay` at
head c0c2fbe (89,473 nodes, 246,558 edges, 7,012 files parsed). Sampling: findings sorted by key, 20 taken at evenly spaced
positions (`keys[int((i + 0.5) * N / 20)]`); a kind with 20 or fewer findings is read whole. Each sample was checked against the
source files (the import line and the other occurrences of the name; both function bodies; the file's line count; the stored edges
for coupling). The bar is 0.96. Samples and helpers were throwaway scripts; the counts below are the verdicts.

## Result after the fixes in this lane

| kind | findings | sampled | true | false | precision | state |
|---|---:|---:|---:|---:|---:|---|
| cycles | 13 | 13 (all) | 13 | 0 | 1.00 | ratchet |
| dead_imports | 99 | 20 | 20 | 0 | 1.00 | ratchet |
| duplicate_bodies | 112 | 20 | 20 | 0 | 1.00 | ratchet |
| size_outliers | 378 | 20 | 20 | 0 | 1.00 | ratchet |
| coupling_outliers | 108 | 20 | 20 | 0 | 1.00 | ratchet |
| boundary_violations | 0 | 0 | 0 | 0 | n/a | no rule in massrelay's `comparegraph.toml` |
| dead_code | 5,388 | 20 | 0 confirmed | 0 refuted | under 0.15 | **advisory** (never in the ratchet unless `--ratchet-dead-code`) |

Small samples: 20 of 99 to 378 findings with no false one gives a 95% lower bound near 0.84 (rule of three), so "1.00" means no
false positive was found, not that none exists. Re-measure on the next baseline.

## What the first run got wrong (before the fixes)

| kind | first run | sample | precision | false-positive class | fix |
|---|---:|---:|---:|---|---|
| cycles | 520 | not sampled; one class dominated | | Rust `mod child;` / `use super::` / `use crate::` between a module and the modules it declares are edges in the file graph, not a dependency cycle (507 of 520) | edges between a Rust file and any file under its module directory are left out of the cycle graph; fixture `rust_parent_child_module_pairs_are_not_cycles` (red before); a real sibling cycle is still reported |
| dead_imports | 1,917 | 20 | 0.05 (1 of 20) | a capitalised Rust import is often a trait used only through its methods (`sqlx::Row`, `base64::Engine`, `axum::IntoResponse`, `rsa::...`): 1,818 of 1,821 Rust findings were capitalised | Rust names starting with an upper-case letter are not judged; fixture `rust_trait_imports_and_module_declarations_are_not_findings` (red before). Lowercase Rust names, TypeScript/JavaScript and Python are still judged |
| coupling_outliers | 129 | 20 | 0.90 (18 of 20) | (a) `lib.rs` file fan-out 103 counted `mod x;` declarations; (b) a TypeScript function fan-in of 77 (`actorFor`) was all name-guessed (0.5) call edges, 0 resolved | module declarations are not file coupling; function fan-in/out counts resolved (1.0) calls only; fixture `module_declarations_and_name_guessed_calls_do_not_count_as_coupling` (red before, and red again when the 1.0 filter is reverted) |
| dead_imports | | | | comment text inside an import clause was read as a name (`// R8 wave ... planL0Metering`) | comments are stripped from the clause; unit test |
| dead_code | 500 shown | | | the dead-code query pages at 500, so the ratchet saw only the first page | health pages through all of them |

## Notes per kind

- **cycles**: 13 true. Eleven are value-import cycles; two (`MeetingUploadTab.tsx` and `RecordHereTab.tsx` with
  `meetingSources.ts`) involve a type-only import (`import type`), erased at build time: a cycle in the type graph, harmless at
  run time. The store does not record type-only imports, so they cannot be told apart without re-reading the text; left in.
- **dead_imports**: the 20 sampled are TypeScript/JavaScript/Python names with a single occurrence (the import itself). Four of
  the 20 sit under `tools/_archive` or `docs/_archive`; `[health]` has no path exclude yet.
- **duplicate_bodies**: all 20 have byte-equal normalised bodies, so the detection is right. The classes not worth fixing are
  test helpers (a 27-line `fact` builder in two test modules), mirrored copies (`apps/site/*.js` and `tools/site/assets/*.js`),
  `deploy/{prod,stag}/init-env.sh`, and `tools/_archive`. A path exclude would drop about a fifth.
- **size_outliers**: file line counts match the files exactly; function spans are the parser's. Generated SQL baselines and
  `schema.sql` rank high; a path exclude is the fix.
- **coupling_outliers**: every sampled count was recomputed from the store (distinct importers or resolved callers) and exceeds its
  threshold. Counts are only as good as call resolution.
- **dead_code**: of 20, none could be confirmed dead. Seventeen have the name elsewhere in the repo (methods named `new`, `list`,
  `fmt`, request-body structs used as extractors, types referenced from other files); three are anonymous (`body#5`, `execute#3`,
  `constructor#3`) and cannot be searched. The graph cannot see trait dispatch, extractors, decorators or framework wiring, so
  this kind stays a candidate list. Kept `advisory`: `sections.dead_code.advisory` is true and it is outside the baseline and the check.

## Current counts (massrelay c0c2fbe), highest impact first

cycles 13, dead_imports 99, duplicate_bodies 112, size_outliers 378, coupling_outliers 108, boundary_violations 0 (no rules),
dead_code 5,388 (advisory). Ratchet baseline: 710 keys.

Top of each by impact (`--top 3`): cycles `decide.rs <-> step.rs`, `device_doors.rs <-> device_lookup.rs`,
`people.rs <-> service.rs` (all length 2, ties broken by the files' import fan-in); duplicates a group weighted 704 (members times
lines), then 536; size `crates/mr-server/src/routes.rs` (27,114 lines), `0000_baseline.sql`, `schema.sql`; coupling
`mr-kernel/src/db.rs::tenant_scope` (3,446 callers), `audit.rs::now_iso` (743), `gates.rs::authorize` (682).

## Finding: `build` overflows its stack on this repo with the default stack (not fixed in this lane)

- Repro: `comparegraph --root <ws> add F:/repos/massrelay --name mr`, then
  `COMPAREGRAPH_PROGRESS=1 comparegraph --root <ws> build mr` (Linux, debug build, 7,012 files). Result: `thread '<unknown>' has
  overflowed its stack`, `fatal runtime error: stack overflow`, abort (core dumped), about 2 minutes in. Reproduced on three runs
  across separate stores.
- Where: the last progress line before the abort is `pass resolve:js done`; the abort is in the next stage, the per-file resolution
  (`resolve.rs`, between `resolve:js` and `resolve:per-file`), on an unnamed `par_map` worker thread (Rust's default 2 MiB stack).
  The file is not named: the abort cannot print `CURRENT_FILE`.
- Workaround measured: `RUST_MIN_STACK=8388608` (8 MiB) builds it (3 min 54 s without the parse cache); 32 MiB and 1 GiB also work.
- Not found cheaply: the single file or nesting depth that triggers it. Next step for whoever takes it: give the `par_map` workers
  an explicit stack size (`std::thread::Builder::stack_size`) or bisect `COMPAREGRAPH_THREADS=1` with `RUST_MIN_STACK` between 2 and
  8 MiB, and print `CURRENT_FILE` from a SIGSEGV handler.
