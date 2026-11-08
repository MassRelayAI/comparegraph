# A field run: one standard and twelve agent harnesses scored on `harness-agents` (2026-11-07)

**Who ran it.** A downstream consumer (MassRelay), five scoring agents in parallel, CLI 0.1.1, workspace of 120+ registered
repos, one shared store.

**What was asked.** "Take one harness as the standard, compare the famous ones against it from real code, not READMEs."

**What was scored.** For each repo, the core subset was exported with `git archive` at the newest upstream commit, and
then registered and built. The taxonomy was `harness-agents` (108 features) and, for some repos, `memory` (20),
`context-control` (45) and `integration` (20).

| repo (core subset) | commit | nodes | claims (has / partial / none / unknown) |
|---|---|---|---|
| hermes-core (the standard) | a8b0bc1505 | 19,867 | harness 108; memory 5/10/5; context-control 4/14/27 |
| harness2-core | 6868817518 | 576 | 3/23/82/0 |
| openai-agents-core | 27c75c5f9c | 6,291 | 24/30/54/0 |
| adk-core | 9be86308a1 | 7,283 | 31/38/39/0 |
| langgraph-core | e5e37b5d70 | 1,876 | 7/23/78/0 |
| agent-framework-core | 924a6e50ec | 3,623 | 32/20/56/0 |
| letta-core (archive branch) | 56ba9c2555 | 1,111 | 20/18/70/0 |
| openhands-core (software-agent-sdk) | e4160d1187 | 6,535 | 35/25/48/0 |
| openclaw-core | d041ef81ab | 71,787 | 60/24/23/1 |
| goose-core | 4facf56cba | 11,535 | 32/21/53/2 |

The 3 `unknown` rows are not judgements: they are rows the engine refused to accept as `none`. See item 2 below.

**What worked.**
- `add`, `build` and `claim --batch` handled a 71k-node TypeScript tree and 1,000+ claims in one afternoon.
- The `none` gate forced every reader to open the candidates.
- `matrix` laid out a family against 113 comparators in one call.

## What cost the most, most costly first (each measured or seen in this run)

1. **`map` is lexical, not structural.** Readers had to replace it with keyword greps on every repo. Examples:
   - `loop` on openai-agents returned `_get_running_loop` and websocket caches.
   - `scheduling` returned `ensure_strict_json_schema`.
   - `compaction-branch-summary` on OpenHands returned git-branch helpers.
   - `cf-bot-free-capacity` on goose returned `_free_bytes` and CSS selectors.
   - On langgraph the same two symbols topped most features.
   - The "(pair row)" and "specific" words of feature definitions entered the term list.

   *Ask:* rank candidates by graph position (reachable from an `entry-points` symbol, community of the matched name), not
   name match alone. Strip boilerplate words from definitions before term extraction.
2. **A `none` cannot be filed when a read-set name contains a comma, a space or a newline.** CSS selectors in goose-mcp
   and openclaw memory-core caused this: `--read-hits` is comma-split. Three rows were filed `unknown` for this reason
   alone. Separately, parser nodes for `.css`/`.html` entered the read set of a Rust or TypeScript agent at all.

   *Ask:* make `hits_read` a JSON array in `--batch` (it is) and on the flag (a repeated flag). Keep non-code kinds out of
   `absent-check` read sets by default.
3. **The store has no busy timeout.** `add`, `build` and `claim` fail with `sqlite: database is locked` whenever another
   writer runs. With 3–5 agents this happened on every repo. Two agents wrote their own retry wrappers. "Wait for
   `pgrep -x comparegraph`" starved callers, because read verbs also hold the process name. One batch of about 50 `none`
   claims took over 15 minutes of waiting.

   *Ask:* set `busy_timeout` (or a `--wait <secs>`), and say in the error which verb holds the lock.
4. **`none` refusals are plain text inside JSON output.** "N of the top 10 candidates were not read" depends on the
   exact `searched` string, and the required list is not returned, so readers looped: `searched` must equal the
   absent-check `terms` without `common`.

   *Ask:* a structured refusal carrying the full required `hits_read` list. Accept `searched` as a superset.
5. **No reachability.** A `has` means "the cited line exists", not "a running agent reaches it". In the standard's
   consumer this hid a real defect for a month: their own tools were all present, but none was reachable from a turn.

   *Ask:* an optional `reach` field on a claim, computed by `traverse` from the repo's `entry-points` (reached /
   unreached / unknown, plus the path), and a matrix filter on it.
6. **No core subset and no "where the core lives".**
   - `add` and `build` have no include/exclude, so readers exported subsets by hand and deleted tests by hand.
   - OpenHands' main repo is now only the GUI, and its agent lives in `software-agent-sdk`.
   - Letta's default branch is a landing page, with the code on `archive` and its successor in `letta-code`.

   Nothing in the engine said any of this.

   *Ask:* `add --include/--exclude` (globs, recorded on the run), and a `moved_to` / `core_repo` note on a registered repo.
