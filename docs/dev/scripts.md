# Scripts

Everything under `scripts/` reproduces a number in a report or packages a release. Each script prints its usage with no
arguments. They read the clone copies they are given and never execute code inside them. The Python helpers under
`scripts/lib/` hold the scoring rules so that every baseline is scored by the same code as our own engine.

| script | reproduces | report |
|---|---|---|
| `gold-score.sh` | call-edge precision and recall of our engine (or code-review-graph through the adapter) against `fixtures/gold/<project>/calls.jsonl` | engine parity, Plan 6 |
| `compare-engines.sh` | the full head-to-head with code-review-graph: Jaccards by kind, timing, the gold score | engine parity |
| `h2h-codegraph.sh` | codegraph-mcp scored on the same gold call labels, next to our engine | engine parity ("codegraph head-to-head") |
| `map-score.sh` | `map` precision@5 and recall@5, the `relate` band and the absent check against `fixtures/gold-map/<clone>/features.jsonl` | mapping gold |
| `map-baselines.sh` | the feature-mapping baselines (plain full-text OR search, codegraph symbol search) scored by the same rules | engine parity ("Feature-mapping baselines") |
| `gold-map-pairs.sh` | regenerates `fixtures/gold-map/pairs.jsonl` from the family pack, in a fixed order | mapping gold |
| `gold-map-merge.sh` | merges two readers' labels into the committed gold, with agreement and Cohen's kappa per clone | mapping gold |
| `package-npm.sh` | builds and stamps the platform npm packages for a release tag | release workflow |
| `install.sh`, `install.ps1` | the user-facing installers: fetch the release asset for the platform, check it against `SHA256SUMS`, install it (`packages/comparegraph/test/install-script.test.mjs` runs the shell one against local assets; the PowerShell one is untested until a Windows run) | README Install |

The floors in `fixtures/gold/FLOORS.json` and `fixtures/gold-map/FLOORS.json` are measured minus 0.01 and are only ever
raised; `cargo test -p comparegraph` checks them when `COMPAREGRAPH_STUDY` points at a directory holding the clone copies
named in the fixtures' READMEs, and says so on stderr when it does not.
