---
title: Engine parity with code-review-graph, measured on three projects
date: 2026-10-12
valid_until: 2027-01-13
lane: plan-4
verdict: calls behind on recall (gold 0.63 vs 0.71 pooled; Go 0.25 vs 0.65, TS 0.31 vs 0.56), ahead on precision (0.98 vs 0.95), on par for Python, ahead on Rust+TS; symbols and imports on par; tests behind; build 1.6-3.4x faster; ahead on cross-repo comparison; absent - embeddings, flows, diff review
status: decided
---

Method: `scripts/compare-engines.sh <scratch> <comparegraph-binary> <project>...` (kind mapping documented in its header).
Each project is copied into scratch; `code-review-graph` 2.3.9 (PyPI, `build`, default post-processing) and CompareGraph at
`2b4c917` (release build, `build`) run on the same copy at `nice -n 10`. Seed 7. Times are the median of 3 cold builds on a
shared 40-core host. Numbers are one run on one machine, not a benchmark suite.

Projects (upstream, commit): graphmind https://github.com/aouicher/graphmind @4a90a7e (Rust + TypeScript) ·
srclight https://github.com/srclight/srclight @23924ff (Python) · code-review-graph https://github.com/tirth8205/code-review-graph @6b12d11 (Python, TypeScript, fixtures in many languages).

## What is compared, and its limit

Nodes are keyed `(relative path, symbol name)`. An edge is keyed `(kind, source key, target key)`. code-review-graph
stores a call target as a bare name when it cannot bind it (`set`, `get`); those cannot be compared by target path.
"Resolved" below means its target equals an indexed node. Only 16 to 35 percent of its call edges are resolved
(graphmind 3,434 of 21,539; srclight 3,663 of 10,875; code-review-graph 15,297 of 43,386, from its `target_resolution` column),
so every edge Jaccard here is over the comparable minority. The name-level Jaccard (target name only) is shown beside it.
Inheritance targets that are bare names in code-review-graph are bound when exactly one class or type has that name.

## Build time (seconds, wall clock)

| project | code-review-graph full | code-review-graph parse only | CompareGraph cold | CompareGraph unchanged rebuild | speed-up vs full |
|---|---|---|---|---|---|
| graphmind (251 files) | 5.59 | 4.47 | 1.76 | 1.18 | 3.2x |
| srclight (100 files) | 5.46 | 4.33 | 0.91 | 0.57 | 6.0x |
| code-review-graph (470 files) | 16.86 | 12.72 | 8.04 | 6.22 | 2.1x |

code-review-graph's figure includes Python start-up and its flows and community post-processing; we include communities.
Against its parse-only mode the factor is 2.5x, 4.8x, 1.6x.

## Nodes (Jaccard over (path, name))

| project | symbols function+method+class+type | both / crg-only / ours-only | function | method | class | file |
|---|---|---|---|---|---|---|
| graphmind | 0.946 | 1,987 / 24 / 90 | 0.980 | 0.822 | 0.704 | 0.944 |
| srclight | 0.988 | 1,649 / 9 / 11 | 0.945 | 0.842 | 0.866 | 0.977 |
| code-review-graph | 0.854 | 7,239 / 346 / 894 | 0.722 | 0.844 | 0.808 | 0.910 |

Per-kind Jaccards differ because the engines draw kind lines differently (we have a `type` kind, 100 / 13 / 138 nodes, where it
puts structs, enums and interfaces under Class; it has a method only as a Function with a parent), so the aggregate is the
fair number. Counts per kind are in each project's JSON from the script. Read from the two sets: our 894 extras in the third project are mostly a minified bundle it skips
(761 in `d3.v7.min.js`) and config keys we store as `type` nodes (`Cargo.toml`, `action.yml`, `pyproject.toml`); its extras
are test classes and functions in `tests/` (about 300 of 346), per-case `it(...)` nodes, and in graphmind the 24 React
components in `.tsx` files that we do not extract.

## Edges (Jaccard over resolved edges; name-level in brackets)

| edge kind | graphmind | srclight | code-review-graph |
|---|---|---|---|
| calls | 0.507 [0.285] | 0.700 [0.503] | 0.714 [0.471] |
| imports (file to file) | 0.441 [0.139] | 0.384 [0.062] | 0.901 [0.149] |
| extends + implements | 0.938 (15 vs 16 edges) | 0.667 (9 vs 6) | 0.730 (32 vs 32) |
| tests (test to tested symbol) | 0.001 | 0.435 | 0.440 |
| contains, by edge | 0.715 | 0.581 | 0.309 |
| contains, by contained symbol | 0.944 | 0.988 | 0.855 |

Resolved call edges: both / only code-review-graph / only ours = graphmind 2,328 / 424 / 1,841; srclight 3,006 / 181 / 1,105;
code-review-graph 11,277 / 304 / 4,216. Our call edges below confidence 1.0 (name guesses): 563 of 4,171 (13.5 percent),
683 of 4,114 (16.6), 1,848 of 15,500 (11.9). code-review-graph marks every edge EXTRACTED at 1.0.
`contains` differs by convention (it nests methods under their class; we link every symbol to its file), hence the by-symbol row.
`tests` is not like for like: graphmind's tests are Rust `#[cfg(test)]` functions in the same files, which we do not treat as
tests at all (only test files); code-review-graph also makes a node per `it(...)` case.

## The 20-edge call samples, read by hand

For each project, 20 random (seed 7) resolved call edges that code-review-graph has and we lack, each judged by reading the
call site and the callee's definition. The script writes these to `<project>.json` (`call_sample`).

| project | our miss | their false edge | ambiguous |
|---|---|---|---|
| graphmind | 19 | 1 | 0 |
| srclight | 19 | 1 | 0 |
| code-review-graph | 18 | 0 | 2 |
| total of 60 | 56 | 2 | 2 |

Our misses are systematic, not noise: `Type::new()` associated calls (Rust, 16 of the 19 graphmind cases), `Indexer(...).index()`
chained on a constructor, a method on a variable typed by `with X as wdb` or by an annotation (`_db: Database | None`), a module
alias (`build_mod.get_platform_variants`), a call at module level (no enclosing function; we record no file-level caller),
decorator arguments, JSX components, Dart and Solidity constructors and same-file calls, and calls made from `it(...)` test cases
that we have no node for. Its two false edges: `_cache.clear()` on a Map bound to the class's own `clear`, and `locked.cancel_flags.get()`
on a HashMap bound to `ProjectConfig::get`. The two ambiguous ones are an untyped PHP `$service?->search()` and a class instantiated
inside a nested class body.

