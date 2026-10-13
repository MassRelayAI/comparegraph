import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, chmodSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const bin = join(dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'comparegraph.js');
const pkgName = `${process.platform}-${process.arch}`;
const isWin = process.platform === 'win32';

function run(args, nodePath) {
  const env = { ...process.env };
  if (nodePath === undefined) delete env.NODE_PATH; else env.NODE_PATH = nodePath;
  return spawnSync(process.execPath, [bin, ...args], { env, encoding: 'utf8' });
}

test('a fake platform package: args round-trip and the exit code propagates', { skip: isWin }, () => {
  const root = mkdtempSync(join(tmpdir(), 'cg-npm-'));
  try {
    const dir = join(root, '@comparegraph', pkgName);
    mkdirSync(join(dir, 'bin'), { recursive: true });
    writeFileSync(join(dir, 'package.json'), JSON.stringify({ name: `@comparegraph/${pkgName}`, version: '9.9.9' }));
    const exe = join(dir, 'bin', 'comparegraph');
    writeFileSync(exe, '#!/bin/sh\necho "fake $@"\n[ "$1" = "--fail" ] && exit 7\nexit 0\n');
    chmodSync(exe, 0o755);

    const ok = run(['--version'], root);
    assert.equal(ok.status, 0);
    assert.equal(ok.stdout.trim(), 'fake --version');

    const bad = run(['--fail'], root);
    assert.equal(bad.status, 7);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('no platform package: the exact error and exit 1', () => {
  const empty = mkdtempSync(join(tmpdir(), 'cg-npm-empty-'));
  try {
    const r = run(['--version'], empty);
    assert.equal(r.status, 1);
    assert.equal(
      r.stderr.trim(),
      `comparegraph: no prebuilt binary for ${pkgName}; build from source: cargo install --locked --git https://github.com/massrelay-ai/comparegraph comparegraph`
    );
    assert.equal(r.stdout, '');
  } finally {
    rmSync(empty, { recursive: true, force: true });
  }
});
