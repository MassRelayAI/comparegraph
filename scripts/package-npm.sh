#!/usr/bin/env bash
# Usage: scripts/package-npm.sh <version> <platform-arch>
# Builds the release binary, strips it, copies it into packages/platforms/<platform-arch>/bin/
# and stamps <version> into the platform package and the shell package. Idempotent.
set -euo pipefail

version="${1:?usage: package-npm.sh <version> <platform-arch>}"
target="${2:?usage: package-npm.sh <version> <platform-arch>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
plat="$root/packages/platforms/$target"
shell="$root/packages/comparegraph"

[ -f "$plat/package.json" ] || { echo "package-npm: unknown platform '$target' (no $plat/package.json)" >&2; exit 2; }
cargo_version="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' "$root/Cargo.toml" | head -1)"
[ "$version" = "$cargo_version" ] || echo "package-npm: warning: version $version differs from Cargo.toml $cargo_version" >&2

(cd "$root" && CARGO_BUILD_JOBS=6 cargo build --release -p comparegraph)

exe=comparegraph; case "$target" in win32-*) exe=comparegraph.exe ;; esac
src="${CARGO_TARGET_DIR:-$root/target}/release/$exe"
[ -f "$src" ] || { echo "package-npm: $src not found" >&2; exit 1; }

mkdir -p "$plat/bin"
cp "$src" "$plat/bin/$exe"
chmod +x "$plat/bin/$exe"
strip "$plat/bin/$exe" 2>/dev/null || echo "package-npm: strip unavailable or failed; continuing" >&2

node - "$version" "$plat/package.json" "$shell/package.json" <<'JS'
const fs = require('node:fs');
const [version, ...files] = process.argv.slice(2);
for (const f of files) {
  const j = JSON.parse(fs.readFileSync(f, 'utf8'));
  j.version = version;
  for (const k of Object.keys(j.optionalDependencies || {})) j.optionalDependencies[k] = version;
  fs.writeFileSync(f, JSON.stringify(j, null, 2) + '\n');
}
JS
echo "package-npm: $target @ $version -> $plat/bin/$exe"
