// scripts/readme-assets.mjs: deterministic render, and --check goes red when the data or the SVG drifts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const script = path.join(root, 'scripts/readme-assets.mjs');
const costJson = path.join(root, 'docs/assets/cost.json');
const costSvg = path.join(root, 'docs/assets/cost.svg');
const run = (...a) => spawnSync('node', [script, ...a], { encoding: 'utf8' });

test('rendering twice gives byte-identical output', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'cg-assets-'));
  const a = path.join(dir, 'a.svg');
  const b = path.join(dir, 'b.svg');
  assert.equal(run('--out', a).status, 0);
  assert.equal(run('--out', b).status, 0);
  assert.equal(readFileSync(a, 'utf8'), readFileSync(b, 'utf8'));
});

test('--check passes on the committed SVG', () => {
  const r = run('--check');
  assert.equal(r.status, 0, r.stderr);
});

test('a changed number changes the output and --check against the committed SVG fails', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'cg-assets-'));
  const cost = JSON.parse(readFileSync(costJson, 'utf8'));
  cost.clones[0].map_bytes += 1_000_000;
  const edited = path.join(dir, 'cost.json');
  writeFileSync(edited, JSON.stringify(cost));
  const out = path.join(dir, 'out.svg');
  assert.equal(run('--in', edited, '--out', out).status, 0);
  assert.notEqual(readFileSync(out, 'utf8'), readFileSync(costSvg, 'utf8'));
  assert.equal(run('--in', edited, '--out', costSvg, '--check').status, 1);
});
