# @massrelay-ai/comparegraph

CompareGraph compares codebases with evidence: it builds a code graph per repository and records claims about features
with their locations. This package runs the prebuilt `comparegraph` binary, so Node users need no Rust toolchain.

```bash
npx -y @massrelay-ai/comparegraph --version
```

As an MCP server entry (stdio):

```json
{ "command": "npx", "args": ["-y", "@massrelay-ai/comparegraph", "mcp"] }
```

Prebuilt for linux-x64, linux-arm64, darwin-arm64, darwin-x64 and win32-x64, each in its own package that npm installs
as an optional dependency. On another platform, or with `--no-optional`, use the install script or build from source.

Everything else (commands, install options, the method) is in the
[GitHub README](https://github.com/massrelay-ai/comparegraph#readme).
