// Renders docs/assets/{cost,accuracy,scale}.svg from the JSON of the same name. Deterministic, no dependencies.
// node scripts/readme-assets.mjs            write all three SVGs
// node scripts/readme-assets.mjs --check    exit 1 if a committed SVG differs from what would be written
// --asset <name> limits the run to one of them; with --in/--out (the test uses them) it defaults to cost.
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const opt = (name, dflt) => (args.includes(name) ? args[args.indexOf(name) + 1] : dflt);

export function tokens(bytes) {
  const t = bytes / 4;
  return t >= 1e6 ? `${(t / 1e6).toFixed(1)} M` : `${Math.round(t / 1e3)} k`;
}
const mb = (b) => `${(b / 1e6).toFixed(1)} MB`;
const kb = (b) => `${Math.round(b / 1e3)} KB`;
const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

export function renderCost(cost) {
  const source = cost.clones.reduce((a, c) => a + c.source_bytes, 0);
  const maps = cost.clones.reduce((a, c) => a + c.map_bytes, 0);
  const n = cost.clones.length;
  const base = 1000; // bars are log10(bytes / base) tall
  const maxH = 140;
  const top = Math.log10(source / base);
  const h = (b) => Math.max(6, Math.round((Math.log10(b / base) / top) * maxH));
  const floor = 215;
  const bar = (x, w, bytes, fill) => {
    const bh = h(bytes);
    return `<rect x="${x}" y="${floor - bh}" width="${w}" height="${bh}" rx="4" fill="${fill}" stroke="#57606a" stroke-width="1.2"/>`;
  };
  const label = (x, bytes, text, sub, anchor = 'start') => {
    const y = floor - h(bytes);
    return `<text x="${x}" y="${y - 42}" font-size="13" font-weight="700" text-anchor="${anchor}">${esc(text)}</text>
<text x="${x}" y="${y - 22}" font-size="18" font-weight="700" text-anchor="${anchor}">${esc(sub[0])}</text>
<text x="${x}" y="${y - 4}" font-size="12" text-anchor="${anchor}" fill="#57606a">${esc(sub[1])}</text>`;
  };
  const feats = cost.features;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1050 300" width="1050" role="img" aria-labelledby="t d">
<title id="t">Size of the source against the size of the ranked candidates</title><desc id="d">Six projects hold ${mb(source)} of source, about ${tokens(source)} tokens. The comparegraph map answers for ${feats} features on all six total ${kb(maps)}, about ${tokens(maps)} tokens, and the matrix is ${kb(cost.matrix_bytes)}. Candidates are places to read, not verdicts.</desc>
<defs><marker id="a" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0L10 5L0 10z" fill="#57606a"/></marker></defs>
<rect width="100%" height="100%" fill="#ffffff"/>
<g font-family="Helvetica, Arial, sans-serif" fill="#1f2328">
${bar(40, 150, source, '#ddf4ff')}
${label(40, source, `Source of ${n === 6 ? 'six' : n} projects`, [mb(source), `≈ ${tokens(source)} tokens (estimate)`])}
<line x1="215" y1="170" x2="425" y2="170" stroke="#57606a" stroke-width="1.5" marker-end="url(#a)"/>
<text x="320" y="156" font-size="13" text-anchor="middle" font-family="monospace" fill="#0550ae">comparegraph map</text>
<text x="320" y="190" font-size="12" text-anchor="middle">${feats} features × ${n} projects</text>
${bar(450, 150, maps, '#fff8c5')}
${label(450, maps, 'Ranked candidates, all six', [kb(maps), `≈ ${tokens(maps)} tokens`])}
${bar(660, 90, cost.matrix_bytes, '#fff8c5')}
${label(660, cost.matrix_bytes, `Matrix, ${feats} × ${n}`, [kb(cost.matrix_bytes), `≈ ${tokens(cost.matrix_bytes)} tokens`])}
<line x1="30" y1="${floor}" x2="780" y2="${floor}" stroke="#57606a" stroke-width="1.2"/>
<text x="780" y="${floor + 15}" font-size="11" text-anchor="end" fill="#57606a">bars: log scale</text>
<text x="800" y="70" font-size="13" font-weight="700">What this shows</text>
<text x="800" y="92" font-size="12">What an agent reads to start,</text>
<text x="800" y="108" font-size="12">${Math.round(source / maps)} times smaller than the source.</text>
<text x="800" y="132" font-size="12">What it keeps is a matrix</text>
<text x="800" y="148" font-size="12">whose cells cite lines.</text>
<text x="525" y="268" font-size="12" text-anchor="middle">Candidates are places to read, not verdicts (held-out recall@5 0.149).</text>
<text x="525" y="286" font-size="12" text-anchor="middle">Every matrix cell cites a line that <tspan font-family="monospace" fill="#0550ae">check</tspan> re-verifies.</text>
</g>
</svg>
`;
}

const FONT = 'font-family="Helvetica, Arial, sans-serif" fill="#1f2328"';
const num = (n) => n.toLocaleString('en-US');
const fix = (v, d = 3) => v.toFixed(d);

export function renderAccuracy(a) {
  const colors = ['#0550ae', '#8c959f', '#d4a72c'];
  const rows = a.rows;
  const px = [30, 560]; // panel left edges
  const labelW = 150, barMax = 270, barH = 12, gap = 3, groupH = 3 * barH + 2 * gap + 14, top = 112;
  const panel = (k, name, key) => {
    let o = `<text x="${px[k]}" y="${top - 18}" font-size="15" font-weight="700">${name}</text>\n`;
    const x0 = px[k] + labelW;
    o += `<line x1="${x0}" y1="${top - 6}" x2="${x0}" y2="${top + rows.length * groupH - 8}" stroke="#57606a" stroke-width="1.2"/>\n`;
    rows.forEach((row, i) => {
      const y = top + i * groupH;
      if (i === 0) o += `<rect x="${px[k] - 8}" y="${y - 6}" width="${labelW + barMax + 78}" height="${groupH - 2}" rx="6" fill="#f6f8fa"/>\n`;
      o += `<text x="${x0 - 10}" y="${y + 2 * barH}" font-size="12" text-anchor="end"${i === 0 ? ' font-weight="700"' : ''}>${esc(row.name)}</text>\n`;
      row[key].forEach((v, e) => {
        const by = y + e * (barH + gap);
        if (v === null) { o += `<text x="${x0 + 6}" y="${by + 10}" font-size="11" fill="#57606a">none answered</text>\n`; return; }
        const w = Math.max(1, Math.round(v * barMax));
        o += `<rect x="${x0}" y="${by}" width="${w}" height="${barH}" fill="${colors[e]}"/>\n<text x="${x0 + w + 6}" y="${by + 10}" font-size="11" fill="#57606a">${fix(v)}</text>\n`;
      });
    });
    return o;
  };
  const pooled = rows[0];
  const lx = 30, ly = 62;
  const legend = a.engines.map((e, i) => {
    const x = lx + [0, 150, 440][i];
    return `<rect x="${x}" y="${ly - 10}" width="14" height="14" fill="${colors[i]}"/><text x="${x + 20}" y="${ly + 2}" font-size="13">${esc(e)}</text>`;
  }).join('\n');
  const h = top + rows.length * groupH + 62;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1050 ${h}" width="1050" role="img" aria-labelledby="t d">
<title id="t">${esc(a.title)}</title><desc id="d">Pooled over the gold, ${esc(a.engines[0])} precision ${fix(pooled.p[0])} and recall ${fix(pooled.r[0])}; ${esc(a.engines[1])} ${fix(pooled.p[1])} and ${fix(pooled.r[1])}; ${esc(a.engines[2])} ${fix(pooled.p[2])} and ${fix(pooled.r[2])}. Five per-project rows follow. ${esc(a.caption)}.</desc>
<rect width="100%" height="100%" fill="#ffffff"/>
<g ${FONT}>
<text x="30" y="30" font-size="18" font-weight="700">${esc(a.title)}</text>
${legend}
${panel(0, 'Precision', 'p')}${panel(1, 'Recall', 'r')}<text x="525" y="${h - 22}" font-size="12" text-anchor="middle" fill="#57606a">${esc(a.caption)}</text>
</g>
</svg>
`;
}

