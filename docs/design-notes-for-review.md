# Design notes for review

Every provisional (P) choice in the analysis layer (Plan 3), one row each. Nothing here is validated against a real
taxonomy author yet; each row says what would make us change it.

| choice | alternative | why provisional | what would change it |
|---|---|---|---|
| tiers basics / common / extra / candidate (plus `retired`) | a numeric priority | names chosen ad hoc | a real taxonomy author's needs |
| `family propose` group = most common community-label last segment, else `misc` | cluster terms by co-occurrence | cheap first cut | groups judged wrong on real corpora |
| `family propose` occurrence counts double-count keywords that are also symbol tokens | count each source once | affects ranking only | a ranking that visibly misleads |
| feature terms = tokens of group, slug and title; the definition is not used | include definition words | definitions are noisy prose | recall measured too low |
| a feature is touched by a symbol OR a community candidate | require a symbol | communities give recall on module-level features | false `related` bands |
| bands: related at or above 0.5, adjacent at or above 0.2 | calibrated thresholds | arbitrary | thresholds measured on labelled clones |
| a `none` backed by the engine's own search is `derived` | `observed` or `inferred` | the search is the engine's, the judgement is the author's | a reviewer who reads it differently |
| no MCP decide tool; a decision is a CLI act | an MCP tool requiring `by` | keeps adopt/reject a human act | the maintainer's call |
| `relate` over MCP reads stored rows only and does not compute | compute on call | keeps MCP read-only except `claim` | agents needing fresh scores in one call |
| matrix: a stale location shows `unknown*`; prevalence counts comparators only, root excluded | count the root; show the old verdict | a stale cell must not read as evidence | users who want the last verdict visible |
| communities: connected components over import and call file edges, split by directory above 30% of files (minimum 8) | label propagation or Leiden | deterministic and cheap | flat packages (srclight 50%) need a graph split |
| relatedness rows keyed by pack hash (sha256 of the active `<id>\t<title>` lines); reads prefer the current pack | one row per run | a pack edit must re-score without a rebuild | a pack content change that the hash misses (definitions are not hashed) |
| a `none` claim's `--searched` matches whole identifier tokens by prefix (`retry` finds `retry_with_backoff`, `backoff` does not); text with no token is refused | substring search | cheap, FTS-backed, deterministic | an absence claim passes because the stem was wrong; search several stems |
| a location becomes stale only when `comparegraph check` runs | verify on every read | reads stay cheap and side-effect free | a vault or matrix read before `check` shows a stale claim as current |
| relatedness `pack_hash` covers id and title only | hash definitions too | hashing definitions would re-score on every wording tweak | a definition-only edit does not re-score the stored relatedness |
| `add` accepts a non-git folder (lane study, null head and licence) | require git | lets a local folder be studied | no provenance for the clone |
| `propose` hit its `--max` cap on the proof corpus | raise the default | bounded output | candidates beyond the cap are not shown |
| vault descriptions are template-built from stored facts, not summaries | a model-written summary per note | keeps the vault deterministic and model-free | triage by template may prove too thin; revisit then |
| history notes render claim and decision trails as documents (full history also in the JSON export) | trails only in JSON | a reader of the vault sees what was superseded and when | history notes grow with every claim row |
| the method skills are not yet installed for users (no plugin manifest, no entry skill); the vault's AGENTS.md carries only the short workflow | ship skills with the vault | the skills are not packaged yet | agents outside this repo never see the full method |
| superpowers parity gaps for the repo itself: no .claude-plugin manifest, no skills/ folder with an entry skill, no AGENTS.md or CLAUDE.md for contributors, no CHANGELOG or CONTRIBUTING, no CI, no hooks | adopt them now | out of scope for the vault task | contributors and agents get no guidance from the repo |
| the core code graph is NOT on par with mature engines (measured on code-review-graph's source: 3 edge kinds, contains/imports/calls; 24% of call edges are name-only guesses at confidence 0.5; relative imports only; one-hop queries; no incremental build) | claim parity | an honest measurement | parity path: the code-review-graph adapter plus multi-hop traversal and impact tools |

## Open items carried from Plans 1 and 2

- srclight-style flat packages: one directory holds the whole package, so the directory split cannot divide it; communities
  there need a graph split.
- The Perl grammar resolves only via Cargo.lock (the upstream version was yanked).
- `parse().unwrap_or` fallbacks in the store hide malformed stored values instead of reporting them.
- The repo head is not refreshed on each run.
- git calls could set `GIT_OPTIONAL_LOCKS=0` and disable fsmonitor.
- `check` does not report leftover `running` runs.
- History was squashed into one clean root commit on 2026-10-12; the old history branch was deleted the same day.
- Non-code files become symbols: the taken parser layer records a Dockerfile `FROM python:3.12-slim` as a `class` (seen on the srclight proof vault, 2026-10-12). Decide whether config/infra languages (Dockerfile, YAML, TOML, HCL, CSS, HTML) should produce symbols at all, or only file nodes, so communities, search and `map` are not polluted.
- Wording in generated descriptions is not pluralised ("1 claims", "1 files"); cosmetic, template-level.

Task 6 (2026-10-12): `typed_receiver_method` resolves `r.m()` from a type declared in the caller (`Square r`, `r: Square`), not inferred. The vendored Java extractor reports no call site for `new T()`; Rust integration tests that name the package crate (`use demo::util::x`) get no `imports` edge (needs Cargo.toml crate-name resolution). Both are recorded in fixtures/{java,rust}/NOTES.md.

## Plan 4 (engine depth)

| decision or known limit | alternative | why | who it hurts |
|---|---|---|---|
| module-level calls have no file-level caller | a file node as caller | the largest remaining miss class, but it changes `entry-points` and `dead-code` | calls under `if __name__`, decorators and module tables are missing |
| return-typed and tuple-unpacked locals are not typed (`ctx = setup(); ctx.run()`, `a, b = f()`) | return-type propagation | needs an inference pass | method calls on such locals bind nothing |
| TS/JSX field types are not read (`this.reader.m()`) | field type table | not built | TypeScript and JSX call recall stays behind |
| a re-exported function can bind a same-named method at 0.5 | follow re-exports through `__init__` | not built | rare false 0.5 edges |
| an imported name used as an object binds any same-named symbol in the module file | require a class | cheap rule | a function and an object of one name in a module |
| `class_method` picks the first same-named method when a file has two | pick by receiver type | the span rule has no type | edges to the wrong class in a file with two |
| naming-convention test links are not restricted to the same language family | restrict | not done | a `test_x.py` can link a same-named non-Python file |
| Java `new T()` has no call site (vendored extractor) | patch the vendored file | vendored files stay byte-identical | constructors in Java are missing |
| TOML emits `Cargo.toml::package` nodes | skip config keys | taken parser layer | config noise in search |
| a trait method and its impl method share one qualified name | disambiguate by owner | qualified names are path plus name | one node stands for both |
| interface-extends-interface is not covered; Go has no inheritance edges | extend the extractor | not built | thinner `extends` in TypeScript interfaces and Go |
| an incremental build rewrites the whole graph each run | reuse the run when nothing changed | simple and correct | a no-op build is slower than it could be |
| entry-point exported-ness is a heuristic | read export syntax per language | cheap | some exported symbols look unexported and the reverse |
| dead-code confidence is `unknown` (dynamic dispatch is invisible) | a graded confidence | honest | candidates, never proof |
| MCP default `min_confidence` 0.5 includes guessed hops (labelled in evidence) | default 1.0 | recall first, the label says so | an agent that ignores the label |
| name-only binds (a bare call with no import, a unique-name base class) are capped at 0.5 and never bind a method; a base imported from outside the repo binds nothing | bind at 1.0 | a name match is not a proof | none beyond the lost recall |

