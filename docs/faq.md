# FAQ

Questions about what CompareGraph is, what it is not and why it answers as it does.

**How is this different from code-review-graph and codegraph?**
Both index one repository for navigation and search; CompareGraph's unit of output is a cell in a features-by-clones matrix,
with a citation. On the [hand-labelled gold](../README.md#how-good-is-it) it scores higher on call-edge precision and recall, and builds at comparable speed
(limits there). What they have that we do not:

| Capability | code-review-graph | codegraph | CompareGraph |
|---|---|---|---|
| Embedding or semantic search | yes | yes | no |
| Markdown or doc indexing and search | not assessed | yes | no |
| A memory store for the agent | not assessed | yes | no |
| PR context and diff-aware review | yes | yes (`pr_context`) | no |
| Execution flows | yes | not assessed | no |
| Compare many repos, cited claims, a vault | claims: no; multi-repo: a search only | not assessed | yes |

The table follows the [parity report](reports/2026-10-12-engine-parity.md) ("Feature parity") for code-review-graph and
the tool lists of codegraph's MCP server, which exposes memory, doc-index and `pr_context` tools. Our parser layer is taken
from graphmind (MIT).

**Does it use an LLM?** No. An agent fits in the loop: it reads `map` candidates, opens the cited lines, and writes the
`claim` with its own name as `--author`.

**Can it adopt code for me?** No. It gives you `path:start-end` and the clone's licence lane. Taking the code is a person's
decision, recorded with `decide`. A port-kit export is on the [roadmap](roadmap.md).

**Why are some call edges 0.5?** A call matched by name only, with no import or declared type behind it, is a guess, stored
at 0.5 (0.4 to 2.0 percent of call edges on the three parity projects; 24 to 33 percent on hono and caddy before Plan 6).
Queries take `--min-confidence 1.0` to drop them.

**Is my code sent anywhere?** No. The engine makes no network calls.

**What does it not do?** It does not diff two versions, run a clone, or infer types. A `none` claim means a search found
nothing, not that the feature is absent. Entry points and dead code are candidates, never proof.


[Back to README](../README.md)
