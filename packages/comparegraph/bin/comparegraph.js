#!/usr/bin/env node
'use strict';
// Thin launcher: finds the prebuilt binary in the matching @comparegraph/<platform>-<arch>
// package and runs it. No fallback to another platform, no postinstall, no dependencies.
const { spawnSync } = require('node:child_process');
const path = require('node:path');

const platform = process.platform;
const arch = process.arch;
const pkg = `@comparegraph/${platform}-${arch}`;
const exe = platform === 'win32' ? 'comparegraph.exe' : 'comparegraph';

let binary;
try {
  binary = path.join(path.dirname(require.resolve(`${pkg}/package.json`)), 'bin', exe);
} catch {
  process.stderr.write(
    `comparegraph: no prebuilt binary for ${platform}-${arch}; ` +
      'build from source: cargo install --locked --git https://github.com/massrelay-ai/comparegraph comparegraph\n'
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });
if (result.error) {
  process.stderr.write(`comparegraph: cannot run ${binary}: ${result.error.message}\n`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
