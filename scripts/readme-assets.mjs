// Renders docs/assets/cost.svg from docs/assets/cost.json. Deterministic, no dependencies.
// node scripts/readme-assets.mjs            write the SVG
// node scripts/readme-assets.mjs --check    exit 1 if the committed SVG differs from what would be written
// The input and output paths can be overridden with --in and --out (the test uses them).
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const opt = (name, dflt) => (args.includes(name) ? args[args.indexOf(name) + 1] : dflt);
const inPath = opt('--in', path.join(root, 'docs/assets/cost.json'));
const outPath = opt('--out', path.join(root, 'docs/assets/cost.svg'));

export function tokens(bytes) {
  const t = bytes / 4;
  return t >= 1e6 ? `${(t / 1e6).toFixed(1)} M` : `${Math.round(t / 1e3)} k`;
}
const mb = (b) => `${(b / 1e6).toFixed(1)} MB`;
const kb = (b) => `${Math.round(b / 1e3)} KB`;
const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

export function render(cost) {
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

const cost = JSON.parse(readFileSync(inPath, 'utf8'));
const svg = render(cost);
if (args.includes('--check')) {
  const have = existsSync(outPath) ? readFileSync(outPath, 'utf8') : '';
  if (have !== svg) { console.error(`${outPath} differs from the render of ${inPath}; run node scripts/readme-assets.mjs`); process.exit(1); }
  console.log('cost.svg is current');
} else {
  writeFileSync(outPath, svg);
}