7. **Claims cannot carry a source commit other than the build head.** `add` on a fresh `git init` stores `head: null`.
   Readers put the upstream commit in `author`.

   *Ask:* `claim --at-commit` (validated against the run) and a `source_commit` on `add`.
8. **No anchor-to-line resolution.** One reader wrote a script to turn (file, regex) into a line for 324 claims.

   *Ask:* `claim --anchor 'path::regex'`, resolved at write time and stored as the line.
9. **Taxonomy drift against the standard.** The standard's scorer kept 122 rows; the family has 108. The 14 extras had
   no feature id, so no comparator could be claimed against them: tool search, command-approval tiers, the skill
   curator, native tool schemas, a memory-edit tool and conformance probes, among others.

   *Ask:* `family propose --from-claims <repo>` that drafts missing feature ids from a standard repo's own scored rows.
10. **Re-export shims.** ADK keeps compatibility modules (`agents/remote_a2a_agent.py`, `flows/llm_flows/compaction.py` and
    others). Readers resolved them to the real definition by hand.

    *Ask:* mark a module whose body is only re-exports and rank its target instead.
11. **A citation is checked for existence, not relevance.** A wrong but existing line passes. Readers re-read every
    cited line.

    *Ask:* an optional `--expect <symbol|regex>` on `at`, checked at write time.
12. **The feature id is truncated in output** (`agents-rk-byo-model-pi`, `agents-rk-secrets-never-in-tr`). Readers looked up
    the full id in JSON.

    *Checked 2026-11-07:* no truncation site found in the engine; the cut ids (`agents-rk-secrets-never-in-tr`) appear nowhere on disk
    and most likely came from the readers' own tables. `tests/untruncated_ids.rs` now proves every verb and export carries the whole id.
    `export vault` still caps note file names and link slugs at 80 characters and `description:` at 160.

## Not asked for
A score, a weight or a model in the engine. The consumer keeps its prioritisation in its own process, as the roadmap's
2026-10-13 ruling says.

## Added after the integration pass (same day: activepieces, pipedream, dify, servers, family `integration`)
13. **`add` on a partial export records `licence: null, lane: study`.** The exported subset has no LICENSE file, so an MIT
    project shows as study. *Ask:* `add --licence/--lane`, or read them from the source clone named on the command.
14. **`claim` accepts a bare `path:1`.** Any line of an existing file resolves; only a missing file is refused. This is the
    same root as item 11. *Ask:* refuse a line that is blank, a comment or a docstring unless `--allow-comment` is given.
15. **No way to say "present upstream, absent from the exported tree".** The evidence was in test files the export dropped.
    *Ask:* an `out_of_subset` verdict qualifier carrying the upstream path.

## Status, 2026-11-08 (all 15 asks answered)

| Ask | Outcome |
|---|---|
| 1 | Shipped in part. `map` ranks by entry-point reach (unreached callers x0.5) and shows `reach`, `entry_hops`, `community_peers`; boilerplate words are stripped. Community peers are shown but not scored: both scoring forms lowered the mapping gold (`docs/decisions.md`). Pooled p5 0.1325 and r5 0.4513 are equal to before. The real openai-agents case was not measured (no clone). |
| 2 | Shipped. `--read-hits` is a repeated flag; css/scss/html nodes stay out of the read set unless `--include-non-code`. |
| 3 | Shipped. `--wait` / `COMPAREGRAPH_WAIT`, default 30 s; the timeout error lists other processes with the store open. A snapshot conflict has its own message. |
| 4 | Shipped. `none_refused` with `error.data` (the exact `searched`, the full `hits_read`); a superset `searched` is accepted. Breaking: the code was `invalid_arguments`. |
| 5 | Shipped. `reach` on has/partial claims, `matrix --reach`. Static only; entry-point quality bounds the result; not measured on a large repo. |
| 6 | Shipped. `add`/`build --include/--exclude`, `--moved-to`. |
| 7 | Shipped. `add --source-commit`, `claim --at-commit`. |
| 8 | Shipped. `claim --anchor 'path::regex'`. |
| 9 | Shipped. `family propose --from-claims <repo> --rows <jsonl>`; drafts only, never writes `rows.md`. |
| 10 | Shipped for Python and Rust-like file shims through `imports` edges (`shim_of` on `map`). Open: JS/TS barrels (the provider writes no edge) and symbol-level targets. |
| 11 | Shipped. `--expect`. |
| 12 | No fix needed; regression test added. |
| 13 | Shipped. `add --licence/--lane/--licence-from`. |
| 14 | Shipped. Blank, comment and docstring lines refused unless `--allow-comment`. |
| 15 | Shipped. `--out-of-subset`. |

Open items: claim patterns use a small hand-written regex subset (no new dependency without a ruling); `comparegraph_claim`
over MCP does not expose the new claim flags; vault file names still cap at 80 characters.

