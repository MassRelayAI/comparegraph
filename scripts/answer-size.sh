#!/bin/sh
# Size of what an agent reads to start (map answers) against the source it replaces.
#
# Usage: scripts/answer-size.sh [--binary <path>] <workspace-dir> <family-dir> <clone-dir>...
#
#   <workspace-dir>  created if missing; each clone is added (name = basename) and built there
#   <family-dir>     a family pack, e.g. families/code-graph-tools; the family name is its basename
#   <clone-dir>...   clones to measure; they are read (find, and the binary's parser), never run, never entered
#   --binary <path>  the comparegraph binary (default: comparegraph on PATH)
#
# Prints one JSON document on stdout, the shape of docs/assets/cost.json. Progress goes to stderr.
# Source bytes: files with a source extension (rs py ts tsx js mjs go c cc cpp h hpp java kt swift rb php) outside
# .git, node_modules, vendor, third_party and target. Answer bytes: the JSON envelope each verb prints.
# Needs node (JSON assembly and timing). Then render: node scripts/readme-assets.mjs
set -eu

bin=comparegraph
if [ "${1:-}" = "--binary" ]; then bin=$2; shift 2; fi
[ $# -ge 3 ] || { sed -n 2,13p "$0" >&2; exit 2; }
ws=$1; fam=$2; shift 2
mkdir -p "$ws"
family=$(basename "$fam")
tmp=$(mktemp -d)
trap 'rm -r "$tmp"' EXIT
: > "$tmp/clones.tsv"

for clone in "$@"; do
  name=$(basename "$clone")
  echo "add + build $name" >&2
  "$bin" --root "$ws" add "$clone" --name "$name" > /dev/null
  "$bin" --root "$ws" build "$name" > /dev/null
  bytes=$(find "$clone" \( -name .git -o -name node_modules -o -name vendor -o -name third_party -o -name target \) -prune -o \
    -type f \( -name '*.rs' -o -name '*.py' -o -name '*.ts' -o -name '*.tsx' -o -name '*.js' -o -name '*.mjs' -o -name '*.go' \
    -o -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.h' -o -name '*.hpp' -o -name '*.java' -o -name '*.kt' \
    -o -name '*.swift' -o -name '*.rb' -o -name '*.php' \) -printf '%s\n' | awk '{s+=$1} END {print s+0}')
  printf '%s\t%s\n' "$name" "$bytes" >> "$tmp/clones.tsv"
done

echo "family add $family" >&2
"$bin" --root "$ws" family add "$fam" > /dev/null

node - "$bin" "$ws" "$family" "$tmp/clones.tsv" <<'JS'
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const [bin, ws, family, tsv] = process.argv.slice(2);
function run(args) {
  const t = process.hrtime.bigint();
  const r = spawnSync(bin, ['--root', ws, ...args], { maxBuffer: 1 << 30 });
  if (r.status !== 0) { console.error(`failed: ${args.join(' ')}\n${r.stderr}`); process.exit(1); }
  return { bytes: r.stdout.length, seconds: Number(process.hrtime.bigint() - t) / 1e9, out: r.stdout.toString() };
}
const version = spawnSync(bin, ['--version']).stdout.toString().trim();
const clones = fs.readFileSync(tsv, 'utf8').trim().split('\n').map((l) => {
  const [name, source_bytes] = l.split('\t');
  const m = run(['map', name, family]);
  return { name, source_bytes: Number(source_bytes), map_bytes: m.bytes, map_seconds: Math.round(m.seconds * 100) / 100 };
});
const matrix = run(['matrix', family]);
const env = JSON.parse(matrix.out);
const d = env.data || env;
const rows = d.features || d.rows || [];
const doc = {
  measured: new Date().toISOString().slice(0, 10),
  binary: version,
  family,
  features: rows.length,
  method: 'source bytes: files with a source extension outside .git, node_modules, vendor, third_party, target; answer bytes: the JSON envelope of each verb; tokens are bytes / 4, an estimate',
  clones,
  matrix_bytes: matrix.bytes,
  relate_bytes: run(['relate']).bytes,
};
process.stdout.write(JSON.stringify(doc, null, 2) + '\n');
JS
