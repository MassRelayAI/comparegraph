# FAQ

Questions about what CompareGraph is, what it is not and why it answers as it does.

## How is this different from code-review-graph and codegraph?

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

## Does it use an LLM?

No. An agent fits in the loop: it reads `map` candidates, opens the cited lines, and writes the
`claim` with its own name as `--author`.

## Can it adopt code for me?

No. It gives you `path:start-end` and the clone's licence lane. Taking the code is a person's
decision, recorded with `decide`. A port-kit export is on the [roadmap](roadmap.md).

## Why are some call edges 0.5?

A call matched by name only, with no import or declared type behind it, is a guess, stored
at 0.5 (0.4 to 2.0 percent of call edges on the three parity projects; 24 to 33 percent on hono and caddy before Plan 6).
Queries take `--min-confidence 1.0` to drop them.

## Is it free? Who supports it?

Yes: Apache-2.0, with no paid tier and no account. Questions and bugs go to
[GitHub issues](https://github.com/massrelay-ai/comparegraph/issues); there is no support contract. Security reports go to
support@massrelay.ai or a private advisory, acknowledged within seven days ([`SECURITY.md`](../SECURITY.md)).

## Is my code sent anywhere?

No. The engine makes no network calls: it has no HTTP or socket dependency and no use of `std::net`. Check it yourself in a
clone of this repository; both commands print nothing (run on 2026-10-24 at commit `9e0ab8d`):

```bash
cargo tree -p comparegraph -e normal --prefix none | grep -iE 'reqwest|hyper|ureq|curl|http|tokio|socket|rustls|openssl|mio'
grep -rn 'std::net' crates/
```

Not yet enforced: no CI test or `deny.toml` ban fails when such a dependency is added; `cargo-deny` checks licences,
advisories and sources only. An external provider you register is a separate program and may use the network; its
manifest says so in `network`, which is passed to it and not enforced. The install script, `npx` and Homebrew download the
binary; after that, nothing does.

## Is it safe to point at a repository I do not trust?

It reads the clone and never runs, imports or builds anything in it. What the reading does with a hostile tree:

- Symbolic links are not followed: a link to a file or a folder outside the clone is skipped (tried on 2026-10-24: a
  clone holding one real file and two links outside it parsed one file).
- A file over 1,048,576 bytes is skipped and counted (`--max-file-bytes`); at most 50,000 files are kept, by sorted path
  (`--max-files`); minified files and the folders in the README's skip list are skipped.
- Files are parsed by tree-sitter grammars compiled into the binary. This project has not fuzzed them. A crafted input that
  makes CompareGraph hang, exhaust memory, run code, or read or write outside the workspace is in scope for
  [`SECURITY.md`](../SECURITY.md).
- An external provider's output is parsed as data: an absolute path, a `..` segment, a NUL or a drive letter in a node path
  is dropped, and its run is bounded by a timeout and an output cap ([plugin protocol](dev/plugin-protocol.md)).

Not offered yet: a software bill of materials (SBOM) for a release, a reproducible-build statement, and a pinned hash for an
external provider.

## What does it not do?

It does not diff two versions, run a clone, or infer types. A `none` claim means a search found
nothing, not that the feature is absent. Entry points and dead code are candidates, never proof.

[Back to README](../README.md)
