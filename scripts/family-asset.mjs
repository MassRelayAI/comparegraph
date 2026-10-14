// Derives docs/assets/family.json from the 30-feature pack and the agreed labels of the mapping gold.
// node scripts/family-asset.mjs            write docs/assets/family.json
// node scripts/family-asset.mjs --check    exit 1 if the committed JSON differs from what the sources give
// Only cells where both readers agreed (`final` not null) are kept. Deterministic: rows.md order, clone names sorted.
import { readFileSync, writeFileSync, readdirSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const opt = (n, d) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);

export function derive(rootDir) {
  const rows = readFileSync(path.join(rootDir, 'families/code-graph-tools/rows.md'), 'utf8').split('\n')
    .filter((l) => l.startsWith('|')).slice(2)
    .map((l) => l.split('|').map((s) => s.trim()).slice(1, 4));
  const groups = [];
  for (const [id, group] of rows) {
    let g = groups.find((x) => x.id === group);
    if (!g) groups.push((g = { id: group, features: [] }));
    g.features.push(id);
  }
  const dir = path.join(rootDir, 'fixtures/gold-map');
  const clones = readdirSync(dir).filter((d) => existsSync(path.join(dir, d, 'features.jsonl'))).sort();
  const cells = [];
  const counts = { has: 0, partial: 0, none: 0, unresolved: 0 };
  const order = new Map(rows.map((r, i) => [r[0], i]));
  for (const clone of clones) {
    const recs = readFileSync(path.join(dir, clone, 'features.jsonl'), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
    for (const r of recs) {
      const v = r.final === null ? null : typeof r.final === 'string' ? r.final : r.final.verdict;
      if (v === null || v === 'unknown') { counts.unresolved++; continue; }
      counts[v]++;
      cells.push({ feature: r.feature_id, clone, verdict: v });
    }
  }
  cells.sort((a, b) => order.get(a.feature) - order.get(b.feature) || clones.indexOf(a.clone) - clones.indexOf(b.clone));
  return {
    family: 'code-graph-tools',
    source: 'families/code-graph-tools/rows.md; fixtures/gold-map/*/features.jsonl (final = both readers agree)',
    groups, clones, cells, counts,
  };
}

const out = opt('--out', path.join(root, 'docs/assets/family.json'));
const text = JSON.stringify(derive(root), null, 1) + '\n';
if (args.includes('--check')) {
  if (!existsSync(out) || readFileSync(out, 'utf8') !== text) { console.error('docs/assets/family.json differs from its sources; run node scripts/family-asset.mjs'); process.exit(1); }
  console.log('family.json is current');
} else writeFileSync(out, text);
