# CompareGraph Plan 3 — the analysis layer (families, propose, relate, map, claim, decide, matrix, exports, MCP, method)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans or subagent-driven-development. Steps use `- [ ]`.

**Goal:** Turn the graph into comparisons: taxonomy packs (family → group → feature), a corpus-seeded draft taxonomy, relatedness
per clone and family, candidate mapping per feature, claims with resolving citations, append-only adopt/reject/park decisions, a
comparison matrix (with gaps and differentiators when a root exists), and all of it in the JSON and vault exports and over MCP.
The engine never calls a model and never writes a claim or decision on its own; `author`/`by` is always given by the caller.

**Provisional design:** every choice marked (P) below is provisional and is listed in `docs/design-notes-for-review.md` (Task 7)
for a later design review. Build it as written; do not redesign.

**Spec:** §2b–2c (workflow, packs), §4 (family, feature, relatedness, claim, location, decision), §5 (evidence), §9 (verbs).

## Global Constraints
- Schema stays v1: the family, feature, relatedness, claim, location, decision tables already exist (store/schema.sql). `feature`
  may be upserted when a pack is re-added (definitions change); `claim`, `location`, `decision`, `relatedness` are append-only
  (a re-score is a new row; the latest wins; `location.stale` is the only flag flip, done by `check`).
- Every verb answers with the Plan 2 envelope; read verbs use `Store::open_existing`; write verbs (`family add`, `family propose`,
  `relate`, `claim`, `decide`) use `Store::open`.
- Claims: `has`/`partial` need ≥ 1 `--at` location that resolves NOW (file exists in the clone, `1 ≤ start ≤ end ≤ lines`); the
  stored content_hash is computed from the current lines; `none` needs `--searched "<terms>"` (stored in note as
  `searched: <terms> → <n> hits`; refused if the engine's own search for those terms finds ≥ 1 hit in that repo — then the
  verdict is not `none`); otherwise refused with `invalid_arguments`. `unknown` needs nothing. `author` is mandatory.
- Every list sorted deterministically; vault output stays byte-identical across two exports of the same store.
- MCP gains read tools + `comparegraph_claim` (write, author required). No MCP tool writes a decision (P: decisions stay a human
  CLI act).
- Clippy `-D warnings`, tests green, named-path commits, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  Never `cargo update`; never touch `src/providers/reference/vendor/` (revert it after any rustfmt run).

## Review Focus
1. A pack whose table has a duplicate feature id or an empty definition → `family add` refuses naming the row; nothing is written (transaction) → Task 1.
2. Re-adding a pack that REMOVES a feature which already has claims → the feature row stays (claims keep their key), it is marked `retired` (P: a `tier` value `retired`) and drops out of matrices → Task 1.
3. A claim citing a path outside the clone (`../x`, absolute path, symlink escaping) → refused → Task 4.
4. A claim whose cited lines change after it was written → `check` marks the location stale and the matrix shows the cell as `unknown*` (stale), not the old verdict → Task 5.
5. `decide` twice on the same repo (or same repo+feature) → both rows kept, the second supersedes the first, views show the latest, history shows both → Task 5.

---

### Task 1: Family packs (+ doctor without a workspace)
**Pack format** — a directory `<pack>/` with `rows.md`:
```markdown
---
family: agent-harness
description: one line
---
| id | group | feature | definition | counts_as_has | tier | weight |
|---|---|---|---|---|---|---|
| routing.retry | routing | Retry with backoff | Failed provider calls are retried with increasing delay | a retry loop with a delay that grows per attempt | basics | 2 |
```
`id` = `<group>.<slug>` (lowercase, `[a-z0-9-]`, one dot); `tier ∈ {basics, common, extra, candidate}` (P); `weight ∈ 1..3`.
**Files:** `application/family.rs` (parse_pack, add_family, list_families, show_family), CLI `family add <dir>`, `family list`,
`family show <name>`; store helpers `upsert_family`, `upsert_feature`, `features_of(family)`, `retire_missing(family, keep_ids)`.
`family.definition_path` = the pack dir path as given (absolute, canonicalised).
Also: `doctor` uses no store when `<root>/.comparegraph` is absent: it reports `store: null` and the providers (fix of a Plan 2 note).
- [ ] Tests first: parse a valid pack (2 groups, 3 features); refuse duplicate id, empty definition, bad tier, bad weight, a table row with the wrong cell count (each names the row number); Review Focus 1 (nothing written); Review Focus 2 (re-add without one feature → it is `retired`, claims untouched); `family list` / `show` envelopes; doctor in an empty dir → ok with store null.
- [ ] Implement; gate; commit `feat(family): taxonomy packs — add, list, show; retire removed features; doctor without a workspace`.

