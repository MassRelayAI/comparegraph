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
| module-level calls (Plan 5 A1: now attributed to the file node, or to an `it:` case node in JS/TS; an entry-file heuristic decides whether they rescue dead-code candidates) | a describe-block node for TS test bodies (Plan 6 A1) | calls in `describe()` bodies still have no enclosing node | TS test-body calls are missed (hono gold recall 0.31) |
| return-typed and tuple-unpacked locals (Plan 5 A2: typed from DECLARED return annotations and class fields only; `a, b = f()` only with `-> tuple[A, B]`) | inference from bodies | declarations only, by rule | a factory without an annotation types nothing |
| TS/JSX field types (Plan 5 A2: `decl_types.rs` reads class fields, constructor parameter properties and `new T()` initialisers; field access detected from the AST) | — | done | Go receiver types are the remaining gap (caddy gold recall 0.25; Plan 6 A2) |
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

| a JS/TS test case node keeps its `calls` edge beside the `tests` edge (as Python `test_*` does) | tests edge only | one rule for every language; `calls` shows what the case runs | callers/impact of a function list test cases (they are test-path nodes) |

## Plan 5 (recall and distribution) — 2026-10-13

| decision or known limit | alternative | why | who it hurts |
|---|---|---|---|
| a bare module-level reference binds only an imported or same-file top-level FUNCTION at 1.0, never a method, never at 0.5 | bind methods too | a method name collides with any class | module tables pointing at methods are missed |
| `if __name__` guard matched by prefix (`!= "__main__"` also matches); class spans by line range; nested functions bindable as table targets | AST conditions | cheap | rare false entry marks |
| two same-named methods in one file: the caller lookup by name picks the FIRST as the source of both edges (targets correct) | caller by span | pre-dates A2 | source-side precision on such files — Plan 6 |
| `it:` case nodes keyed by name+line; same-title cases on one line share a key; `test.describe.serial/only` lose the title prefix; Playwright `test.*` members other than skip/only/todo/concurrent/fails are not cases | — | measured on hono | cosmetic |
| a duplicate alias from conditional imports in `__init__` binds the first at 1.0; star/external/own-definition re-export guards untested; the cycle test is vacuous | drop duplicate aliases | small | rare |
| a reused (no-op) run keeps its old limitations even when a new unparseable file appeared; rename/binary/cap/version cases untested | compare walk counters | the graph is identical | stale limitation text |
| `setup`: a bare `--bin` name relies on the client's PATH; dangling symlinks are not followed; `COMPAREGRAPH_NOW` is a test hook in production; serde_json `preserve_order` + `arbitrary_precision` are crate-global (envelope keys now in insertion order; `1e5` round-trips as `1e+5`); the `result_large_err` clippy allow should become a boxed variant | — | shipped | none observed |
| `catalog` prints raw JSON (not an envelope) so it can be redirected; ERROR_CODES is a hand-kept list with no cross-check test; matrix CSVs ignore `--repo` | — | documented | drift risk on a new error code |
| the vault's description lines are template-built from stored facts, never summaries | model-written summaries | deterministic, model-free | thin triage text |
| crash fix: the only unsafe byte-index site was resolve.rs:269; a panic in resolution now fails the run as `Panicked(file)`; pre-loop passes are not file-marked; `show` caps callers/callees at 50 (`--max-callers` 1..2000) and reports totals | — | measured on hermes-agent and openclaw | a panic in a pre-loop pass names "an unknown file" |
| scale: openclaw (38k files) took 1,866 s with every parse a cache hit — resolution is super-linear | indexed lookups, incremental edges | measured in the proof | large monorepos — Plan 6 B1–B3 |
| measured after Plan 5 (five projects, two readers, gold): precision on par or ahead; recall on par Python, ahead Rust, BEHIND TypeScript (0.31 vs 0.56) and Go (0.25 vs 0.65); speed ahead (ratios, loaded host) | — | `docs/reports/2026-10-12-engine-parity.md` | Plan 6 targets |

## Plan 6 (TypeScript/Go recall and scale) — A1, 2026-10-13

| decision or known limit | alternative | why | who it hurts |
|---|---|---|---|
| a `describe`/`suite`/`context`/`test.describe` block IS a node (kind `function`, `describe: outer > inner`, title cut at 80 chars, repeats numbered `#2`); it reverses Plan 5 A3's "describe blocks are not nodes". Calls in its body (and its hooks) belong to it; nested describes and `it:` cases are contained by their parent describe, not the file | titles only | gold scores a site by its smallest enclosing node; 18 of 44 hono sites sat in describe bodies | two describes opening on one line parent each other wrongly; `describe.each/for(...)(…)` is no node and its body's calls stay unattributed; a describe written inside a plain function gets no node |
| a JSX element `<Foo />` / `<Foo>` is treated as a RENDERING CALL (kind `calls`, 1.0) from the smallest enclosing function, case or describe (else the file) to `Foo` when `Foo` is imported or defined in the same file; lower-case and member tags (`<div>`, `<A.B>`) and type-only imports (`import type`, `{ type A }`) bind nothing; no name-only fallback. The two readers marked a JSX element ambiguous (a call? a reference?) | a `refs` kind; no edge | a component that is rendered is used; dead-code and impact need it | an arrow component bound to a const is no function node in the vendored extractor, so its elements belong to the file |
| hooks (`useX()`) get no special case: they resolve as any call | — | verified by a test | — |
| a test file's own scope and a describe body do not rescue a dead-code candidate (`callers_of` ignores `test`-kind and `describe: ` sources — the `test`-kind rule covers the file node of EVERY language's test file, not only TypeScript, and is wanted); an `it:` case still does. This tightens Plan 5 A1: a call at test-file scope used to count because the file node is kind `test`, not `file` | count them | load-time code is not a test | a helper called only from a describe body is listed as a dead-code candidate |
| class fields bound to a function (`request = (…) => {}`) become `method` nodes (own pass; the vendored extractor misses them; `#private` fields are skipped) | edit the vendored file (forbidden) | hono's `request`/`fire` were invisible, 7 gold sites | calls made inside such a body are still unattributed |
| `Foo.bar()` / `foo.bar()` with `foo: Foo`: the class this file defines or imports decides (two `Node` classes no longer collide), its method or its base's via 1.0 `extends` edges. A class that is resolved and, with every base it has, lacks the method binds NOTHING (the name rule no longer lands on another file's class). A local, parameter, catch binding or destructured name that hides the class binds nothing. Only a chain that breaks (a written base that did not resolve: alias re-export, external base) falls back to the old name-unique rule | name-unique only | precision | an aliased re-export (`export { A as B }`) still resolves by the old rule; shadowing is by line range of the scope, not by column |
| the gold scorer falls back to the file node (kind `file` or `test`) when no function or method holds the line; the head-to-head gives crg the same rule (its File node, whose module-level CALLS edges have the file as source) | functions only | module-level test code is the file's | a file-level edge answers any site not inside a function |
| measured (gold): hono precision 0.846→0.920, recall 0.306→0.639; pooled 0.980→0.982 / 0.626→0.677; the other four unchanged | — | `fixtures/gold/FLOORS.json` raised | — |

