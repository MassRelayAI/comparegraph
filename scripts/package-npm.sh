#!/usr/bin/env bash
# Usage: scripts/package-npm.sh [--cross <triple>] [--allow-version-mismatch] <version> <platform-arch>
# Builds the release binary, strips it (linux hosts only), copies it into
# packages/platforms/<platform-arch>/bin/ and stamps <version> into the platform package and the
# shell package. Idempotent.
#   exit 2 unknown platform, 3 host does not match <platform-arch>, 4 version differs from Cargo.toml.
#   --cross <triple>  skip the host check, build with `cargo build --target <triple>` and read
#                     target/<triple>/release/ (UNTESTED on this host).
#   --allow-version-mismatch  downgrade the Cargo.toml version check to a warning.
set -euo pipefail

cross=""; allow_mismatch=0; pos=()
while [ $# -gt 0 ]; do
  case "$1" in
    --cross) cross="${2:?--cross needs a triple}"; shift 2 ;;
    --allow-version-mismatch) allow_mismatch=1; shift ;;
    *) pos+=("$1"); shift ;;
  esac
done
version="${pos[0]:?usage: package-npm.sh [--cross <triple>] [--allow-version-mismatch] <version> <platform-arch>}"
target="${pos[1]:?usage: package-npm.sh [--cross <triple>] [--allow-version-mismatch] <version> <platform-arch>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
plat="$root/packages/platforms/$target"
shell="$root/packages/comparegraph"

[ -f "$plat/package.json" ] || { echo "package-npm: unknown platform '$target' (no $plat/package.json)" >&2; exit 2; }

host_os="$(uname -s)"; host_arch="$(uname -m)"
case "$host_os/$host_arch" in
  Linux/x86_64) host=linux-x64 ;;
  Darwin/arm64) host=darwin-arm64 ;;
  Darwin/x86_64) host=darwin-x64 ;;
  MINGW*/x86_64|MSYS*/x86_64|CYGWIN*/x86_64) host=win32-x64 ;;
  *) host="$host_os/$host_arch" ;;
esac
if [ -z "$cross" ] && [ "$host" != "$target" ]; then
  echo "package-npm: host $host cannot build $target; run this on a $target runner" >&2
  exit 3
fi

cargo_version="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' "$root/Cargo.toml" | head -1)"
if [ "$version" != "$cargo_version" ]; then
  if [ "$allow_mismatch" = 1 ]; then
    echo "package-npm: warning: version $version differs from Cargo.toml $cargo_version" >&2
  else
    echo "package-npm: version $version differs from Cargo.toml $cargo_version (use --allow-version-mismatch)" >&2
    exit 4
  fi
fi

cargo_args=(build --release -p comparegraph)
rel="${CARGO_TARGET_DIR:-$root/target}/release"
if [ -n "$cross" ]; then cargo_args+=(--target "$cross"); rel="${CARGO_TARGET_DIR:-$root/target}/$cross/release"; fi
(cd "$root" && CARGO_BUILD_JOBS=6 cargo "${cargo_args[@]}")

exe=comparegraph; case "$target" in win32-*) exe=comparegraph.exe ;; esac
src="$rel/$exe"
[ -f "$src" ] || { echo "package-npm: $src not found" >&2; exit 1; }

mkdir -p "$plat/bin"
cp "$src" "$plat/bin/$exe"
chmod +x "$plat/bin/$exe"
# strip on linux hosts only: macOS strip can break code signing, windows has none
if [ "$host_os" = Linux ]; then
  strip "$plat/bin/$exe" 2>/dev/null || echo "package-npm: strip unavailable or failed; continuing" >&2
fi

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
