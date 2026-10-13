# Cutting a release

Maintainers only. A release is a tag; the workflow in `.github/workflows/release.yml` does the rest.

## Before the tag

1. The version lives in one place, `[workspace.package] version` in `Cargo.toml`. Bump it in the commit that will be tagged;
   `scripts/package-npm.sh` stamps that version into the npm packages at build time and exits 4 when the tag disagrees.
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

- `build`: four jobs, one per platform (linux-x64, darwin-arm64, darwin-x64, win32-x64), each on a runner of that platform.
  Each builds the release binary, stamps the npm packages, uploads the binary with its `.sha256`, and uploads the stamped
  platform package as a tarball (tar keeps the executable bit; a plain artifact does not).
- `release`: downloads the four binaries, writes `SHA256SUMS`, checks it, and publishes a GitHub release with the binaries,
  the sums and generated notes.
- `publish-npm`: runs in the `npm` environment, which is configured with a required reviewer, so it waits for a manual
  approval. It needs the `NPM_TOKEN` repository secret and skips cleanly without it. It verifies every package version
  against the tag and that each Unix binary is executable, then publishes the four `@comparegraph/<platform>` packages and
  last the `comparegraph` shell, with provenance. A package already on npm at that version is skipped, so the job can be
  re-run.

The npm scope `@comparegraph` must exist on npm (an organisation or a user of that name) and the token must be allowed to
publish in it.

## After the release

On a clean machine, or in an empty directory with a cleared npm cache:

```bash
npx -y comparegraph@latest --version       # the shell resolves the platform package and runs the binary
```

In Claude Code: `/plugin install comparegraph --marketplace massrelay-ai/comparegraph`, then `comparegraph doctor` in a
workspace from the agent. Download one release asset and check it against `SHA256SUMS`. Then move the changelog on: a new
empty `## [Unreleased]` is already there.
