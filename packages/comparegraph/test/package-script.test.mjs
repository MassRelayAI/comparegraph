import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, chmodSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const script = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..', 'scripts', 'package-npm.sh');
const skip = process.platform === 'win32';

// A fake `uname` first on PATH; the guards run before any cargo step, so nothing is built.
function run(args, os, machine) {
  const dir = mkdtempSync(join(tmpdir(), 'cg-uname-'));
  try {
    const u = join(dir, 'uname');
    writeFileSync(u, `#!/bin/sh\ncase "$1" in -s) echo ${os};; -m) echo ${machine};; esac\n`);
    chmodSync(u, 0o755);
    return spawnSync('bash', [script, ...args], { env: { ...process.env, PATH: `${dir}:${process.env.PATH}` }, encoding: 'utf8' });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('a wrong host is refused with exit 3', { skip }, () => {
  const r = run(['0.1.0', 'linux-x64'], 'Darwin', 'arm64');
  assert.equal(r.status, 3);
  assert.match(r.stderr, /package-npm: host darwin-arm64 cannot build linux-x64; run this on a linux-x64 runner/);
});

test('a version differing from Cargo.toml is fatal (exit 4)', { skip }, () => {
  const r = run(['9.9.9', 'linux-x64'], 'Linux', 'x86_64');
  assert.equal(r.status, 4);
  assert.match(r.stderr, /differs from Cargo\.toml/);
});
