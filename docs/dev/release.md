# Cutting a release

Maintainers only. A release is a tag; the workflow in `.github/workflows/release.yml` does the rest.

## Before the tag

1. The version is `[workspace.package] version` in `Cargo.toml`, and `server.json` repeats it (CI checks they agree).
   Bump both in the commit that will be tagged; every release job exits 4 when the tag disagrees.
2. Date the section in `CHANGELOG.md` (`## [x.y.z] - YYYY-MM-DD`), leave an empty `## [Unreleased]` above it, and update
   the two link lines at the bottom.
3. `cargo test -p comparegraph` and `cargo clippy -p comparegraph --tests -- -D warnings` green; the `ci` workflow green on
   `main` for that commit.

## The tag

```bash
git tag -a v0.1.0 -m "v0.1.0"
git push origin v0.1.0
```

## What the workflow does

- `build`: five jobs, one per platform (linux-x64, linux-arm64, darwin-arm64, darwin-x64, win32-x64), each on a runner
  of that platform. Each checks the version, builds with `--locked`, strips the Linux binaries, and uploads the binary
  with its `.sha256`. Windows builds with `clang-cl`: one grammar's C build passes a GCC-style warning flag that `cl`
  rejects.
- `release`: downloads the five binaries, writes `SHA256SUMS`, checks it, and publishes a GitHub release with the
  binaries, the sums and generated notes.
- `publish-registry`: downloads a pinned `mcp-publisher` (checksum verified), checks that `server.json` carries the
  tag's version, validates it, authenticates through GitHub OIDC (the registry grants this repository's owner the
  `io.github.massrelay-ai/*` namespace) and publishes. The entry carries the repository, not a package: the install
  script is the distribution.

The install scripts (`scripts/install.sh`, `scripts/install.ps1`) read `releases/latest/download/`, so they serve the new
version as soon as the release exists; nothing in them is versioned. The Homebrew tap (`massrelay-ai/homebrew-comparegraph`)
updates its formula from the latest release on a schedule and on demand (its own workflow, no secret).

## After the release

On a clean machine:

```bash
curl -fsSL https://raw.githubusercontent.com/massrelay-ai/comparegraph/main/scripts/install.sh | sh
comparegraph --version
```

In Claude Code: `/plugin install comparegraph --marketplace massrelay-ai/comparegraph`, then `comparegraph doctor` in a
workspace from the agent. Run the tap's update workflow and `brew install massrelay-ai/comparegraph/comparegraph`. Look for
`io.github.massrelay-ai/comparegraph` at https://registry.modelcontextprotocol.io/v0/servers?search=comparegraph. Then move
the changelog on: a new empty `## [Unreleased]` is already there.
