# Usage

The quick start on the shipped sample, what you get from the graph, family packs, configuration, the vault it exports, the rules block `setup` writes and the Rust library.

## Quick start on the shipped sample

The example family pack (`families/`) and the sample repositories (`fixtures/`) ship in the repository, not in the binary,
so clone it once even if you installed the binary another way; the commands below start inside that clone. The sample is
`fixtures/typescript` (three files) against the ten-feature example pack:

```bash
mkdir ../ws && cd ../ws
comparegraph add ../comparegraph/fixtures/typescript --name tsfix   # register a clone (read, never run)
comparegraph build tsfix                                   # parse it: one run, one store hash
comparegraph family add ../comparegraph/families/example-code-graph-tools
comparegraph relate                                        # related | adjacent | unrelated
comparegraph map tsfix example-code-graph-tools            # candidates per feature, writes nothing
comparegraph claim tsfix symbols.extraction has --author me --at src/util.ts:11-11
comparegraph claim tsfix search.embeddings none --author me  # no --searched: the graph check runs
comparegraph matrix example-code-graph-tools               # features x clones, prevalence, gaps
comparegraph export vault --out ./vault                    # open ./vault in Obsidian
```

Every command prints one JSON envelope (add `--pretty` to indent it). Real output from the steps above, trimmed:

```text
build   {"run":1,"nodes":13,"edges":21,"files_parsed":3,"communities":1,
         "store_hash":"8a64f321...","reused":false}
relate  {"repo":"tsfix","family":"example-code-graph-tools","score":0.0,"band":"unrelated"}
claim   {"feature":"symbols.extraction","verdict":"has","confidence":"derived",
         "locations":[{"path":"src/util.ts","line_start":11,"line_end":11,"stale":false}]}
claim   {"feature":"search.embeddings","verdict":"none","confidence":"inferred",
         "note":"graph-absent (names, paths, community terms, kinds; not strings, flags or config keys):
                 terms=embedding,embeddings,search; stems=embedd; channels=symbols:0,paths:0,communities:0,kinds:0"}
claim   a bad citation is refused: "src/nope.ts: file does not exist in the clone"  (error invalid_arguments)
matrix  symbols.extraction -> has (prevalence 1.0);  search.embeddings -> none;  the other eight cells "-"
check   {"repos":1,"runs_done":1,"stale_locations":0,"drifted_runs":0,"schema_ok":true,"problems":[]}
export  {"written":"./vault","repos":1,"files":27,"communities":1,"symbols":10}
```

`relate` says `unrelated`, as it should: a three-file fixture is not a code-graph tool, and `map` finds no candidates. The
matrix has two claimed cells and eight unclaimed `-` cells.

## What you get

Four levels. Each answer says how it knows.

| Level | Example | Where |
|---|---|---|
| Repo surface | licence, licence lane, head commit, role (root or comparator) | `repos`, `show` |
| Structure | module communities with label, keywords, cohesion, size | `communities` |
| Symbols | callers, callees, blast radius, tests that reach a symbol, entry points, dead-code candidates | `show`, `traverse`, `impact`, `tests-for`, `entry-points`, `dead-code` |
| Claims | verdict, citation, author, trail of superseded rows | `claim`, `claims`, `matrix` |

A real `show` answer from the quick start (trimmed):

```json
{"ok":true,"verb":"show","schema":1,
 "data":{"repo":"tsfix","lane":"study",
  "node":{"kind":"method","qualified_name":"src/util.ts::bump","path":"src/util.ts",
          "line_start":11,"line_end":11,"language":"typescript"},
  "community":"src","callers":[],"callees":["src/util.ts::add"],"stale":false,"confidence":"observed"},
 "evidence":{"confidence":"observed","limitations":[]}}
```

Envelope fields: `ok`, `verb`, `schema`, `data`, `evidence`, `page`, `cost`, `error`. The exit code is 0 when `ok` and 1
otherwise. List verbs take `--limit` (default 50, maximum 500) and `--cursor`. Error codes (from
[`docs/catalog.json`](catalog.json)): `below_floor`, `catalog_stale`, `check_failed`, `empty`, `invalid_arguments`,
`io_error`, `not_found`, `provider_unavailable`, `store_error`.

## Family packs

A family is a directory with a `rows.md`: front matter with `family:` and `description:`, then a table with `id`, `group`,
`feature`, `definition`, `counts_as_has`, `tier` (`basics`, `common`, `extra` or `candidate`) and `weight`, plus an optional eighth column `synonyms` (comma-separated words that also name the feature, e.g. `reattempt, backoff`). Synonyms are terms of `map` and of the absent check, and a `claim none --searched` that uses one counts as naming the feature; an undeclared unrelated word is still refused. A header with any other ninth column is refused. A feature id is `<group>.<slug>`: lowercase `[a-z0-9-]`, exactly one dot, and the `group` column must equal the id's group (`symbols.extraction` has group `symbols`); a row that breaks it is refused when the pack loads. Two packs ship:
[`families/example-code-graph-tools`](../families/example-code-graph-tools/rows.md) (10 features, the quick start) and
[`families/code-graph-tools`](../families/code-graph-tools/rows.md) (30 features, the mapping gold). `family propose <name>
--repos a,b,c` drafts a pack from the terms the clones share, with no model. The tier names and band thresholds (`related` at
0.5 or more, `adjacent` at 0.2 or more) are provisional, and a relatedness score is vocabulary overlap, not evidence
([design notes](decisions.md)).

