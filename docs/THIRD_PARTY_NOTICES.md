# Third-party notices

| what | from | licence | where in this tree | taken at |
|---|---|---|---|---|
| the tree-sitter parser layer: `extractor.rs`, `parser.rs`, `resolver.rs`, `registry.rs`, `languages/*.rs` | https://github.com/aouicher/graphmind `crates/graphmind-core/src/` | MIT | `crates/comparegraph/src/providers/reference/vendor/` | 4a90a7e (2026-10-12) |
| ideas only, no REA file or line is copied: the compact advertised output-schema profile (a widening, bounded in bytes), typed bounded error recovery, and the fingerprint-multiset comparison of network transactions with `unknown` instead of `unchanged` when completeness cannot support it. Files: `src/application/output_schema.rs`, `src/application/ui_net.rs`, `RECOVERY` in `src/application/catalog.rs` | https://github.com/morluto/rea `src/contracts/compactOutputSchema.ts`, `src/domain/analysisErrorRecovery.ts`, `src/domain/browserScenarioNetworkComparison.ts`, `src/domain/browserCompleteness.ts` | MIT | `crates/comparegraph/src/application/` (each file's header names the source and commit) | a016743 (2026-11-12) |

Every file under `vendor/` except `vendor/mod.rs` (our wrapper) carries the attribution header. Modifications are listed in the header of the modified file.

The vendored files stay byte-identical. One choice of theirs is overridden from outside them: `vendor/parser.rs` reads `tsx`
with tree-sitter-typescript's TypeScript grammar, which cannot read JSX; the reference provider (`providers/reference/mod.rs`,
`parse_file`) reads `tsx` with the TSX grammar (`LANGUAGE_TSX`) itself and passes the tree to the vendored
`extractor::extract_symbols`, `extractor::extract_call_sites` and `resolver::extract_imports`, the very functions `parse` calls
for `tsx`. The provider version carries the override (`+tsx`), so no parse cache of the other choice is reused.