The reverse check (a smaller sample, 10 per project from the edges only we have; stored in `cg_only_call_sample`):

| project | correct | false | ambiguous |
|---|---|---|---|
| graphmind | 4 | 6 | 0 |
| srclight | 3 | 5 | 2 |
| code-review-graph | 0 | 5 | 5 (minified d3) |
| total of 30 | 7 | 16 | 7 |

So our surplus edges are mostly wrong. The false ones are name guesses on standard-library or builtin methods bound to a repo
function of the same name: `.is_empty()`, `row.get()`, `.open()`, `conn.execute()`, `.decode()`, `self.conn.commit()` bound to
`Database.commit`, `hunk_re.match()`, `subprocess.run()`; and a function calling `db.search_symbols()` bound to itself
(`server.py::search_symbols -> server.py::search_symbols`, at 1.0). Several are at confidence 1.0, so the confidence column does not
yet protect a reader. code-review-graph's own false edges in the part we share were not measured.

## Feature parity

| question | code-review-graph | CompareGraph |
|---|---|---|
| search symbols | yes: FTS5 plus embeddings, `search.py`, `main.py:498` | partial: FTS only, `application/search.rs:102`, verb `search` |
| callers and callees | yes: `query_graph` (`main.py:399`) | yes: verb `show`, `application/show.rs:26` |
| multi-hop traversal | yes: `traverse_graph` (`main.py:1224`) | yes: verb `traverse`, `graph.rs:313` |
| impact (blast radius) | yes: `graph.py:2745`, `get_impact_radius` | yes: verb `impact`, `graph.rs:337` |
| related tests | yes: `query_graph tests_for`, TESTED_BY edges | partial: verb `tests-for`, `graph.rs:420`; test files only |
| entry points | yes: `flows.py:156` | yes: verb `entry-points`, `graph.rs:536` |
| dead code | yes: `refactor.py:235` | yes: verb `dead-code`, `graph.rs:556` |
| communities | yes: `communities.py:797` | yes: verb `communities`, `communities.rs:110` |
| incremental build | yes: git-diff driven, `incremental.py:1889` | partial: per-file parse cache (`build.rs`, cache_hits); the graph is rewritten each run |
| embeddings / semantic search | yes: `embeddings.py:1157` | no |
| execution flows, diff-aware review (`detect_changes`, `get_review_context`) | yes: `main.py:888`, `main.py:454` | no |
| hub, bridge, knowledge-gap, refactor tools | yes: `main.py:1088` onward | no |
| multi-repo comparison | partial: `cross_repo_search` (`main.py:1271`), a search | yes: verbs `matrix`, `relate`, `family` (`matrix.rs:42`) |
| claims and evidence | no | yes: verb `claim`, `claim.rs:138`, citations checked |
| vault / wiki export | partial: markdown wiki from communities, `wiki.py:204` | yes: `export_vault`, `vault.rs:103` |
| MCP tools | 30 (`@mcp.tool` count, `main.py`) | 17 (`tools/list` of `comparegraph mcp`) |

## Verdict per area

| area | verdict | what would close it |
|---|---|---|
| symbol extraction | on par (Jaccard 0.85 to 0.99) | nothing needed; skip minified bundles and stop storing config keys as `type` |
| containment | on par by symbol (0.86 to 0.99); different shape | nothing |
| inheritance | on par on small counts (15 to 32 edges per project; 0.67 to 0.94) | the 3 srclight `extends` edges it binds and we lack (not diagnosed); React components in `.tsx` |
| call recall | behind: 424, 181 and 304 resolved edges lacking; 56 of 60 sampled were our misses | resolve `Type::assoc()` and `Type(...).m()` chains, annotated and `with ... as` receivers, module aliases, a file-level caller for module code, JSX components |
| call precision | behind on edges we add alone: 16 of 30 sampled false, 7 correct | stop binding builtin and standard-library method names (`get`, `is_empty`, `execute`, `commit`, `decode`, `run`) on untyped receivers; reject an edge from a function to itself unless the receiver is `self`; lower such guesses below 1.0 |
| imports | behind on Python and Rust (0.44 and 0.38); on par on TypeScript-heavy code (0.90) | probably package-absolute Python imports and Rust `use crate::` / workspace crates (hypothesis, not diagnosed) |
| test links | behind (0.00 to 0.44) | treat Rust `#[cfg(test)]` and `#[test]` as tests; nodes for `it`/`describe` cases; link a test to what it calls through constructors |
| build speed | ahead: 2.1x to 6.0x cold, 1.6x to 4.8x against parse-only | not applicable |
| incremental build | behind: its update is git-diff driven; ours reparses nothing but rewrites the whole graph | write only changed files' rows |
| semantic search, flows, diff review, hub and refactor analyses | behind: absent | each is a feature, not a fix; none is planned in Plan 4 |
| multi-repo comparison, claims, vault | ahead: absent in code-review-graph | nothing |

## Caveats

- One host, one run set, medians of 3; the host was shared. Python start-up is inside its numbers.
- Edge Jaccards cover only the comparable minority of its call edges (16 to 35 percent resolved).
- The classifications are by one reader and the 30 reverse-sample cases include judgement calls; the ambiguous class is kept
  rather than forced.
- Both engines include vendored, minified and fixture files; no filtering was applied to either side.
- Nothing here measures query latency or answer quality on real tasks.

## After the precision and recall pass (2026-10-13)

The sections above are the first measurement and are left as written. This section re-measures after the precision pass (`4d14fee`:
unknown-receiver and builtin-name calls bind nothing, scoped self-recursion, max-confidence dedup, best-confidence paths) and the
recall pass (`4e00187`: same-crate and workspace Rust types and crate-name paths, Python declared types and import aliases, Rust
`#[test]` and `#[cfg(test)]`, `it()` bodies, minified files skipped).

Method, unchanged from the first run: `scripts/compare-engines.sh` with code-review-graph 2.3.9 from the same venv, a fresh copy of
the three upstream commits, CompareGraph release build at `4e00187`, `nice -n 10`, seed 7, medians of 3 cold builds on the shared host.
Samples: 20 resolved call edges only code-review-graph has (all three projects; srclight has only 14 left, so all 14 were read) and
the first 10 of the script's 20 edges only we have. Every sampled call site was read with its callee's definition.
The first-run reverse sample was drawn as 10; this one is the first 10 of a 20-draw, a different random subset.