### Task 2: `family propose` — a draft taxonomy from the corpus, no model
CLI `family propose <name> --repos a,b,c [--out <dir>] [--min-repos 2] [--max 60]`. For each repo's latest done run collect terms:
community labels' last path segment, community keywords, and symbol-name tokens (same tokenizer and stoplist as communities).
Count, per term, the repos it appears in and its total occurrences. Keep terms in ≥ `min-repos` repos (when only one repo is given,
min-repos = 1); rank by (repos desc, occurrences desc, term asc); take `max`. Group = the most common community-label segment the
term co-occurs with, else `misc` (P). Write `<out or <root>/families/<name>>/rows.draft.md` in the pack format with tier
`candidate`, weight 1, `definition` = `draft: seen in <k> of <n> repos (<repos>)`, `counts_as_has` = `draft: define`. Never
overwrite an existing `rows.md`; overwrite an existing `rows.draft.md`. Envelope data: the candidates with their per-repo counts.
- [ ] Tests first: three tiny repos sharing `retry` and `auth` symbols and one unique term → the draft holds the shared terms, not the unique one; determinism (two runs identical bytes); refuses with < 1 built repo; never touches rows.md.
- [ ] Implement; gate; commit `feat(propose): a corpus-seeded draft taxonomy (no model), written as rows.draft.md`.

