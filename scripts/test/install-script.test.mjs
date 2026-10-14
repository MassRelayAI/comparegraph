// scripts/install.sh against a local directory of release assets: a good checksum installs, a bad one refuses.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, existsSync, statSync, rmSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const script = path.join(root, 'scripts/install.sh');
const skip = process.platform !== 'linux' || process.arch !== 'x64' ? 'needs a linux-x64 host' : false;

function assets(hashOf) {
  const dir = mkdtempSync(path.join(tmpdir(), 'cg-install-'));
  const bin = '#!/bin/sh\necho "comparegraph 0.0.0-test"\n';
  const asset = path.join(dir, 'assets');
  mkdirSync(asset);
  writeFileSync(path.join(asset, 'comparegraph-linux-x64'), bin);
  const sum = hashOf(createHash('sha256').update(bin).digest('hex'));
  writeFileSync(path.join(asset, 'SHA256SUMS'), `${sum}  comparegraph-linux-x64\n${'0'.repeat(64)}  comparegraph-darwin-arm64\n`);
  return { dir, asset, install: path.join(dir, 'bin') };
}

function run(a) {
  return spawnSync('sh', [script], {
    encoding: 'utf8',
    env: { ...process.env, COMPAREGRAPH_BASE_URL: `file://${a.asset}`, COMPAREGRAPH_INSTALL_DIR: a.install, COMPAREGRAPH_VERSION: 'v0.0.0-test' },
  });
}

test('a matching checksum installs the binary and prints its version', { skip }, () => {
  const a = assets((h) => h);
  try {
    const r = run(a);
    assert.equal(r.status, 0, r.stderr);
    const exe = path.join(a.install, 'comparegraph');
    assert.ok(existsSync(exe));
    assert.ok(statSync(exe).mode & 0o100, 'executable');
    assert.match(r.stdout, /installed .*comparegraph \(comparegraph 0\.0\.0-test\)/);
    assert.match(r.stdout, /comparegraph setup --apply {3}\(then: comparegraph doctor\)/);
  } finally { rmSync(a.dir, { recursive: true, force: true }); }
});

test('a checksum mismatch refuses and installs nothing (exit 1)', { skip }, () => {
  const a = assets((h) => 'f'.repeat(32) + h.slice(32));
  try {
    const r = run(a);
    assert.equal(r.status, 1);
    assert.match(r.stderr, /checksum mismatch for comparegraph-linux-x64/);
    assert.match(r.stderr, /nothing was installed/);
    assert.ok(!existsSync(path.join(a.install, 'comparegraph')));
  } finally { rmSync(a.dir, { recursive: true, force: true }); }
});

test('an asset missing from SHA256SUMS refuses (exit 1)', { skip }, () => {
  const a = assets((h) => h);
  writeFileSync(path.join(a.asset, 'SHA256SUMS'), `${'0'.repeat(64)}  comparegraph-darwin-arm64\n`);
  try {
    const r = run(a);
    assert.equal(r.status, 1);
    assert.match(r.stderr, /not listed in SHA256SUMS/);
    assert.ok(!existsSync(path.join(a.install, 'comparegraph')));
  } finally { rmSync(a.dir, { recursive: true, force: true }); }
});