export function renderScale(s) {
  const trees = s.trees;
  const lo = 2; // axis starts at 10^2 files
  const decades = 3;
  const x0 = 235, axisW = 330, rowH = 62, top = 84;
  const xf = (f) => x0 + Math.round(((Math.log10(f) - lo) / decades) * axisW);
  const rows = trees.map((t, i) => {
    const y = top + i * rowH;
    const w = xf(t.files) - x0;
    const edges = t.edges === null ? '' : ` · ${num(t.edges)} edges`;
    return `<text x="30" y="${y + 17}" font-size="14" font-weight="700">${esc(t.name)}</text>
<rect x="${x0}" y="${y}" width="${w}" height="26" rx="4" fill="#ddf4ff" stroke="#57606a" stroke-width="1.2"/>
<text x="${x0 + w + 8}" y="${y + 18}" font-size="12" fill="#57606a">${num(t.files)} files</text>
<text x="600" y="${y + 12}" font-size="14" font-weight="700">${num(t.nodes)} nodes${edges}</text>
<text x="600" y="${y + 30}" font-size="13" fill="#0550ae">${t.seconds} s to build</text>`;
  }).join('\n');
  const axisY = top + trees.length * rowH - 8;
  const ticks = [100, 1000, 10000, 100000].map((v) => `<line x1="${xf(v)}" y1="${top - 12}" x2="${xf(v)}" y2="${axisY}" stroke="#d0d7de" stroke-width="1"/><text x="${xf(v)}" y="${axisY + 16}" font-size="11" text-anchor="middle" fill="#57606a">${num(v)}</text>`).join('\n');
  const q = s.queries;
  const qy = axisY + 52;
  const pills = q.items.map((it, i) => {
    const x = 30 + i * 250;
    return `<rect x="${x}" y="${qy}" width="232" height="52" rx="8" fill="#fff8c5" stroke="#57606a" stroke-width="1.2"/>
<text x="${x + 12}" y="${qy + 22}" font-size="13" font-family="monospace" fill="#0550ae">${esc(it.name)}</text>
<text x="${x + 12}" y="${qy + 42}" font-size="16" font-weight="700">${it.seconds} s</text>`;
  }).join('\n');
  const h = qy + 52 + 52;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1050 ${h}" width="1050" role="img" aria-labelledby="t d">
<title id="t">${esc(s.title)}</title><desc id="d">${trees.map((t) => `${t.name}: ${num(t.files)} files, ${num(t.nodes)} nodes${t.edges === null ? '' : `, ${num(t.edges)} edges`}, ${t.seconds} seconds`).join('; ')}. On the ${esc(q.graph)} graph: ${q.items.map((it) => `${it.name} ${it.seconds} seconds`).join(', ')}. ${esc(s.caption)}.</desc>
<rect width="100%" height="100%" fill="#ffffff"/>
<g ${FONT}>
<text x="30" y="30" font-size="18" font-weight="700">${esc(s.title)}</text>
<text x="${x0}" y="56" font-size="12" fill="#57606a">bars: files parsed, log scale</text>
${ticks}
${rows}
<text x="30" y="${qy - 12}" font-size="14" font-weight="700">Queries on the ${esc(q.graph)} graph, and a rebuild with nothing changed</text>
${pills}
<text x="525" y="${h - 22}" font-size="12" text-anchor="middle" fill="#57606a">${esc(s.caption)}</text>
</g>
</svg>
`;
}

const assets = { cost: renderCost, accuracy: renderAccuracy, scale: renderScale };
const custom = args.includes('--in') || args.includes('--out');
const names = args.includes('--asset') ? [opt('--asset')] : custom ? ['cost'] : Object.keys(assets);
let bad = false;
for (const name of names) {
  if (!assets[name]) { console.error(`unknown asset ${name}`); process.exit(2); }
  const inPath = opt('--in', path.join(root, `docs/assets/${name}.json`));
  const outPath = opt('--out', path.join(root, `docs/assets/${name}.svg`));
  const svg = assets[name](JSON.parse(readFileSync(inPath, 'utf8')));
  if (args.includes('--check')) {
    const have = existsSync(outPath) ? readFileSync(outPath, 'utf8') : '';
    if (have !== svg) { console.error(`${outPath} differs from the render of ${inPath}; run node scripts/readme-assets.mjs`); bad = true; }
    else console.log(`${name}.svg is current`);
  } else {
    writeFileSync(outPath, svg);
  }
}
if (bad) process.exit(1);
