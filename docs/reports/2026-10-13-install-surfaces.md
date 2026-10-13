---
title: Install surfaces across the twelve study projects, against ours
date: 2026-10-13
valid_until: 2027-01-13
lane: distribution
verdict: npm is one of three registries in use (three of twelve projects), not the norm; the surfaces that recur are a release binary with an install script, a Homebrew tap, an MCP Registry manifest, a one-command agent setup that also writes skills, hooks and a rules line, and a Claude Code plugin; we have the plugin, the agent setup for six clients, npm and release assets, and lack the install script, the registry manifest, the tap, hooks and the rules line
status: decided; shipped 2026-10-14 as the Rust tools do it (binaries, install scripts, a Homebrew tap, the registry entry, the plugin over the binary); npm and crates.io dropped by the maintainer; the rest is in docs/roadmap.md "Distribution"
---

Method: the install sections of each project's README and the files that back them (package manifests, install scripts,
Homebrew formulae, `.claude-plugin/`, `.mcp.json`, `server.json`, hooks and skills directories, the release workflows) were
read in the study copies on 2026-10-13. Nothing was run. Two of the twelve (caddy, hono) are libraries with no agent
surface and are listed for completeness only.

## What each project ships

| project | language | registry | binary install | agent setup | plugin / skills / hooks | MCP Registry |
|---|---|---|---|---|---|---|
| code-review-graph | Python | PyPI (`pip`, `pipx`, `uvx`) | no | `install` detects 16 tools, writes the MCP entry, hooks, skills and a rules line; `uninstall` | 7 skills, SessionStart + PostToolUse hooks, `.mcp.json`, a GitHub Action, a VS Code extension | no |
| codanna | Rust | no (Homebrew, Nix, cargo) | `curl \| sh`, `install.ps1`, `brew`, `nix run` | manual `.mcp.json` snippet | Claude Code plugin (skills only, "needs the CLI") in a subdirectory | no |
| graphmind | Rust | no (Homebrew) | `curl \| sh`, `brew`, direct download, desktop app | `setup` writes hooks, the skill, MCP entries, a CLAUDE.md line, git hooks | `.mcp.json`, one skill | no |
| octocode | Rust | crates.io, ghcr | `curl \| sh`, `brew`, Chocolatey, Scoop | `claude mcp add` | no | `server.json` |
| tokensave | Rust | crates.io | `brew` tap, `cargo install` | manual | no | `server.json` with `mcpb` bundles per platform |
| codeseek | Rust + Node | npm | `brew` (formula in the repo) | manual | no | no |
| rea | Node | npm | `curl \| bash`, `npx` | `npx rea-agents` | no | `server.json`, published by `mcp-publisher` in CI |
| CodeGraph | Rust + Node | npm (`mcp-package`) | VS Code and JetBrains extensions | manual | pre-edit hook scripts in the npm package | `server.json` |
| srclight | Python | PyPI, Docker | `pip` | `claude mcp add` | no | `server.json`, published by `mcp-publisher` in CI; Glama listing |
| ripwire | C++ | no | `install.sh` builds from source | `install.sh` activates skills and advisory hooks for Claude, Codex, Hermes | 18 skills, 5 hooks, `.mcp.json` | no |
| caddy, hono | Go, TS | Go modules, npm | n/a | n/a | n/a | n/a |

Counts over the ten agent-facing projects: install script 5, Homebrew 5, MCP Registry manifest 5 (2 publish it in CI),
agent setup command 4 (one writes 16 tools), skills shipped 4, hooks shipped 4, Claude Code plugin 1, npm 3, crates.io 2,
PyPI 2, Docker 2, Windows package managers 1.

## Ours, the same columns

Rust; npm (four platforms, pending the first publish); release assets with `SHA256SUMS` (pending the tag); `setup` and
`doctor` for six clients (Claude Code, Codex, Cursor, Gemini CLI, OpenCode, Hermes), additive with a backup; a Claude Code
plugin that carries the seven skills and starts the MCP server through `npx`; no install script, no Homebrew tap, no
`server.json`, no hooks, no rules line written by `setup`, no crates.io.

## Verdict

Update 2026-10-14, the maintainer: "they don't use npm, so skip; just follow others". npm was removed the same day; the
first release run had also shown that the npm packaging script's `sed` did not run on macOS. The routes below are the
Rust tools' routes.

The gaps that cost little and reach far, in order:

1. An MCP Registry manifest (`server.json`) and an `mcp-publisher` step in the release workflow. Every MCP client that
   browses the registry finds the server; the npm package must carry `mcpName` for the registry to accept it.
2. `scripts/install.sh` and `install.ps1` that fetch the release asset for the platform and check it against
   `SHA256SUMS`; the `curl | sh` line in the README. Our own script, modelled on the shape the five projects share.
3. `setup` writes more: a rules line into the agent's instruction file (what code-review-graph and graphmind do) and, for
   Claude Code, a SessionStart hook that runs `comparegraph doctor`; the plugin ships the same hook. More clients after
   the six, from code-review-graph's table (MIT).
4. A Homebrew tap (`massrelay-ai/homebrew-tap`, a formula updated by the release workflow); needs a second repository.
5. ~~crates.io~~ dropped 2026-10-14 (the maintainer: nobody uses it); `cargo install --locked --path` stays as the from-source line.

Not taken: Docker (the engine reads local clones), a VS Code extension, a desktop app, a GitHub Action (a PR review is
not our product), Windows package managers (after the tap, if asked).
