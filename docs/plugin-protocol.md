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

## Conformance gate
An external provider is listed by `doctor` but `build` refuses it until `comparegraph conform <name>` has written a passing
`.comparegraph/conform-<name>.json`; `--allow-unverified` (or `COMPAREGRAPH_ALLOW_UNVERIFIED=1`) waives both this and the
manifest requirement. `conform` itself always runs.

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