### Jaccards and edge counts, before to after

| measure | graphmind | srclight | code-review-graph |
|---|---|---|---|
| symbols (function+method+class+type) | 0.946 to 0.946 | 0.988 to 0.988 | 0.854 to 0.939 |
| calls, resolved | 0.507 to 0.712 | 0.700 to 0.969 | 0.714 to 0.960 |
| imports, file to file | 0.441 to 0.647 | 0.384 to 0.904 | 0.901 to 0.901 |
| tests | 0.001 to 0.118 | 0.435 to 0.605 | 0.440 to 0.512 |
| extends + implements | 0.938 to 0.938 | 0.667 to 0.667 | 0.730 to 0.730 |
| call edges both | 2,328 to 2,487 | 3,006 to 3,173 | 11,277 to 11,196 |
| call edges only code-review-graph | 424 to 265 | 181 to 14 | 304 to 385 |
| call edges only ours | 1,841 to 739 | 1,105 to 89 | 4,216 to 78 |
| our call edges below 1.0 (the 0.5 share) | 563 of 4,171 (13.5 pct) to 6 of 3,228 (0.2) | 683 of 4,114 (16.6) to 9 of 3,262 (0.3) | 1,848 of 15,500 (11.9) to 51 of 11,274 (0.5) |

The symbol change in the third project is the minified bundle: 894 extras fell to 123 once `d3.v7.min.js` and its kind were skipped
(`skipped_minified`). Our resolved call edges fell by 21 to 27 percent in total while "both" rose in two projects; the third lost 81 shared edges
(not diagnosed). The imports gain in
srclight is package-absolute Python paths under `src/` (`from srclight.server import ...`): a dotted path of two or more names now
resolves to the one repo file that ends with it. graphmind imports: no edge only code-review-graph has remains (303 of 303 shared);
our 165 extras were not sampled. The remaining 83 in the third project are not diagnosed.

### Build time (seconds, median of 3 cold builds; final run)

| project | code-review-graph full | CompareGraph cold | unchanged rebuild | speed-up | first run: cold, speed-up |
|---|---|---|---|---|---|
| graphmind | 6.28 | 2.57 | 2.09 | 2.4x | 1.76, 3.2x |
| srclight | 5.97 | 1.48 | 1.01 | 4.0x | 0.91, 6.0x |
| code-review-graph | 18.95 | 6.50 | 4.34 | 2.9x | 8.04, 2.1x |

