# CLI reference

Every command, with the flags that matter.

Global options on every command, before or after the verb: `--root <ROOT>` (the workspace, default `.`), `--pretty`, `--allow-unverified` (use an
external provider with no manifest). `comparegraph <verb> --help` prints every flag.

## Build and inspect

| Command | What it does | Flags that matter |
|---|---|---|
| `add <PATH>` | Register a clone. Read, never run. With `--replace` (and `--name <existing>`) it re-points that registered repo at `<PATH>` instead: claims and runs stay, the root flag is kept unless `--as-root` is passed too. It refuses an unknown name ("nothing to replace; drop --replace") and a path the repo already has. The envelope reports `old_path`, `new_path` and the new path's `head`, with a limitation saying the last build read the old path and to run `build` again. | `--name`, `--as-root`, `--replace` |
| `build <REPO>` | Parse a clone into the graph (one run). The envelope's `data` names the clone `path` it read and the `head` (short commit) it found there; `--progress` prints the same as its first stderr line. | `--provider` (default `reference`), `--providers a,b` (several engines in turn, with an agreement table), `--max-file-bytes`, `--max-files`, `--full`, `--progress` (stderr lines every 500 files and per pass; also `COMPAREGRAPH_PROGRESS=1`; stdout stays one envelope) |
| `check` | Verify schema, determinism and citation locations. Also counts `unbuilt_claim_runs`: claims whose cited run has no row or is not a finished build; each affected repo adds a problem line (`<repo>: <n> claims cite run <id> which was never built`) and the verb exits non-zero, like any other problem. `stale_locations` counts only locations of live claims (the latest claim per repo and feature, the one `matrix` and `export` treat as current) and each is a problem; `stale_locations_superseded` counts the stale locations of superseded claims (append-only history that cannot change) and is reported in `notes`, never as a problem. | |
| `repos` | Registered repos with their latest run. | `--limit` (50), `--cursor` |
| `doctor` | Report the store, every provider and each agent client's registration. | `--bin`, `--brief` (plain text for a session-start hook: nothing without a store, else one or two lines; the one output that is not an envelope) |
| `conform <PROVIDER>` | Run the golden fixtures against a provider. | `--fixtures` |
| `catalog` | Print the MCP tool catalog as JSON. | `--check` (compare with `docs/catalog.json`) |

## Graph queries

| Command | What it does | Flags that matter |
|---|---|---|
| `search <QUERY>` | Full-text search over symbols of each latest run. | `--repo`, `--kind`, `--limit`, `--cursor` |
| `show <REPO> <QN>` | A symbol with callers, callees, community, licence lane. | `--detail full` |
| `communities <REPO>` | Module clusters of the latest run. | `--limit`, `--cursor` |
| `traverse <REPO> <QN>` | Nodes reachable over calls, extends, implements, with hop distance and path. | `--direction`, `--depth` (3), `--max-nodes` (200), `--kinds`, `--min-confidence` (0.5), `--limit` (50), `--cursor` |
| `impact <REPO> [QN]` | Everything that transitively depends on a symbol or files, plus the tests that reach it. Summary first (`total`, `by_hop`, `by_kind`, `tests_total`), then `items` paged, best `path_confidence` first; the `tests` list is capped at `--limit` too, with a `tests_capped` limitation when cut (`tests-for` has the full list). | `--file`, `--depth` (5), `--max-nodes` (500), `--limit` (50), `--cursor` |
| `tests-for <REPO> <QN>` | Tests linked directly or through at most 3 call hops. | `--max-nodes`, `--min-confidence`, `--limit` (50), `--cursor` |
| `entry-points <REPO>` | Exported, handler or main symbols nothing calls. | `--limit`, `--cursor` |
| `dead-code <REPO>` | Candidates nothing references. Dynamic dispatch is invisible. | `--limit`, `--cursor` |

## Families and claims

