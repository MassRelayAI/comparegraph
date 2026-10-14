# Plugin protocol v1 (external providers)

An external provider is any executable that speaks JSON lines on stdin/stdout. The host (`providers/external.rs`) starts one
process per request, writes ONE request line, closes stdin, and reads answer lines until the first `ok` answer. Everything
the plugin prints is data: it is parsed into our own types and never interpreted as instructions.

## Discovery
1. `<workspace>/comparegraph.toml`:
   ```toml
   [providers.mine]
   command = "tools/my-provider"   # relative paths resolve against the workspace
   ```
2. otherwise an executable named `comparegraph-provider-<name>` on `PATH`.

Use it with `comparegraph build <repo> --provider <name>`, `comparegraph conform <name>`; `comparegraph doctor` lists every
external provider (name, command, manifest state, last conformance result from `.comparegraph/conform-<name>.json`).

## Manifest (beside the command): `comparegraph-plugin.toml`
```toml
kind = "provider"
name = "mine"
version = "0.1.0"
protocol = "1"
languages = ["python", "typescript"]
network = false
```
No manifest means `unverified`: the provider runs only with `--allow-unverified` (or `COMPAREGRAPH_ALLOW_UNVERIFIED=1`).
A manifest whose `protocol` is not `"1"` is refused. `network` is declared and passed on as `COMPAREGRAPH_NETWORK=0|1`; the
host does not sandbox the process, so the declaration is a contract, not a wall.

## Requests and answers
Environment: `COMPAREGRAPH_REPO_ROOT` (build only), `COMPAREGRAPH_NETWORK`. No arguments.

| request | `data` of the `{"ok":true,"data":…}` answer |
|---|---|
| `{"comparegraph":"1","op":"capabilities"}` | `{"languages":[…],"edge_kinds":[…],"communities":false,"embeddings":false}`; optional `"unavailable":"<reason and install hint>"` when the plugin's own engine is missing |
| `{"comparegraph":"1","op":"build","repo_root":"/abs/clone","options":{"max_file_bytes":1048576,"max_files":50000,"full":false}}` | `{"nodes":[…],"edges":[…],"limitations":[…],"files_seen":n,"files_parsed":n}` (the two counts are optional) |

* node: `kind` (file, module, class, function, method, type, route, test), `name`, `qualified_name`, `path`,
  `line_start`/`line_end` (or `"lines":[a,b]`), `content_hash`, `language`, optional `provider_ref`.
* edge: `kind` (contains, imports, calls, implements, extends, tests, routes_to), `source_qn`, `target_qn`, `confidence`.
  A confidence outside [0,1] becomes 0.5.
* limitation: `{"code","detail"}` (or a plain string).
* A row with an unknown kind is dropped; one `unknown_kind` limitation counts them.
* Failure: `{"ok":false,"error":{"code":"…","message":"…"}}`.
* Progress: any number of `{"progress":{"files":n}}` lines before the answer; they are counted, not stored.

## An engine that is not installed
A plugin that wraps another tool answers `capabilities` with `"unavailable": "<reason>"` when that tool is missing. `doctor` shows
the reason on the provider's row (`unavailable`), and `build` refuses before it starts a run (`provider_unavailable`, no run row).
The shipped example is `providers/crg/` (the code-review-graph adapter; `comparegraph.toml` at the repository root registers it).

## Limits and failure
Timeout 1800 s (`COMPAREGRAPH_PLUGIN_TIMEOUT`, whole seconds; an unparsable value or `0` is refused before the run, never
defaulted), stdout cap 256 MiB (enforced while a line is read, so one endless line is never buffered). A timeout, a line that
is not JSON, an oversize output, an `ok:false` answer, a stdout that closes while the process keeps running (it is killed
after a 2 s grace), or a non-zero exit all become `provider_unavailable`. The child's whole process group is killed. No nodes,
edges or communities are written.

Which cases leave a run row: once the build has started, a failure writes a `failed` run row whose limitation is
`provider_unavailable` (timeout, garbage, oversize, `ok:false`, exit, EOF). Failures that happen before the row exists leave
none: a missing binary, an unverified manifest or protocol mismatch, no passing conformance result, a bad
`COMPAREGRAPH_PLUGIN_TIMEOUT`.

A valid answer with no files is `empty_build`, not `provider_unavailable`. The plugin's own `files_seen`/`files_parsed` are
taken as given; when `files_parsed` is absent it is inferred from the file nodes and a `files_parsed_inferred` limitation says so.