### Task 3: `relate` and `map`
**Feature terms (P):** tokens of the feature's slug, group and `feature` title (tokenizer + stoplist), deduped; the definition is NOT used (too noisy).
**map** `map <repo> <family> [--feature id] [--limit 5]` → per active feature: up to `limit` symbol candidates (FTS with the feature terms OR-joined, ranked by bm25) and up to 3 communities whose label segments or keywords contain a feature term (ranked by number of matching terms, then label). Writes nothing; every item labelled `candidate`; confidence `observed`.
**relate** `relate [--repo r] [--family f]` → for each (repo, family) pair (all when omitted): touched = features with ≥ 1 map candidate; score = touched / active features; band = `related` ≥ 0.5, `adjacent` ≥ 0.2, else `unrelated` (P); basis = touched feature ids (sorted). Writes one `relatedness` row per pair keyed to the repo's latest done run (append; re-run → new rows only if the run changed, else returns the existing row).
Store: add an OR-search helper next to the existing AND search (do not change `search`'s semantics).
- [ ] Tests first: a repo with `retry_with_backoff` maps feature `routing.retry` to it; a repo with nothing relevant → band `unrelated`; relate twice on the same run writes one row; map on an unbuilt repo → not_found.
- [ ] Implement; CLI + envelopes; gate; commit `feat(relate,map): feature candidates from the graph and a relatedness band per clone and family`.

### Task 4: `claim` and `claims`
CLI `claim <repo> <feature-id> <has|partial|none|unknown> --author <who> [--at <path>:<start>-<end>]... [--searched "<terms>"] [--note "..."]`
(`<path>:<line>` alone means start = end). Resolution: path is relative to the clone root, normalised, must stay inside it after
canonicalisation (Review Focus 3), file must exist, lines in range; content_hash from the current lines; `node_id` = the smallest
node of the latest done run whose span contains the cited span, else NULL. Confidence `derived` when every location resolved (all
must, or the claim is refused). `run_id` = the repo's latest done run (refuse if none). `supersedes_id` = the latest claim for the same
(repo, feature). Insert claim + locations in one transaction.
`claims [--repo r] [--family f] [--feature id] [--history]` → latest per (repo, feature) by default; `--history` lists every row with its supersedes chain. Each item: verdict, confidence (shown `unknown` if any location is stale), author, created_at, locations, note.
- [ ] Tests first: has with a resolving location → derived, node_id set; has without --at → invalid_arguments; Review Focus 3 (`../x`, `/etc/passwd`, a symlink pointing outside) → refused; none without --searched → refused; none with --searched whose terms DO hit → refused; none with terms that miss → stored with the note; a second claim supersedes the first and `claims` shows the second, `--history` both.
- [ ] Implement; gate; commit `feat(claim): claims with resolving citations, append-only with a supersede trail`.

### Task 5: `decide`, the matrix, check integration
CLI `decide <repo> <adopt|reject|park> --why "<text>" --by <who> [--feature <id>]` → decision row (scope `repo` or `feature`), supersedes the latest of the same scope; empty why → invalid_arguments. `decisions [--repo r] [--history]`.
CLI `matrix <family>` → rows = active features (group, id, tier, weight), columns = repos (root first if any, then by name), cell = latest claim verdict with a `*` suffix when stale (`unknown*`), `-` when no claim; per feature: `prevalence` = share of comparator repos with has|partial; when a root exists: `gap` = some comparator has|partial and the root is none|unknown|-, `differentiator` = root has|partial and no comparator has|partial. Each repo column also carries its relatedness band and its latest decision (scope repo).
`check`: unchanged location logic; additionally reports the count of claims whose confidence is downgraded by a stale location (informational, not a problem).
- [ ] Tests first: Review Focus 4 (edit the cited lines → check marks stale → matrix cell `has*`→ displayed as `unknown*`); Review Focus 5; gap and differentiator with a root; prevalence excludes the root.
- [ ] Implement; gate; commit `feat(decide,matrix): append-only decisions; the comparison matrix with gaps and differentiators`.

### Task 6: Exports and MCP
`export json` gains `families` (features), `relatedness` (latest per pair), `claims` (latest + `history` count), `decisions` (latest + history), and per family the matrix. Vault gains: `families/<family>.md` (front matter {type: family, features, repos}; the matrix as a markdown table with links), `features/<family>/<feature-slug>.md` (definition, counts_as_has, per-repo verdict with location links `[[symbols/...]]` when node_id is set, else `path:start-end` text), and the repo note gains its relatedness bands, claims count and latest decision. Index links families. Still byte-identical across two exports.
MCP tools added: `comparegraph_families`, `comparegraph_map` {repo, family, feature?, limit?}, `comparegraph_relate` {repo?, family?} (read: returns latest rows; does NOT compute — P), `comparegraph_claims` {repo?, family?, feature?, history?}, `comparegraph_matrix` {family}, `comparegraph_claim` {repo, feature, verdict, author, at?: [string], searched?: string, note?: string} (write). No decide tool.
- [ ] Tests first: json export contains the new keys on a workspace with a pack + one claim + one decision; vault has the family and feature notes and every wikilink resolves; determinism; MCP tools/list has 12 tools; comparegraph_claim without author → invalid_arguments; comparegraph_matrix returns the matrix.
- [ ] Implement; gate; commit `feat(export,mcp): families, claims, decisions and matrices in the exports and over MCP`.

### Task 7: The method, the notes for review, the proof
- `method/` (shipped as plain markdown, superpowers-style, each `method/<skill>/SKILL.md` with front matter `name`, `description`): `adding-a-clone`, `defining-a-family`, `mapping-features`, `writing-a-claim`, `recording-a-decision`, `reading-the-vault`. Each: when to use, the exact commands, what the engine refuses, how to verify (command + expected envelope fields). Keep each under 60 lines.
- `docs/design-notes-for-review.md`: every (P) choice in this plan, one row each — the choice, the alternative, why provisional, what would change it; plus the open items carried from Plans 1–2 (srclight flat-package communities need a graph split; Perl grammar lockfile; unwrap_or fallbacks; head refresh per run; history squash before publishing).
- `families/example-code-graph-tools/rows.md`: a small real pack (8–12 features: symbol extraction, call graph, import graph, communities, full-text search, embeddings, MCP server, multi-repo registry, incremental update, export formats, …) — the study clones ARE code-graph tools.
- Proof (paths never committed): add graphmind, srclight, code-review-graph as comparators; add the pack; `family propose` over the three; `relate`; `map` one repo; write 3 claims with real citations by `--author proof-run`; `matrix`; vault export. Append a "Plan 3" section to `docs/reports/2026-10-12-plan-1-proof.md` (relatedness bands, matrix as a table, draft-term count, timings).
- README: the analysis verbs and the method folder.
- [ ] Gate; commit `docs: method skills, design notes for review, example pack, plan-3 proof`.

## Self-review
Spec §9 verbs: family add/propose, relate, map, claim, decide all covered; matrix and claims/decisions lists added as views. Tracing: supersedes on claim and decision (Tasks 4–5). Engine never writes a claim/decision on its own (author/by mandatory; relate and propose write only derived rows). Review Focus mapped to Tasks 1, 1, 4, 5, 5.
