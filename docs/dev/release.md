# Cutting a release

Maintainers only. A release is a tag; the workflow in `.github/workflows/release.yml` does the rest.

## Before the tag

1. The version is `[workspace.package] version` in `Cargo.toml`; `Cargo.lock` (the `comparegraph` entry), `server.json`,
   `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json` and `npm/package.json` (its own version and its five
   `optionalDependencies`) repeat it, and `docs/catalog.json` carries it
   (`comparegraph catalog > docs/catalog.json` regenerates it). CI checks `server.json`; `cargo test` checks the rest.
   Bump them in the commit that will be tagged; every release job exits 4 when the tag disagrees.
2. Date the section in `CHANGELOG.md` (`## [x.y.z] - YYYY-MM-DD`), leave an empty `## [Unreleased]` above it, and update
   the two link lines at the bottom.
3. `cargo test -p comparegraph` and `cargo clippy -p comparegraph --tests -- -D warnings` green; the `ci` workflow green on
   `main` for that commit.

## The tag

```bash
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

## What the workflow does

- `build`: five jobs, one per platform (linux-x64, linux-arm64, darwin-arm64, darwin-x64, win32-x64), each on a runner
  of that platform. Each checks the version, builds with `--locked`, strips the Linux binaries, and uploads the binary
  with its `.sha256`. Windows builds with `clang-cl`: one grammar's C build passes a GCC-style warning flag that `cl`
  rejects.
- `release`: downloads the five binaries, writes `SHA256SUMS`, checks it, and publishes a GitHub release with the
  binaries, the sums and generated notes.
- `publish-npm`: runs only when the repository variable `NPM_PUBLISH` is `true`. Checks `npm/package.json` against the tag
  (exit 4), takes the released binaries from the same artifacts (checksums verified), builds six packages with
  `npm/build.mjs`, publishes the five platform packages and then the wrapper through npm trusted publishing.
- `publish-registry`: downloads a pinned `mcp-publisher` (checksum verified), checks that `server.json` carries the
  tag's version, validates it, authenticates through GitHub OIDC (the registry grants this repository's owner the
  `io.github.massrelay-ai/*` namespace) and publishes. The entry carries the repository, not a package: the install
  script is the distribution.

## npm

`@massrelay-ai/comparegraph` is a launcher; the binary comes from one of five platform packages
(`@massrelay-ai/comparegraph-<platform>`) that npm installs as optional dependencies. `npm/build.mjs <version> --bins <dir>
--out <dir>` builds them from the release binaries; `scripts/test/npm-package.test.mjs` builds, packs, installs and runs them.

One-time steps for the maintainer:

1. Create the `massrelay-ai` organisation on npmjs.com.
2. Make the six package names exist and trust this repository. A package must exist before npm lets you configure a trusted
   publisher on it, so either publish the six packages by hand once (`node npm/build.mjs <version> --bins <dir> --out
   dist-npm`, then `npm publish --access public` in each directory, platform packages first), or use whatever npmjs.com offers
   for configuring a publisher on a name not yet published. Then, for each of the six packages, add a trusted publisher:
   GitHub Actions, repository `massrelay-ai/comparegraph`, workflow filename `release.yml`.
3. In the repository settings (Actions, Variables) set `NPM_PUBLISH` to `true`. Until then the `publish-npm` job is skipped
   and a release is unaffected.

The job needs npm 11.5.1 or later and Node 22.14 or later (it installs the latest npm); provenance is attached automatically
for a public repository.

After the first successful npm publish, add this entry to `server.json` (not before: `mcp-publisher publish` checks
`mcpName` on the live npm package and fails the registry step while the package is absent):

```json
"packages": [
  {
    "registryType": "npm",
    "registryBaseUrl": "https://registry.npmjs.org",
    "identifier": "@massrelay-ai/comparegraph",
    "version": "<version>",
    "runtimeHint": "npx",
    "transport": { "type": "stdio" },
    "packageArguments": [{ "type": "positional", "value": "mcp" }]
  }
]
```

The `version` there is the released one, so it moves with every release like the other version files.

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
