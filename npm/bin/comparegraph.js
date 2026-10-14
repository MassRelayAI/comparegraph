#!/usr/bin/env node
// Runs the prebuilt comparegraph binary shipped by the platform package for this machine.
'use strict';
const { spawnSync } = require('node:child_process');

const PLATFORMS = ['linux-x64', 'linux-arm64', 'darwin-arm64', 'darwin-x64', 'win32-x64'];

function binaryPath() {
  if (process.env.COMPAREGRAPH_BINARY) return process.env.COMPAREGRAPH_BINARY;
  // COMPAREGRAPH_NPM_FORCE_PLATFORM exists for tests: it pretends to run on another platform.
  const key = process.env.COMPAREGRAPH_NPM_FORCE_PLATFORM || `${process.platform}-${process.arch}`;
  const exe = key.startsWith('win32') ? 'comparegraph.exe' : 'comparegraph';
  if (PLATFORMS.includes(key)) {
    try {
      return require.resolve(`@massrelay-ai/comparegraph-${key}/bin/${exe}`);
    } catch (_) {
      /* not installed: fall through */
    }
  }
  process.stderr.write(
    `comparegraph: no prebuilt binary for ${key} (supported: ${PLATFORMS.join(', ')}; was the install run with --no-optional?). ` +
      'Use the install script in the README: https://github.com/massrelay-ai/comparegraph#install\n',
  );
  process.exit(2);
}

const r = spawnSync(binaryPath(), process.argv.slice(2), { stdio: 'inherit' });
if (r.error) {
  process.stderr.write(`comparegraph: ${r.error.message}\n`);
  process.exit(1);
}
if (r.signal) {
  process.stderr.write(`comparegraph: terminated by ${r.signal}\n`);
  process.exit(1);
}
process.exit(r.status === null ? 1 : r.status);
