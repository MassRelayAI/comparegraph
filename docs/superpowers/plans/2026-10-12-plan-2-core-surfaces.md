# CompareGraph Plan 2 — core surfaces (envelope, communities, search, show, vault export, MCP)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans or subagent-driven-development. Steps use `- [ ]`.

**Goal:** Make the Plan 1 engine usable by any tool: one response envelope for every verb, communities (module clusters) per run,
full-text search over symbols, `show` (a symbol with its code and licence lane), an Obsidian-style vault export, and an MCP stdio
server exposing the read verbs. Deterministic; no LLM anywhere.

**Out of scope (the analysis layer, designed later in a separate session):** families, `family propose`, `relate`, `map`, `claim`,
`decide`. Their tables already exist in schema v1; this plan writes none of them.

**Architecture:** unchanged rings. New: `application/{communities,search,show,vault}.rs`, `adapters/{envelope,mcp}.rs`. The store
gains community and FTS helpers. Schema stays v1 (unreleased): add one FTS5 virtual table to `schema.sql`.

**Tech Stack:** as Plan 1; SQLite FTS5 (compiled into rusqlite's bundled SQLite — `-DSQLITE_ENABLE_FTS5` in libsqlite3-sys). No new crates.

**Spec:** `docs/superpowers/specs/2026-10-12-comparegraph-design.md` §4 (community), §5 (evidence), §9 and §9b (verbs, envelope), §10 (MCP), §15 (standards).

## Global Constraints
- Every CLI and MCP answer is the §9b envelope (Task 1). Exit 0 on `ok:true`, 1 on `ok:false`.
- Compact by default: no code bodies unless `--detail full` (show only). Lists sorted deterministically; `--limit` (default 50, max 500) and `--cursor` (an opaque string = the decimal offset) on list verbs.
- Snippets are read from the clone at request time, never stored. Every snippet carries the repo's `licence` and `lane`.
- Communities are part of a run: written at build, never updated; `store_hash` is unchanged by this plan (communities are derived from nodes/edges already hashed).
- Vault export is regenerated whole into an output dir; deterministic file names and contents (two exports of the same store are byte-identical).
- MCP: JSON-RPC 2.0, one JSON object per line on stdin/stdout, protocol version string `"2025-06-18"`; methods `initialize`, `notifications/initialized` (no reply), `tools/list`, `tools/call`, `ping`; unknown method → error -32601. Tool results are `{"content":[{"type":"text","text":"<envelope JSON>"}],"isError":<!ok>}`.
- Clippy `-D warnings`, tests green, named-path commits, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never `cargo update`. Never touch `src/providers/reference/vendor/`.

## Review Focus
1. A clone with no import edges at all (a single file, or a language whose imports are not resolved): every file must still belong to exactly one community (a singleton) — test in Task 2.
2. Search with FTS syntax characters in the query (`"`, `*`, `-`, `:`, parentheses): must not error; the query is tokenised and quoted → test in Task 3.
3. `show` on a symbol whose file changed since the build: return the current lines with `stale: true` and confidence `unknown`, not the old content hash as if valid → test in Task 4.
4. Vault export into a dir that already holds an older export: stale notes from the old export must not survive (export writes to a temp dir and swaps) → test in Task 5.
5. MCP server receiving a malformed line or a request without `id`: reply `-32700`/`-32600` for malformed requests and keep serving; never exit on a bad line → test in Task 6.

---

### Task 1: The envelope

**Files:** Create `crates/comparegraph/src/adapters/envelope.rs`; modify `adapters/mod.rs`, `adapters/cli.rs`, `application/mod.rs`; tests in `tests/cli.rs`.

**Interfaces (produces):**
```rust
#[derive(serde::Serialize)]
pub struct Envelope { pub ok: bool, pub verb: String, pub schema: u32 /* =1 */, pub data: serde_json::Value,
  pub evidence: Evidence, #[serde(skip_serializing_if = "Option::is_none")] pub page: Option<Page>,
  pub cost: Cost, #[serde(skip_serializing_if = "Option::is_none")] pub error: Option<ErrorBody> }
#[derive(serde::Serialize)] pub struct Evidence { pub confidence: &'static str /* observed|derived|unknown */, pub limitations: Vec<crate::domain::Limitation> }
#[derive(serde::Serialize)] pub struct Page { pub next: Option<String>, pub total: usize }
#[derive(serde::Serialize)] pub struct Cost { pub rows: usize, pub bytes: usize }
#[derive(serde::Serialize)] pub struct ErrorBody { pub code: &'static str, pub message: String, #[serde(skip_serializing_if = "Option::is_none")] pub hint: Option<String> }
impl Envelope { pub fn ok(verb: &str, data: Value, confidence: &'static str, limitations: Vec<Limitation>, page: Option<Page>) -> Self;
                pub fn err(verb: &str, e: &AppError) -> Self; pub fn to_json(&self) -> String /* fills cost.bytes = len of data JSON, cost.rows = array len of data or data["items"], else 1 */ }
```
`AppError::code(&self) -> &'static str` (add in `application/mod.rs`): Store→`store_error`, Provider→`provider_unavailable`, NotFound→`not_found`, Empty→`empty`, Io→`io_error`. Keep stable; they are the catalog's error codes.

- [ ] Tests first (tests/cli.rs, update the existing three to read `v["data"]…` instead of top-level fields, plus): `envelope_shape_on_success_and_error` — `comparegraph --root <ws> doctor` stdout parses as JSON with `ok:true`, `verb:"doctor"`, `schema:1`, `evidence.confidence`, `cost.bytes > 0`; `comparegraph build nope` exits 1 and stdout has `ok:false`, `error.code:"not_found"` (errors now go to STDOUT as an envelope; also print the one-line message to stderr).
- [ ] Implement; every `Cmd` branch builds an Envelope. Confidence per verb: add/build/doctor/conform/export → `observed`; check → `derived`.
- [ ] Gate, commit `feat(envelope): one response envelope for every verb with stable error codes`.

### Task 2: Communities per run

**Files:** Create `application/communities.rs`; modify `store/mod.rs` (insert/select communities), `application/build.rs` (call after edges, before finish_run(Done)); tests in the module and `tests/application.rs`.

**Algorithm (deterministic):**
1. Undirected graph over FILE/TEST nodes; an edge between two files when an `imports` edge links them, or a `calls` edge links symbols contained in them (cross-file).
2. Connected components (union-find), iterate files sorted by path.
3. Label = the longest common directory prefix of the component's file paths (trimmed of trailing `/`); if empty or `.`, the most frequent first path segment; singletons are labelled by their file path. Ensure labels unique per run by appending ` (2)`, ` (3)` in path order.
4. Keywords = top 5 lowercase tokens (split names on `_`, `-`, camelCase) across member symbol names, ties broken alphabetically, excluding tokens of length < 3 and a stoplist {get,set,new,the,and,for,with,from,init,test}.
5. Cohesion = internal file-edges / (internal + external file-edges) over the call+import file graph; a component with no edges has cohesion `null`.
6. Members = the files and every symbol they contain (via `contains`).
**Store API:** `insert_communities(repo, run, &[CommunityRow{label, keywords: Vec<String>, cohesion: Option<f64>, members: Vec<NodeId>}]) -> Result<Vec<i64>>`, `communities_of_run(run) -> Result<Vec<CommunityRow /* + id */>>` ordered by label.
- [ ] Tests first: (a) two files importing each other + one isolated file → 2 communities, the isolated one a singleton labelled by its path, cohesion null for the singleton; (b) **Review Focus 1**: a repo of 3 files with no imports and no cross-file calls → 3 singleton communities covering all files; (c) labels: files `src/auth/a.ts`, `src/auth/b.ts` linked → label `src/auth`; (d) build twice → identical community labels and member counts.
- [ ] Implement; BuildReport gains `communities: usize`.
- [ ] Gate (also run the ignored real-clone test once on one study clone and note the community count in the commit body), commit `feat(communities): import/call connected components per run, labelled, with keywords and cohesion`.

### Task 3: Search

**Files:** modify `store/schema.sql` (add `CREATE VIRTUAL TABLE IF NOT EXISTS node_fts USING fts5(name, qualified_name, path, tokenize='unicode61 remove_diacritics 2 tokenchars ''_''')` with rowid = node.id), `store/mod.rs` (`insert_nodes` also inserts into node_fts in the same transaction); create `application/search.rs`; CLI `search <query> [--repo r] [--kind k] [--limit n] [--cursor c]`.

**Interface:** `search(root, q: &SearchQuery{ text, repo: Option<String>, kind: Option<NodeKind>, limit, offset }) -> Result<SearchPage{ items: Vec<Hit{ repo, qualified_name, kind, path, line_start, line_end, community: Option<String>, score: f64, label: "candidate" }>, total, next: Option<String> }>`.
Only nodes of each repo's LATEST DONE run are searched (join run). Query building: split `text` on non-alphanumeric/underscore, drop empties, each token double-quoted with a `*` suffix for prefix match, joined by spaces (AND). Empty token list → `AppError::Empty("empty query")`. Score = `-bm25(node_fts)`; order by score desc, then qualified_name asc.
- [ ] Tests first: finds a function by a name prefix; filters by repo and kind; pagination (`limit 1` → next = "1"; following page returns the second hit; total constant); **Review Focus 2**: queries `foo"bar`, `a*`, `-x`, `x:y`, `(a)` do not error; old runs' nodes are not returned after a rebuild.
- [ ] Implement, CLI wiring (envelope `page`), commit `feat(search): FTS5 over symbols of each repo's latest run, quoted tokens, paginated`.

### Task 4: Show

**Files:** create `application/show.rs`; CLI `show <repo> <qualified_name> [--detail full]`.
**Interface:** `show(root, repo, qn, full: bool) -> Result<Shown{ repo, licence: Option<String>, lane, node: Node, community: Option<String>, callers: Vec<String>, callees: Vec<String>, stale: bool, confidence: &'static str, lines: Option<Vec<String>> }>`.
callers/callees = qualified names via `calls` edges of the run (sorted, max 50 each). `stale` = the file's current lines `[line_start..=line_end]` hash ≠ node.content_hash (or file missing / out of range). confidence = `observed` when not stale, `unknown` when stale. `lines` only when `full`; when stale, the CURRENT lines are returned (if readable).
- [ ] Tests first: compact show has no `lines`; full show returns the exact lines; licence and lane present; **Review Focus 3**: edit the file after build → `stale:true`, `confidence:"unknown"`, current lines returned; unknown qn → `not_found`.
- [ ] Implement, commit `feat(show): a symbol with its callers, callees, community, licence lane and on-request lines; stale when the code moved`.

### Task 5: Vault export

**Files:** create `application/vault.rs`; CLI `export vault --out <dir>`.
**Layout (all from each repo's latest done run):**
```
<out>/index.md                      front matter {type: index, repos: n, generated_by: comparegraph <version>, schema: 1}; a table of repos with links
<out>/llms.txt                      llms.txt convention: title, one-line summary, links to index.md and every repo note
<out>/AGENTS.md                     how an agent should read this vault (which note to open first, that snippets need `comparegraph show`)
<out>/repos/<repo>.md               front matter {type: repo, name, licence, lane, role, head, files, nodes, edges, communities}; communities as [[wikilinks]] with size and cohesion
<out>/communities/<repo>/<slug>.md  front matter {type: community, repo, label, size, cohesion, keywords}; [[../../repos/<repo>]] parent link; member files, then symbols as `- [[<symbol-note>]] kind path:line`
<out>/symbols/<repo>/<slug>.md      only for class|function|method|type|route nodes: front matter {type: symbol, repo, kind, path, line_start, line_end, community, content_hash}; callers and callees as wikilinks
```
Slug = lowercase, non `[a-z0-9]` → `-`, collapsed, trimmed, max 80 chars; collisions get `-2`, `-3` in qualified-name order. No timestamps in contents (determinism). Write into `<out>.tmp-<pid>`, then remove `<out>` (only if it contains `index.md` with `generated_by: comparegraph` — otherwise refuse with `AppError::Io`) and rename. Refuse when 0 nodes.
- [ ] Tests first: the layout exists with expected counts on a small repo; two exports are byte-identical (hash every file); **Review Focus 4**: a note present in an old export but not in the new one is gone; refuses to overwrite a dir that is not a comparegraph vault; front matter parses as YAML (check `---` fences and `key: value` lines).
- [ ] Implement, commit `feat(vault): deterministic Obsidian-style vault — index, llms.txt, AGENTS.md, repos, communities, symbols`.

### Task 6: MCP stdio server

**Files:** create `adapters/mcp.rs`; CLI `mcp` (serves until stdin EOF); tests `tests/mcp.rs` spawning the binary.
**Tools (name → input schema → verb):** `comparegraph_repos` {} → list repos with latest run summary; `comparegraph_search` {query: string, repo?: string, kind?: string, limit?: integer, cursor?: string}; `comparegraph_show` {repo, qualified_name, detail?: "compact"|"full"}; `comparegraph_communities` {repo, limit?, cursor?}; `comparegraph_check` {}; `comparegraph_doctor` {}. Each `inputSchema` is JSON Schema 2020-12 (`type: object`, `properties`, `required`, `additionalProperties: false`). Descriptions say what the tool returns and that results are compact by default.
Every tools/call result: `{"content":[{"type":"text","text": <envelope JSON string>}], "isError": !envelope.ok}`. Invalid arguments → `isError:true` with envelope error code `invalid_arguments`. `--root` comes from the CLI flag the server was started with.
- [ ] Tests first: initialize → result has `protocolVersion`, `serverInfo.name == "comparegraph"`, `capabilities.tools`; tools/list returns the 6 tools with schemas; tools/call comparegraph_search on a built workspace returns an envelope with items; **Review Focus 5**: a garbage line → error -32700 and the next valid request still answers; a request with an unknown method → -32601; EOF exits 0.
- [ ] Implement (synchronous read_line loop; flush after each response), commit `feat(mcp): stdio JSON-RPC server exposing repos, search, show, communities, check, doctor`.

### Task 7: Docs and proof

- [ ] README: add search, show, export vault, mcp; an MCP client config snippet (`{"mcpServers":{"comparegraph":{"command":"comparegraph","args":["--root","/path/to/workspace","mcp"]}}}`).
- [ ] Proof: on the three study clones (paths passed by env, never committed), record community count, search latency for 3 queries, vault file count and export seconds; append to `docs/reports/2026-10-12-plan-1-proof.md` as a "Plan 2" section (no local paths).
- [ ] Gate, commit `docs: README for the core surfaces; plan-2 proof numbers`.

## Self-review
Spec §9 core verbs covered (search, show, export vault, mcp, doctor); analysis verbs explicitly out. §9b envelope Task 1. §15: JSON Schema (Task 6), JSON-RPC/MCP (Task 6), Obsidian + llms.txt + AGENTS.md (Task 5). Review Focus each mapped to a test. Types: `Hit`, `Shown`, `CommunityRow` defined where produced; `Envelope` consumed by Tasks 3–6.