The host was busier for this run (code-review-graph's own times rose 9 to 12 percent against the first run), and the run just
before it, one small change earlier, gave CompareGraph 2.24 / 1.20 / 6.15 s. The new Rust and Python context passes cost roughly 0.5 to 0.8 s on graphmind and 0.3 to 0.6 s on
srclight; the third project got faster because the bundle is no longer parsed. We stay ahead, by a smaller margin on the first two.

### Samples read by hand

Edges only code-review-graph has (54 read: 20, 14 and 20):

| project | our miss | their false edge | ambiguous |
|---|---|---|---|
| graphmind | 12 | 8 | 0 |
| srclight (all 14) | 13 | 1 | 0 |
| code-review-graph | 18 | 1 | 1 |
| total of 54 | 43 | 10 | 1 |

Before: 56 of 60 were our misses and 2 their false. The 43 misses now split into: 10 calls at module level (`main()` under
`if __name__`, decorator arguments, `LANGUAGES = {... LanguageConfig(...)}`: no function encloses them, and a file-level caller
would change `entry-points` and `dead-code`, so it was not added); 10 on a local whose type comes from a function return or a tuple
unpack (`ctx = setup(); ctx.dispatch()`, `root, store = _make_repo()`); 13 in TypeScript and TSX (JSX components, `this.reader.m()`
field types, an imported instance, a closure parameter, a function nested in a component); 9 in languages whose extractor we do not
extend (Dart 5, Objective-C, Scala, ReScript, a notebook); 1 in a file with two classes of the same method name.
Of their 10 false edges, 9 are library methods bound to a repo method of that name (`get` 4 on maps and a database row, `HashMap::remove`, `Sha256::update`,
tree-sitter `Parser::parse` bound to our `parse`, `ignore`'s `.build()`, an emitter's `dispose`) and one is a local callable
(`is_available = getattr(...); is_available()`), the same class as before. Their ambiguous one is an ansible `notify:` handler
link in a YAML fixture, which is a reference rather than a call.

Edges only we have (30 read: 10 each):

| project | correct | false | ambiguous |
|---|---|---|---|
| graphmind | 8 | 0 | 2 |
| srclight | 10 | 0 | 0 |
| code-review-graph | 8 | 2 | 0 |
| total of 30 | 26 | 2 | 2 |

Before: 7 correct, 16 false, 7 ambiguous. The two false are a bare call to a re-exported function bound to a same-named method in
another file at 0.5 (`from code_review_graph.tools import get_impact_radius`, twice). The two ambiguous are enum-variant
constructors bound to their enum (`EmbedError::NotConfigured(..)`). The reading found and fixed three defects of our own in
earlier iterations of this pass, each now pinned by a test: a Rust variable named like a function (`let hooks_dir = ...;
hooks_dir.join()`) was taken for a type and bound to the function; a Python local named like an imported module
(`daemon = env["d"]; daemon.run()`) bound to the module; a parameter called by name (`normalise(x)`) bound to a same-named function
in another file. One more shape is left alone on purpose: a same-file function with the name of a pytest fixture parameter stays bound
(`def test_x(serve): serve(...)`), because the parameter is that fixture; this keeps 32 edges code-review-graph also has.

### Verdict per area, now

| area | before | now | what would close the rest |
|---|---|---|---|
| symbol extraction | on par | on par (0.946, 0.988, 0.939) | nothing needed; config keys are still `type` nodes |
| containment | on par by symbol | unchanged | nothing |
| inheritance | on par on small counts | unchanged (0.938, 0.667, 0.730) | the 3 srclight `extends` and 12 in the third project, not diagnosed |
| call recall | behind (56 of 60 misses) | on par for Python (srclight 14 edges lacking of 3,187; 0.969), close for the third project (385 of 11,581; 0.960), behind for Rust and TS/JSX (graphmind 265 of 2,752; 0.712) | return-type propagation for locals (`x = f()` where `f` is annotated, `a, b = f()`), field types (`self.cache`, `this.reader`), JSX components, a file-level caller for module code, Dart/Go/ObjC constructors |
| call precision | behind (16 of 30 false) | ahead of its own earlier state, and the surplus is now mostly right: 2 false, 2 ambiguous, 26 correct of 30; its own false edges (10 of 54 sampled) remain | none claimed: the sample is 30 (26 correct, 2 false, 2 ambiguous); the two false ones are re-exports (`from pkg import f` where `pkg/__init__` re-exports) |
| imports | behind (0.44, 0.38) | recall on par: 0 of 303, 19 of 198 and 83 of 841 lacking; Jaccard 0.647, 0.904, 0.901 (graphmind lowered by 165 extras we add, not sampled) | follow Python re-exports through `__init__`; sample the Rust extras |
| test links | behind (0.00 to 0.44) | still behind (0.118, 0.605, 0.512), up in all three | per-case `it`/`describe` nodes (it makes one per case; we attribute to the file); module-level and fixture-parameter calls from tests |
| build speed | ahead 2.1x to 6.0x | ahead 2.4x to 4.0x | not applicable |
| incremental build, semantic search, flows, diff review, hub and refactor analyses | behind | unchanged | each is a feature, not a fix |
| multi-repo comparison, claims, vault | ahead | unchanged | nothing |

Caveats of this section: one host, one final run (timings vary 5 to 15 percent run to run, see above); the 54 + 30 classifications are
by one reader; the re-measure was run several times while the pass was finished, and the numbers above are from the last run at
`4e00187` only; the surplus that remains in graphmind (739 edges only we have) was sampled at 10.

## After the review fixes (2026-10-13)

The whole-branch review of `7c249d6` found three name-only paths that still bound at full confidence: a receiver "typed" by any
adjacent capitalised token (`new Wrapper(inner); inner.run()` bound `run` to `Wrapper`), the repo-wide unique bare-name bind (1.0, and
onto methods), and the inheritance fallback by unique repo name for a base imported from outside the repo. The fix (`3afed80`):
declared-type shapes only for receivers, the bare-name bind capped at 0.5 and never onto a method, an externally imported base
records no edge. The parse-cache key now includes a hash of the vendored parser sources.

Method as above: `scripts/compare-engines.sh`, code-review-graph 2.3.9, fresh copies of the three upstream commits, release build at
`3afed80`, `nice -n 10`, seed 7. **These are the script's automated numbers only. No edge was read or classified by hand in this
note; the 54 + 30 hand classifications above describe `4e00187` and were not redone.**

| measure | graphmind | srclight | code-review-graph |
|---|---|---|---|
| calls, resolved Jaccard | 0.712 to 0.713 | 0.969 to 0.969 | 0.960 to 0.963 |
| imports Jaccard | 0.647 to 0.647 | 0.904 to 0.904 | 0.901 to 0.901 |
| tests Jaccard | 0.118 to 0.119 | 0.605 to 0.605 | 0.512 to 0.513 |
| our resolved call edges | 3,228 to 3,225 | 3,262 to 3,262 | 11,274 to 11,286 |
| call edges only code-review-graph / only ours | 265 / 739 to 265 / 736 | 14 / 89 to 14 / 89 | 385 / 78 to 364 / 69 |
| our call edges below 1.0 (the 0.5 share) | 6 (0.2 pct) to 66 (2.0) | 9 (0.3) to 14 (0.4) | 51 (0.5) to 199 (1.8) |

Reading: recall did not move (the Jaccards are within 0.003), and the "only ours" surplus did not grow. The 0.5 share rose because
bare-name binds that were 1.0 are now 0.5: that is the intended change (a name match is no longer reported as certain), and a
`--min-confidence 1.0` query drops them. Cold build medians on the busy host: 2.30 / 1.12 / 5.16 s against code-review-graph's
6.03 / 5.34 / 16.67 s.

## After Plan 5 — five projects, two readers, a gold standard (2026-10-13)

The sections above are left as written. This section re-measures at main `1dba411` (release build, not rebuilt), code-review-graph
2.3.9, five projects, `scripts/compare-engines.sh --sample-size 100 --gold 150 --gold-seed 11`, seed 7, three cold builds per
engine per project alternated (ours, crg, crg-raw, ours, ...) at `nice -n 10`, medians. Run started 2026-10-13T02:36:57+07:00.
Projects (upstream, commit): graphmind https://github.com/aouicher/graphmind @4a90a7e · srclight https://github.com/srclight/srclight
@23924ff · code-review-graph https://github.com/tirth8205/code-review-graph @6b12d11 · hono https://github.com/honojs/hono @552ea02
(TypeScript-first, new) · caddy https://github.com/caddyserver/caddy @d293dcf (Go, new). Gold labels are committed under
`fixtures/gold/<project>/calls.jsonl` (draw method and rubric in `fixtures/gold/README.md`).

### Automated comparison

Symbol = function+method+class+type by (path, name); calls, imports, tests are resolved distinct edges.

| project | symbol J | call J (resolved) | import J | test J | 0.5 share (our call edges below 1.0) | ours cold s | crg cold s | ratio crg/ours |
|---|---|---|---|---|---|---|---|---|
| graphmind | 0.9457 | 0.7411 | 0.6474 | 0.1185 | 66 of 3331 (2.0 pct) | 3.24 | 5.96 | 1.84x |
| srclight | 0.988 | 0.9695 | 0.904 | 0.6052 | 14 of 3271 (0.4 pct) | 1.61 | 5.51 | 3.42x |
| code-review-graph | 0.9352 | 0.9647 | 0.9013 | 0.5221 | 60 of 11575 (0.5 pct) | 7.60 | 17.03 | 2.24x |
| hono | 0.191 | 0.0116 | 0.9413 | 0.0 | 909 of 3722 (24.4 pct) | 6.93 | 11.30 | 1.63x |
| caddy | 0.7772 | 0.2412 | 0.0 | 0.1053 | 698 of 2088 (33.4 pct) | 2.68 | 8.93 | 3.33x |

Speed caveat, verbatim: the host (40 cores) ran at load 37-73 during the whole run because other sessions shared it; absolute seconds
are inflated and noisy, the alternation keeps the ratio fair — read the ratios only. The 2026-10-12 table was taken on a different load.

Resolved distinct edges (crg / ours / both): calls graphmind 2,752 / 3,328 / 2,588 · srclight 3,187 / 3,271 / 3,179 ·
code-review-graph 11,581 / 11,575 / 11,370 · hono 5,267 / 3,463 / 100 · caddy 5,105 / 2,088 / 1,398. Imports (crg / ours / both):
303 / 468 / 303 · 198 / 179 / 179 · 841 / 758 / 758 · 937 / 882 / 882 · 0 / 389 / 0. Unchanged rebuilds reused the cache on all five.

### Inter-reader agreement

Two independent readers, neither saw the other's labels. Cohen's kappa over the label sets: crg-only (our_miss / their_false / ambiguous), ours-only (correct / false / ambiguous), gold: `is_call` (yes/no) and, over the rows both call `yes`, the target (path separators normalised, `#n` ignored, `external` and `unknown` as labels).

| project | crg-only raw / kappa (disagree of n) | ours-only raw / kappa (disagree of n) | gold is_call raw / kappa (disagree) | gold target raw / kappa (disagree of both-yes) |
|---|---|---|---|---|
| graphmind | 0.75 / 0.58 (25 of 100) | 0.99 / 0.90 (1 of 100) | 1.00 / 1.00 (0) | 0.98 / 0.98 (2 of 130) |
| srclight | 1.00 / 1.00 (0 of 8) | 0.99 / 0.85 (1 of 92) | 1.00 / 1.00 (0) | 0.98 / 0.96 (3 of 134) |
| code-review-graph | 0.95 / 0.86 (5 of 100) | 0.95 / 0.88 (5 of 100) | 0.99 / 0.96 (1) | 1.00 / 1.00 (0 of 137) |
| hono | 0.84 / 0.62 (16 of 100) | 0.96 / 0.84 (4 of 100) | 1.00 / 1.00 (0) | 0.94 / 0.89 (9 of 146) |
| caddy | 0.98 / 0.96 (2 of 100) | 0.86 / 0.10 (14 of 100) | 0.97 / 0.43 (5) | 0.95 / 0.92 (7 of 143) |
| pooled | 0.88 / 0.76 (48 of 408) | 0.95 / 0.77 (25 of 492) | 0.99 / 0.94 (6 of 750) | 0.97 / 0.95 (21 of 690) |

Reading: gold labelling is reliable (kappa 0.94 to 0.95 pooled). The sample classifications are weaker on two cells. caddy ours-only
has raw agreement 0.86 but kappa 0.10: 96 of 100 rows are `correct` for reader 1, so chance agreement is near 1 and kappa collapses;
the real disagreement is that reader 2 marked 11 Go type conversions (`HandlerFunc(...)`, `WeakString(...)`) `ambiguous` where reader 1
called them correct calls. The crg-only kappas of graphmind (0.58) and hono (0.62) come from the same split on JSX components and
interface/trait-object receivers (reader 2 `ambiguous`: 24 rows in graphmind, 10 in hono). Gold disagreements are mostly one rubric
point: a call through a local closure or fixture-returned function (reader 1 named the closure, reader 2 said `unknown`) and dynamically
assigned `app.get` in hono (reader 1 named the declaring property, reader 2 `unknown`).

#### Five example disagreements per project (R1 / R2 labels, then their notes)

**graphmind**

- crg-only `crates/graphmind-desktop/src/pages/Setup.tsx:888` `StepAddProject`: R1 `our_miss` / R2 `ambiguous`. R1: JSX component use in same file. R2: JSX.
- ours-only `crates/graphmind-config/src/config.rs:77` `LicenseConfig`: R1 `false` / R2 `ambiguous`. R1: caller McpConfig::default never mentions LicenseConfig. R2: LicenseConfig::default().
- gold `crates/graphmind-cli/tests/cli_claude_hooks_e2e.rs:358` `git`: R1 `crates/graphmind-cli/tests/cli_claude_hooks_e2e.rs::git` / R2 `unknown`. R1: local closure git. R2: git is a local closure in the test.
- crg-only `crates/graphmind-embeddings/tests/embedding_pipeline.rs:561` `embed_batch`: R1 `their_false` / R2 `ambiguous`. R1: engine is a Voyage engine built from config; target is the MockEngine impl. R2: trait-object.
- gold `crates/graphmind-desktop/src/pages/Integrations.tsx:40` `refresh`: R1 `crates/graphmind-desktop/src/hooks/useClients.ts::refresh` / R2 `unknown`. R1: (no note). R2: refresh is destructured from a hook return.

**srclight**

- ours-only `src/srclight/server.py:3519` `close`: R1 `correct` / R2 `false`. R1: call text present, receiver is the imported module/instance of the target. R2: _workspace_db.
- gold `tests/test_qualified_name_graph.py:180` `serve`: R1 `tests/test_qualified_name_graph.py::_serve` / R2 `unknown`. R1: fixture returns inner _serve. R2: serve is a pytest fixture returning a closure.
- gold `tests/test_scan_gaps.py:419` `real`: R1 `src/srclight/indexer.py::_get_parser` / R2 `unknown`. R1: real = Indexer._get_parser. R2: real is a local variable holding the original method.

**code-review-graph**

- crg-only `tests/fixtures/sample.sv:41` `Adder`: R1 `our_miss` / R2 `ambiguous`. R1: SystemVerilog module instantiation. R2: SystemVerilog.
- ours-only `code_review_graph/eval/runner.py:94` `run`: R1 `ambiguous` / R2 `false`. R1: module-level dict reference agent_baseline.run (line 37); shown subprocess.run is stdlib. R2: subprocess.run.
- gold `code_review_graph/visualization.py:1695` `nodeToCommunity.get`: R1 `external` / R2 `not a call`. R1: (no note). R2: JavaScript.
- crg-only `tests/fixtures/sample.sol:208` `mulPrecise`: R1 `our_miss` / R2 `ambiguous`. R1: call resolves to a definition in the repo. R2: Solidity.
- ours-only `code-review-graph-vscode/src/views/treeView.ts:32` `StatsItem`: R1 `false` / R2 `ambiguous`. R1: caller getChildren (32-43) never creates StatsItem; that is another class. R2: same-named.

**hono**

- crg-only `src/jsx/streaming.test.tsx:96` `Content`: R1 `our_miss` / R2 `ambiguous`. R1: call resolves to a definition in the repo. R2: Content: cannot tell statically.
- ours-only `src/jsx/dom/index.ts:20` `useEffect`: R1 `ambiguous` / R2 `false`. R1: export/re-export specifier, not a call. R2: line.
- gold `adapters/aws-lambda/test/index.test.ts:84` `app.get`: R1 `src/hono-base.ts::get` / R2 `unknown`. R1: Hono.get is declared at hono-base.ts:104 and assigned dynamically in the constructor loop (line 131). R2: app.get is dynamically assigned in HonoBase.
- crg-only `src/adapter/bun/websocket.test.ts:11` `send`: R1 `our_miss` / R2 `their_false`. R1: call resolves to a definition in the repo. R2: send: claimed target wrong: builtin/other-type method or not a call.
- ours-only `src/jsx/children.test.ts:39` `createElement`: R1 `correct` / R2 `false`. R1: real call or instantiation, target resolves. R2: imports.

**caddy**

- crg-only `caddyconfig/load.go:59` `AdminHandlerFunc`: R1 `our_miss` / R2 `ambiguous`. R1: call resolves to a definition in the repo. R2: type.
- ours-only `modules/caddyhttp/server_test.go:1159` `HandlerFunc`: R1 `correct` / R2 `ambiguous`. R1: real call or type conversion, target resolves. R2: type.
- gold `admin.go:266` `addRoute`: R1 `admin.go::addRoute` / R2 `unknown`. R1: closure at admin.go:245. R2: addRoute is a local closure variable.
- ours-only `caddyconfig/caddyfile/dispenser.go:535` `isNextOnNewLine`: R1 `false` / R2 `correct`. R1: call is to the package func isNextOnNewLine(t1,t2) in lexer.go; ours bound it to the method itself. R2: isNextOnNewLine: call resolves to claimed symbol.
- gold `admin.go:270` `addRouteWithMetrics`: R1 `admin.go::addRouteWithMetrics` / R2 `unknown`. R1: closure at admin.go:235. R2: addRouteWithMetrics is a local closure variable.

### Sample classifications, agreed rows only

A row counts only when both readers gave the same label. Precision of code-review-graph's extra edges = our_miss / (our_miss + their_false); precision of our extra edges = correct / (correct + false). `ambiguous` rows are left out of both ratios. In brackets: reader 1 alone, reader 2 alone (the sensitivity band).

| project | crg-only agreed n of drawn | our_miss / their_false / ambiguous | crg-edge precision (R1, R2) | ours-only agreed n of drawn | correct / false / ambiguous | our-extra precision (R1, R2) |
|---|---|---|---|---|---|---|
| graphmind | 75 of 100 | 46 / 29 / 0 | 0.61 (0.68, 0.62) | 99 of 100 | 95 / 0 / 4 | 1.00 (0.99, 1.00) |
| srclight | 8 of 8 | 6 / 1 / 1 | 0.86 (0.86, 0.86) | 91 of 92 | 88 / 1 / 2 | 0.99 (0.99, 0.98) |
| code-review-graph | 95 of 100 | 76 / 5 / 14 | 0.94 (0.94, 0.94) | 95 of 100 | 71 / 2 / 22 | 0.97 (0.96, 0.95) |
| hono | 84 of 100 | 66 / 18 / 0 | 0.79 (0.79, 0.76) | 96 of 100 | 85 / 11 / 0 | 0.89 (0.88, 0.86) |
| caddy | 98 of 100 | 59 / 26 / 13 | 0.69 (0.70, 0.69) | 86 of 100 | 85 / 1 / 0 | 0.99 (0.96, 0.99) |
| pooled | 360 of 408 | 253 / 79 / 28 | 0.76 | 467 of 492 | 424 / 15 / 28 | 0.97 |

Reading: edges only code-review-graph has are mostly real calls we lack (pooled majority our_miss); its extra edges are X
percent of the time on graphmind, hono and caddy (bare-name binds, a local `reverse` bound to `Array.reverse`'s name). Our extra edges are
correct 88 to 99 percent of the time. The sampled populations differ greatly in size (hono crg-only 5,167, caddy 3,707), so these are
precision estimates for the sampled edge sets, not engine-wide accuracy. The hono false edges have three causes: `render()` bound to the
benchmark copy `benchmarks/jsx/src/hono.ts::render` instead of `src/jsx/dom/render.ts` (8 of 12 reader-1 false rows), the global
`new Response` bound to the type alias `src/client/types.ts::Response` (3), and a `setCount` destructured from `useState` bound to a `let` in another test (1).

### Gold standard: 150 sites per project, scored against the labels

An engine answers a site when the smallest function or method node of its own graph enclosing the line has a call edge whose target name equals the callee's last segment (`unresolved::` answers ignored). Precision = answered sites whose answer set contains the true target / answered sites with a known truth (an answer on an `external` site is a false answer). Recall = repo-target sites whose answer set contains the true target / repo-target sites. `final` = rows both readers agree on (disagreements dropped; their count below). The band is reader 1 alone and reader 2 alone (disagreements resolved in favour of each); the table gives min to max over final, R1, R2.

| project | gold rows dropped (disagree) | repo-target sites (final) | ours precision | ours recall | crg precision | crg recall |
|---|---|---|---|---|---|---|
| graphmind | 2 | 48 | 0.973 (0.947 to 0.973) | 0.750 (0.720 to 0.750) | 0.963 (0.963 to 0.963) | 0.542 (0.520 to 0.542) |
| srclight | 3 | 50 | 1.000 (0.951 to 1.000) | 0.780 (0.736 to 0.780) | 1.000 (0.949 to 1.000) | 0.740 (0.698 to 0.740) |
| code-review-graph | 1 | 53 | 1.000 (1.000 to 1.000) | 0.925 (0.925 to 0.925) | 1.000 (1.000 to 1.000) | 1.000 (1.000 to 1.000) |
| hono | 9 | 36 | 0.846 (0.846 to 0.857) | 0.306 (0.273 to 0.306) | 0.870 (0.840 to 0.870) | 0.556 (0.477 to 0.556) |
| caddy | 12 | 48 | 1.000 (1.000 to 1.000) | 0.250 (0.245 to 0.250) | 0.886 (0.872 to 0.886) | 0.646 (0.607 to 0.646) |
| pooled | 27 | 235 | 0.980 (0.962 to 0.980) | 0.626 (0.586 to 0.626) | 0.954 (0.934 to 0.954) | 0.711 (0.668 to 0.711) |

Counts behind the pooled final row: ours 147 correct of 150 answered, 147 hits of 235 repo-target sites; code-review-graph 167 of 175
answered, 167 of 235. Reader 1 alone (the A6a provisional): ours precision 150/156, recall 150/256; crg 171/183, 171/256. The dropped rows
are mostly sites where reader 1 named a local closure or dynamic method that reader 2 called `unknown`; neither engine models them, so
dropping them slightly flatters both recalls (hono ours 0.273 to 0.306, graphmind 0.72 to 0.75). Per-project sampling noise is about
+-0.1 (36 to 53 repo-target sites).

### Verdict per area

| area | verdict | the number that decides it |
|---|---|---|
| symbols | on par on Python, Rust+TS and mixed repos; not comparable on hono, behind on caddy | Jaccard 0.935 to 0.988 on graphmind, srclight, code-review-graph; caddy 0.777 (types: we emit 1,010 `type` nodes, crg 0); hono 0.191 (function 0.107, class 0.07, types 1,168 vs 0, keying) |
| calls, Python (srclight, code-review-graph) | on par | gold recall 0.78 / 0.925 ours vs 0.74 / 1.0 crg, precision 1.0 both; resolved call J 0.970 and 0.965 |
| calls, Rust + TS/JSX (graphmind) | ahead | gold recall 0.75 vs 0.54, precision 0.97 vs 0.96; call J 0.741; but 29 of 75 agreed crg-only rows were their_false, 46 our_miss |
| calls, TypeScript (hono) | behind | gold recall 0.31 vs 0.56 (band 0.27 to 0.31 vs 0.48 to 0.56); precision 0.85 vs 0.87; call J 0.012 |
| calls, Go (caddy) | behind | gold recall 0.25 vs 0.65, precision 1.0 vs 0.89; call J 0.241 |
| calls, pooled | behind on recall, ahead on precision | recall 0.626 vs 0.711 (band 0.586 to 0.626 vs 0.668 to 0.711), precision 0.980 vs 0.954 |
| imports | on par on Python and TS, ahead on Go and Rust+TS by superset | J 0.904 / 0.901 / 0.941 (srclight, code-review-graph, hono); graphmind 0.647 with ours a superset (468 vs 303); caddy: crg 0 Go imports, ours 389 |
| tests | behind | J 0.12, 0.61, 0.52, 0.00, 0.11 (hono's 0.00: 2,683 ours vs 4,181 crg, 0 shared) |
| speed | ahead | cold build 1.63x to 3.42x faster (ratios only, loaded host) |
| comparison layer (matrix, relate, claims, vault) | ahead | absent in code-review-graph; not re-measured in this run |

### What the numbers cannot show

- Keying artefacts in hono symbol J (0.191): the two engines name `describe`/`it` blocks, class-like objects and type aliases under different kinds and names, so the (path, name) key overlaps little even where both found the code; hono's call J (0.012) is the same artefact compounded, and the gold recall is the trustworthy figure there.
- Go imports are absent in code-review-graph (0 edges), so import J is 0.0 for caddy by emptiness, not by disagreement.
- The host was loaded (37 to 73 on 40 cores): absolute seconds are not comparable with 2026-10-12; only the ratios are meaningful.
- Callee extraction in the gold draw is a one-line, per-language regex, first call per line: it undersamples chained and multi-line calls and over-represents simple statement-start calls, and the scoring matches on callee name from the enclosing node, so a right name from the right function counts as a hit even when several same-name targets exist.
- Two readers, one rubric author: kappa is high on gold but agreement on `ambiguous` is weak, and both readers are language-model agents, not independent humans. Samples are 36 to 53 repo-target sites per project (about +-0.1).
- Recall is over repo-target sites only; the 79 to 96 external-call sites per project test only whether an engine wrongly answers them.

### What would close each behind

- TypeScript recall (hono 0.31 vs 0.56): 18 of 44 reader-1 repo-target sites sit in `describe()` bodies or at module level, with no enclosing function node in our graph (we create one node per `it`/`test`, none for `describe`), so only 26 of 44 are reachable; add describe-body and module-level function nodes (a file-level caller). Dynamically assigned `app.get/use/on` need the constructor loop modelled.
- Go recall (caddy 0.25 vs 0.65): same-file calls on a receiver method (`s.handle()` where `s` is the enclosing method's receiver) are not bound; bind them by the receiver's declared type in the same file, then across the package.
- hono false edges (11 of 96 agreed ours-only rows false): never bind a bare name to `benchmarks/**` copies when a non-benchmark definition of the same name is imported in the file (the `render()` case), and do not bind `new Response` / `Response(...)` to a type alias (the constructor is a global).
- Tests (J 0.00 to 0.61): link a test to its callees through the new per-case nodes in TS and Go test files; hono's 0.00 is not diagnosed beyond the keying difference.
- Symbols on caddy and hono: align the kind mapping (Go `type` declarations, TS type aliases) in `compare-engines.sh` before reading these Jaccards as disagreement.

## After Plan 6 (2026-10-13)

The sections above are left as written. This section re-measures at main `feef80c` (release build), code-review-graph 2.3.9 through
the Plan 6 C2 adapter (`providers/crg`, conformance gate per language), the same committed gold labels (`fixtures/gold/*/calls.jsonl`,
150 sites per project, two readers, labels unchanged), scored with `scripts/gold-score.sh` in fresh workspaces. Host load during the
gold scoring was 5 to 9 on 40 cores (scoring is not timed); build timings below ran at load 6 to 15 and are read as ratios between
binaries measured back to back. The codegraph-mcp head-to-head is NOT re-run here: it is the maintainer's timing and still pending.

### Gold standard, after Plan 6 (precision / recall; repo-target sites in brackets)

| project | ours Plan 5 | ours Plan 6 | crg native (Plan 5) | crg through the adapter (Plan 6) |
|---|---|---|---|---|
| graphmind [48] | 0.973 / 0.750 | 0.973 / 0.750 | 0.963 / 0.542 | 0.963 / 0.542 |
| srclight [50] | 1.000 / 0.780 | 1.000 / 0.780 | 1.000 / 0.740 | 1.000 / 0.740 |
| code-review-graph [53] | 1.000 / 0.925 | 1.000 / 0.925 | 1.000 / 1.000 | 1.000 / 1.000 |
| hono [36] | 0.846 / 0.306 | 1.000 / 0.667 | 0.870 / 0.556 | 0.880 / 0.611 |
| caddy [48] | 1.000 / 0.250 | 1.000 / 1.000 | 0.886 / 0.646 | 0.897 / 0.729 |
| pooled [235] | 0.980 / 0.626 | 0.995 / 0.834 | 0.954 / 0.711 | 0.956 / 0.736 |

Counts behind the pooled row: ours 196 correct of 197 answered, 196 hits of 235 repo-target sites (Plan 5: 147 of 150, 147 of 235);
crg through the adapter 173 of 181 answered, 173 of 235. The adapter reproduces native crg on graphmind, srclight and
code-review-graph exactly and differs slightly on hono (+0.055 recall) and caddy (+0.083 recall) because it binds a bare name crg left
unresolved at 0.5. Inter-reader kappa is unchanged (the labels did not change; see the Plan 5 table). The floors in
`fixtures/gold/FLOORS.json` (pooled 0.984 / 0.819, hono 0.99 / 0.656, caddy 0.99 / 0.99) all hold.

Caveat on caddy 1.000: the gold has 48 repo-target sites drawn before the Go work; read it as "every labelled site", not a rate. A
separate 40-edge read of the build found 40 correct (wrong-edge rate under about 7 percent at 95 percent confidence).

### Agreement between the engines (srclight, `build --providers reference,crg`)

Jaccard over qualified names (nodes) or (source, target) pairs (edges); `<` marks below 0.8. Ours first.

| scope | kind | ours | crg | both | Jaccard |
|---|---|---|---|---|---|
| nodes | class | 91 | 96 | 84 | 0.816 |
| nodes | file | 35 | 33 | 33 | 0.943 |
| nodes | function | 1,234 | 1,144 | 1,144 | 0.927 |
| nodes | method | 366 | 450 | 362 | 0.797 < |
| nodes | test | 50 | 50 | 50 | 1.000 |
| nodes | type | 13 | 0 | 0 | 0.000 < |
| edges | calls | 3,271 | 3,195 | 3,170 | 0.962 |
| edges | contains | 1,704 | 1,690 | 1,681 | 0.981 |
| edges | extends | 6 | 8 | 6 | 0.750 < |
| edges | implements | 0 | 1 | 0 | 0.000 < |
| edges | imports | 179 | 198 | 179 | 0.904 |
| edges | tests | 1,785 | 2,003 | 1,412 | 0.594 < |

The adapter dropped 10,162 crg edges whose endpoint is not an indexed node (calls 7,208, tests 2,507, imports 438, extends 9).
Conformance of the adapter: go, python, typescript pass; java (edges recall 0.77) and rust (nodes 0.86, edges recall 0.53) stay below the floors.

### Build time and scale (release build, `COMPAREGRAPH_PROFILE=1`)

Hermes-agent (10,288 files) and openclaw (38,258 files) on scratch copies; the incremental edit is one function appended to one source
file of a copy (`hermes_cli/gateway_migrate_guards.py`, `src/entry.ts`). Host load average 6 to 15.

| tree | step | wall | cache hits / misses | edges copied | store_hash |
|---|---|---|---|---|---|
| hermes-agent | full | 93.2 s | 0 / 10,288 | 0 | 2f3fb527... |
| hermes-agent | no-op rebuild | 2.0 s (reused) | | 0 | same as full |
| hermes-agent | one-file edit, incremental | 47.5 s | 10,287 / 1 | 215,935 | d51b68f0... |
| hermes-agent | same tree `--full` | 85.2 s | | 0 | equal to the incremental |
| openclaw | full | 499.6 s | 0 / 38,258 | 0 | 2acfc1f0... |
| openclaw | no-op rebuild | 7.5 s (reused) | | 0 | same as full |
| openclaw | one-file edit, incremental | 289.7 s | 38,257 / 1 | 670,907 | 43dbae2f... |
| openclaw | same tree `--full` | 499.2 s | | 0 | equal to the incremental |

Per pass, openclaw full: read+parse 299.6 s, resolve_edges 145.0 s, nodes 8.7 s, inherit 13.7 s, store inserts 17.8 s, store_hash 7.9 s;
max RSS 3.54 GB. Incremental: read+parse 171.0 s (all cache hits, the per-file analyses still run), resolve_edges 41.4 s, load_prev 3.3 s.

Back to back on hermes-agent, full build, three binaries, same load window (6 to 9): pre-Plan 6 binary 194.4 s, Plan 6 B1 final 64.5 s,
`feef80c` 90.5 s. So `feef80c` is 2.15x faster than before Plan 6 and 1.40x slower than B1 alone: resolve_edges went 10.0 s to 30.3 s
and read+parse 38.8 s to 44.6 s after the A lanes (go_ctx, barrel re-exports, class-receiver shadowing, describe nodes) merged. This
was measured, not diagnosed.

### Query bounds on the openclaw graph (454k nodes, 1.29M edges, defaults unless stated)

| query | wall | max RSS | response | note |
|---|---|---|---|---|
| `impact` on `normalizeOptionalString` (3,156 callers) | 2.5 s | 14 MB | 328 KB | 500 nodes (cap), `truncated`, 458 tests |
| `traverse` same symbol, `--direction both --depth 5` | 3.6 s | 12 MB | 110 KB | 200 nodes (cap), `truncated` |
| `dead-code --limit 50` | 1.6 s | 15 MB | 12 KB | 50 items |

### What Plan 6 changed in the verdict table

| area | after Plan 5 | after Plan 6 |
|---|---|---|
| calls, TypeScript (hono) | behind: recall 0.31 vs 0.56 | ahead: 0.667 / P 1.000 vs crg 0.556 / 0.870 (adapter 0.611 / 0.880) |
| calls, Go (caddy) | behind: 0.25 vs 0.65 | ahead on the labelled sites: 1.000 / 1.000 vs 0.646 / 0.886 (48 sites) |
| calls, pooled | behind on recall | ahead on both: 0.834 / 0.995 vs 0.711 / 0.954 |
| speed (cold, hermes-agent, same binary family) | ahead | 2.15x faster than before Plan 6; openclaw 38k files 499.6 s cold (was 1,866 s with every parse a cache hit), target 8 min missed by 20 s |
| tests edges | behind (J 0.00 to 0.61) | unchanged; srclight tests J 0.594 |
