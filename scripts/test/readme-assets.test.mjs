// scripts/readme-assets.mjs: deterministic render of three illustrations, and --check goes red when the data or the SVG drifts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const script = path.join(root, 'scripts/readme-assets.mjs');
const run = (...a) => spawnSync('node', [script, ...a], { encoding: 'utf8' });
const json = (n) => path.join(root, `docs/assets/${n}.json`);
const svg = (n) => path.join(root, `docs/assets/${n}.svg`);

// One number per asset, changed in a temp copy.
const edits = {
  cost: (d) => { d.clones[0].map_bytes += 1_000_000; },
  accuracy: (d) => { d.rows[0].p[0] = 0.5; },
  scale: (d) => { d.trees[0].files += 5000; },
};

for (const name of Object.keys(edits)) {
  test(`${name}: rendering twice gives byte-identical output`, () => {
    const dir = mkdtempSync(path.join(tmpdir(), 'cg-assets-'));
    const a = path.join(dir, 'a.svg');
    const b = path.join(dir, 'b.svg');
    assert.equal(run('--asset', name, '--in', json(name), '--out', a).status, 0);
    assert.equal(run('--asset', name, '--in', json(name), '--out', b).status, 0);
    assert.equal(readFileSync(a, 'utf8'), readFileSync(b, 'utf8'));
    assert.equal(readFileSync(a, 'utf8'), readFileSync(svg(name), 'utf8'), 'the committed SVG is the render');
  });

  test(`${name}: a changed number changes the output and --check against the committed SVG fails`, () => {
    const dir = mkdtempSync(path.join(tmpdir(), 'cg-assets-'));
    const data = JSON.parse(readFileSync(json(name), 'utf8'));
    edits[name](data);
    const edited = path.join(dir, `${name}.json`);
    writeFileSync(edited, JSON.stringify(data));
    const out = path.join(dir, 'out.svg');
    assert.equal(run('--asset', name, '--in', edited, '--out', out).status, 0);
    assert.notEqual(readFileSync(out, 'utf8'), readFileSync(svg(name), 'utf8'));
    assert.equal(run('--asset', name, '--in', edited, '--out', svg(name), '--check').status, 1);
  });
}

test('--check passes on all three committed SVGs', () => {
  const r = run('--check');
  assert.equal(r.status, 0, r.stderr);
  for (const n of Object.keys(edits)) assert.match(r.stdout, new RegExp(`${n}.svg is current`));
});