## The vault

`comparegraph export vault --out ./vault` regenerates the whole vault from the store.

```text
vault/
  index.md      llms.txt      AGENTS.md     entry point, a short map, the short workflow for an agent
  repos/        one note per clone
  communities/  symbols/      one folder per clone
  families/     features/     one note per family; one folder per family with a note per feature and its cells
  history/      one note per clone: the dated claim and decision trail, superseded rows marked
```

Every note starts with a front-matter `description:` built from stored facts by a template, not by a model, for example
`function pkg/core.py::add in pkg/core.py:1-2, 3 callers, 0 callees`. An agent reads the descriptions first and opens a note
only when the line is not enough. At 4.4k files the vault is one file per symbol (66,970 files, 275 MB in the proof report), too
large to open as one vault in a note tool.

## Configuration

| Setting | Where | Meaning |
|---|---|---|
| `--root <dir>` | every command (default `.`) | the workspace; the store is `<root>/.comparegraph/comparegraph.db` |
| `COMPAREGRAPH_THREADS` | environment | `build`: cap on per-file analysis threads, 1 to 64 (default 8, also bounded by the CPU count); `0` or an unparsable value is ignored |
| `COMPAREGRAPH_PROFILE` | environment | `build`: `1` prints one stderr line per pass with its wall time and a count |
| `COMPAREGRAPH_PLUGIN_TIMEOUT` | environment | an external provider's run limit in whole seconds (default 1800; `0` or unparsable is refused) |
| `COMPAREGRAPH_PLUGIN_CAPS_TIMEOUT` | environment | the deadline of a `capabilities` probe in whole seconds (default 10) |
| `COMPAREGRAPH_PLUGIN_OUTPUT_CAP` | environment | the plugin stdout cap (default 256 MiB) |
| `COMPAREGRAPH_ALLOW_UNVERIFIED` | environment | `1` is the same as `--allow-unverified` |
| `COMPAREGRAPH_BIN` | environment | the command `setup` registers, when `--bin` is not given |
| `COMPAREGRAPH_NO_PIDNS` | environment | `1` skips the PID-namespace test runner, for contributors ([`docs/dev/pid-namespaces.md`](dev/pid-namespaces.md)) |
| `COMPAREGRAPH_NOW` | environment | a fixed clock for tests; do not set it in real use |
| `--min-confidence` | `traverse`, `impact`, `tests-for`, `entry-points`, `dead-code` | `0.5` (default, includes guessed hops) or `1.0` |
| `--max-file-bytes`, `--max-files`, `--full` | `build` | skip files over this size (1048576); keep the first N files by sorted path (50000); bypass the per-file parse cache |

## The rules block and the hook

The table of clients and their files is in the [README](../README.md#agents-platforms-uninstall).

Besides the entry, `setup` writes a CompareGraph block, between `<!-- comparegraph -->` and `<!-- /comparegraph -->`, into
the agent's instruction file in the workspace root (`--root`, default the current directory; the `Rules file` column; only
once `.comparegraph/` exists there, so run `comparegraph add` first; skip it with `--no-rules`), and for Claude
Code a `SessionStart` hook in `.claude/settings.json` (project scope) or `~/.claude/settings.json` (user scope; skip it with
`--no-hook`). The hook runs `comparegraph doctor --brief`, which prints nothing outside a workspace with a store. A block from
an older release is refreshed in place; a block you edited by hand is left alone unless you pass `--force`. Markers quoted
inside a fenced code block, or a second block, count as the block (the first marker pair wins); a close marker alone is
ignored and a block is appended.

## Library usage

The crate is `comparegraph`. Each verb is a function in `comparegraph::application::*` taking the workspace root; the CLI and
MCP server are thin adapters over them.

```rust
use comparegraph::application::{add, build, matrix};
use comparegraph::providers::BuildOptions;
use std::path::Path;

let root = Path::new(".");
add::add_repo(root, Path::new("../comparegraph/fixtures/typescript"), Some("tsfix"), false)?;
let report = build::build_repo(root, "tsfix", "reference", &BuildOptions::default())?;
let m = matrix::matrix(root, "example-code-graph-tools")?;
println!("{} nodes, {} comparators", report.nodes, m.comparators);
```

The stable contracts: the JSON envelope (`export json`, every CLI answer), and
[`crates/comparegraph/src/store/schema.sql`](../crates/comparegraph/src/store/schema.sql) for read-only SQL on
`.comparegraph/comparegraph.db`. `claim` and `decision` rows are append-only: read them, do not update them.


[Back to README](../README.md)
