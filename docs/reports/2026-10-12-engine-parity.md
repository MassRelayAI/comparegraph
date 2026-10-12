---
title: Engine parity with code-review-graph, measured on three projects
date: 2026-10-12
valid_until: 2027-01-10
lane: plan-4
verdict: on par for symbols, containment and inheritance and 2-6x faster to build; behind on call recall, imports and test links; ours emits many false name-guess edges; no embeddings, flows or diff review
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
| multi-hop traversal | yes: `traverse_graph` (`main.py:1224`) | yes: verb `traverse`, `graph.rs:288` |
| impact (blast radius) | yes: `graph.py:2745`, `get_impact_radius` | yes: verb `impact`, `graph.rs:312` |
| related tests | yes: `query_graph tests_for`, TESTED_BY edges | partial: verb `tests-for`, `graph.rs:395`; test files only |
| entry points | yes: `flows.py:156` | yes: verb `entry-points`, `graph.rs:511` |
| dead code | yes: `refactor.py:235` | yes: verb `dead-code`, `graph.rs:531` |
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
| call precision | behind (16 of 30 false) | ahead of its own earlier state, and the surplus is now mostly right: 2 false, 2 ambiguous, 26 correct of 30; its own false edges (10 of 54 sampled) remain | none needed on our side; the two remaining false ones are re-exports (`from pkg import f` where `pkg/__init__` re-exports) |
| imports | behind (0.44, 0.38) | recall on par: 0 of 303, 19 of 198 and 83 of 841 lacking; Jaccard 0.647, 0.904, 0.901 (graphmind lowered by 165 extras we add, not sampled) | follow Python re-exports through `__init__`; sample the Rust extras |
| test links | behind (0.00 to 0.44) | still behind (0.118, 0.605, 0.512), up in all three | per-case `it`/`describe` nodes (it makes one per case; we attribute to the file); module-level and fixture-parameter calls from tests |
| build speed | ahead 2.1x to 6.0x | ahead 2.4x to 4.0x | not applicable |
| incremental build, semantic search, flows, diff review, hub and refactor analyses | behind | unchanged | each is a feature, not a fix |
| multi-repo comparison, claims, vault | ahead | unchanged | nothing |

Caveats of this section: one host, one final run (timings vary 5 to 15 percent run to run, see above); the 54 + 30 classifications are
by one reader; the re-measure was run several times while the pass was finished, and the numbers above are from the last run at
`4e00187` only; the surplus that remains in graphmind (739 edges only we have) was sampled at 10.