Paths: a node whose `path` is absolute, has a `..` segment (`/` or `\\`), a NUL or a drive letter is dropped, as are edges to
it; one `bad_path` limitation counts them.

A `capabilities` probe (doctor, build's pre-check) has its own deadline: 10 s, `COMPAREGRAPH_PLUGIN_CAPS_TIMEOUT` (whole seconds) overrides it, never longer than the build timeout.

## Conformance gate (per language)
`comparegraph conform <name>` writes `.comparegraph/conform-<name>.json` with one result per fixture; a fixture is named for its
language (`go`, `java`, `python`, `rust`, `typescript`; `typescript` also covers tsx, javascript, jsx and mjs). `doctor` shows the
overall `passed` (all fixtures) and the split by language (`conformed`, `failed`, `summary`: "conformed: go, python, typescript; failed: java, rust"). `build` is stricter about the repo in front of it and looser about the rest: it computes the
languages present in the clone once (the reference walker's extension map), and refuses only when a present language has a failed
fixture, or when there is no result file at all. A language without a fixture is not judged. A repo with none of the failed
languages builds with no waiver. A result with no fixture list falls back to its overall `passed`. The refusal names the failed
languages and `comparegraph conform <name>`, and leaves no run row. `--allow-unverified` (or `COMPAREGRAPH_ALLOW_UNVERIFIED=1`)
waives this and the manifest requirement. `conform` itself always runs. `detect()` no longer reads the conformance result.

## Example exchange
```
> {"comparegraph":"1","op":"build","repo_root":"/clones/app","options":{"max_file_bytes":1048576,"max_files":50000,"full":false}}
< {"progress":{"files":1}}
< {"ok":true,"data":{"nodes":[{"kind":"file","name":"a.py","qualified_name":"a.py","path":"a.py","language":"python"},{"kind":"function","name":"f","qualified_name":"a.py::f","path":"a.py","line_start":1,"line_end":2,"language":"python"}],"edges":[{"kind":"contains","source_qn":"a.py","target_qn":"a.py::f","confidence":1}],"limitations":[]}}
```

## A 25-line Python provider
```python
#!/usr/bin/env python3
# comparegraph-provider-minipy: one function node per `def` in *.py
import json, os, re, sys
req = json.loads(sys.stdin.readline())
if req.get("op") == "capabilities":
    print(json.dumps({"ok": True, "data": {"languages": ["python"], "edge_kinds": ["contains"],
                                           "communities": False, "embeddings": False}}))
    sys.exit(0)
root = req["repo_root"]
nodes, edges = [], []
for dp, _, files in os.walk(root):
    if ".git" in dp.split(os.sep):
        continue
    for fn in files:
        if not fn.endswith(".py"):
            continue
        rel = os.path.relpath(os.path.join(dp, fn), root)
        nodes.append({"kind": "file", "name": fn, "qualified_name": rel, "path": rel, "language": "python"})
        for i, line in enumerate(open(os.path.join(dp, fn), errors="replace"), 1):
            m = re.match(r"def (\w+)", line)
            if m:
                qn = f"{rel}::{m.group(1)}"
                nodes.append({"kind": "function", "name": m.group(1), "qualified_name": qn, "path": rel,
                              "line_start": i, "line_end": i, "language": "python"})
                edges.append({"kind": "contains", "source_qn": rel, "target_qn": qn, "confidence": 1})
    print(json.dumps({"progress": {"files": len(nodes)}}), flush=True)
print(json.dumps({"ok": True, "data": {"nodes": nodes, "edges": edges, "limitations": []}}))
```

## The second engine: code-review-graph through the adapter

[`providers/crg/`](../../providers/crg) is `comparegraph-provider-crg`, an adapter that runs
code-review-graph 2.3.9 through the protocol (registered in the root `comparegraph.toml`; code-review-graph itself must be
installed, else `doctor` shows it `unavailable`). `comparegraph build <repo> --providers reference,crg` builds both and
prints an agreement table. On srclight: calls Jaccard 0.962, contains 0.981, imports 0.904, tests 0.594 (below the 0.8 mark),
nodes 0.797 to 1.000 by kind ([parity report](../reports/2026-10-12-engine-parity.md), "Agreement between the engines").
Adapter conformance: go, python and typescript pass; java (edge recall 0.77) and rust (nodes 0.86, edge recall 0.53) stay below
the floors, so the gate refuses it on clones that contain them. Through the adapter, code-review-graph scores like its native
self on three gold projects and slightly higher on hono and caddy (the [gold table](../../README.md#how-good-is-it)).

To add your own engine, write a provider and pass `comparegraph conform <name>`; a provider below the floor is refused.