| Command | What it does | Flags that matter |
|---|---|---|
| `family add <DIR>` | Load a pack (`rows.md`). Features it no longer lists become retired. | |
| `family list` / `family show` | Registered families; one family by group. | |
| `family propose <NAME>` | Draft `rows.draft.md` from terms the given repos share. No model. | `--repos a,b,c`, `--min-repos`, `--max`, `--out` |
| `relate` | Score each clone against each family and record the band. | `--repo`, `--family`, `--stored` |
| `map <REPO> <FAMILY>` | Candidate symbols and communities per feature. Writes nothing. | `--feature`, `--limit` (5) |
| `absent-check <REPO> <FAMILY> <FEATURE> [--searched <terms>]` | Is the feature absent? Also returns `data.read_set`: `{cap: 10, total, candidates[]}`, the union of the symbol, path and kind channels (and, with `--searched`, the hits of that search) ranked by score (exact name 8, name token 4, path or qualified-name substring 2, kind-only or search-only 1, +1 for a community label carrying a term; `why` lists the reasons), best first, capped at 10; `--searched` terms must name the feature, as for `claim`, and a search hit makes `absent` false. Counts and up to 5 hits over four channels: symbols, paths, community labels and keywords, node kinds the feature names. `data.absent` is true only when all four are zero. A family word (a term in the term set of at least half of the family's features and of at least 4: `agents` where every row is `agents.*`) names the family, not the feature: it is dropped from all four channels and listed in `data.common` and in the note as `common=<terms>`; a hit on a remaining term refuses as before, and zero hits with a family word set aside also refuses, naming the uncounted words (record that absence with `--searched`); when every term is a family word the check refuses and records nothing (pass `--searched` with specific terms). The symbol channel counts every node whose name carries a term, whatever its kind. Reads only. `data.reason` says why `absent` has its value: `hits` (a channel found something), `family words set aside: <words>` (every channel is zero but terms were set aside, so absence is not concluded) or `absent`. | |
| `claim <REPO> <FEATURE> <VERDICT>` | Record `has`, `partial`, `none` or `unknown`. `none` without `--searched` runs the absent check: all channels empty records it as `inferred`; any hit refuses it and lists the candidates. `none --searched <terms>` must name the feature (a word of its id, title, synonyms, definition or counts-as-has text, other than a stop word or a family word; the refusal lists at most 15 accepted words) and records `derived` when the search finds nothing. When the feature has candidates (its channel hits plus the search hits, ranked as `absent-check --searched` returns them), open the top 10 and pass `--read-hits` with their qualified names (comma or space separated; batch rows use `hits_read: [..]`, MCP `read_hits`): the claim is then recorded `observed` with the note `searched: <terms> → N candidates, top 10 read and rejected: <names>` (20 names, then `+k more`); when N exceeds 10 the envelope carries a `read_capped` limitation and the note ends `; N-10 lower-ranked candidates not demanded`. Refusals: `not none: K of the top 10 candidates were not read: <all unread>; run absent-check --searched "<terms>" for the list, or claim has/partial`; `--read-hits names symbols the search does not find: <names>` (a name outside the whole union, at any rank); `--read-hits` without `--searched` or on a verdict other than `none`. With no candidate at all the claim records as before and the note says nothing was read. | `--author` (required), `--at path:start-end`, `--searched`, `--read-hits <qualified_name,…>` (the `read_set.candidates` names), `--note`, `--family`, `--batch <jsonl\|->` (one JSON object per line, e.g. `{"repo":"a","feature":"x.y","verdict":"has","author":"me","at":["src/a.rs:1-9"]}`; one store open and one transaction; one envelope line per row with its `line`, printed as each row is decided, then a `{rows, ok, skipped, failed, committed}` summary; rows are durable only when that last line says `committed`; exit 1 if any row failed; a row equal to the live claim (same run, verdict, author, note and locations) is skipped, reported `skipped: true` with the existing id, and counted in `skipped`, so an interrupted batch can be re-run) |
| `claims` | Latest claim per clone and feature. | `--repo`, `--family`, `--feature`, `--history` |
| `decide <REPO> <DISPOSITION>` | Record `adopt`, `reject` or `park`. Append-only. | `--why`, `--by` (both required), `--feature`, `--family`, `--batch <jsonl\|->` (rows like `{"repo":"a","disposition":"park","why":"...","by":"me"}`; output as for `claim`) |
| `decisions` | Latest decision per scope. | `--repo`, `--history` |
| `matrix <FAMILY>` | Features against every clone: verdicts, prevalence, gaps, differentiators. A clone with no stored relatedness band shows `relatedness: "not run"`, and the envelope's `limitations` says how many comparators lack one. | `--relate` (score the comparators with no band first, then build the matrix; this writes relatedness rows) |

## Exports and agents

| Command | What it does | Flags that matter |
|---|---|---|
| `export json` | The graph as JSON (to stdout). | `--repo`, `--out` |
| `export csv` | RFC 4180 CSV: nodes, edges, claims, decisions, one matrix per family. | `--out` (required) |
| `export vault` | An Obsidian-style markdown vault, regenerated whole. | `--out` |
| `mcp` | Serve the read verbs over MCP stdio (JSON-RPC, one object per line). | |
| `setup` | Register the MCP server with agent clients. A plan by default. | `--client`, `--scope`, `--apply`, `--force`, `--bin`, `--no-rules`, `--no-hook` |


[Back to README](../README.md)
