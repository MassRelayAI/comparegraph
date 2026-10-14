#!/usr/bin/env node
// node npm/build.mjs <version> --bins <dir> --out <dir>
// Writes <out>/comparegraph (the wrapper) and <out>/comparegraph-<platform> for each binary found in <dir>.
// Deterministic and offline.
import { mkdirSync, readFileSync, writeFileSync, copyFileSync, chmodSync, existsSync, rmSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const SCOPE = '@massrelay-ai';
const PLATFORMS = {
  'linux-x64': { os: 'linux', cpu: 'x64' },
  'linux-arm64': { os: 'linux', cpu: 'arm64' },
  'darwin-arm64': { os: 'darwin', cpu: 'arm64' },
  'darwin-x64': { os: 'darwin', cpu: 'x64' },
  'win32-x64': { os: 'win32', cpu: 'x64' },
};

function fail(msg) {
  process.stderr.write(`npm/build.mjs: ${msg}\nusage: node npm/build.mjs <version> --bins <dir> --out <dir>\n`);
  process.exit(1);
}

const args = process.argv.slice(2);
const version = args[0];
const opt = (name) => {
  const i = args.indexOf(name);
  return i > 0 ? args[i + 1] : undefined;
};
const bins = opt('--bins');
const out = opt('--out');
if (!version || version.startsWith('--') || !/^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$/.test(version)) fail('a semver version is required');
if (!bins || !out) fail('--bins and --out are required');

const template = JSON.parse(readFileSync(path.join(here, 'package.json'), 'utf8'));
const built = [];

for (const [platform, { os, cpu }] of Object.entries(PLATFORMS)) {
  const exe = os === 'win32' ? 'comparegraph.exe' : 'comparegraph';
  const src = path.join(bins, `comparegraph-${platform}${os === 'win32' ? '.exe' : ''}`);
  if (!existsSync(src)) continue;
  const dir = path.join(out, `comparegraph-${platform}`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(path.join(dir, 'bin'), { recursive: true });
  copyFileSync(src, path.join(dir, 'bin', exe));
  chmodSync(path.join(dir, 'bin', exe), 0o755);
  const pkg = {
    name: `${SCOPE}/comparegraph-${platform}`,
    version,
    description: `The comparegraph binary for ${platform}. Install ${SCOPE}/comparegraph, not this package.`,
    license: template.license,
    repository: template.repository,
    homepage: template.homepage,
    bugs: template.bugs,
    os: [os],
    cpu: [cpu],
    files: ['bin'],
    publishConfig: { access: 'public' },
  };
  writeFileSync(path.join(dir, 'package.json'), JSON.stringify(pkg, null, 2) + '\n');
  built.push(pkg.name);
}

const wrapperDir = path.join(out, 'comparegraph');
rmSync(wrapperDir, { recursive: true, force: true });
mkdirSync(path.join(wrapperDir, 'bin'), { recursive: true });
copyFileSync(path.join(here, 'bin', 'comparegraph.js'), path.join(wrapperDir, 'bin', 'comparegraph.js'));
chmodSync(path.join(wrapperDir, 'bin', 'comparegraph.js'), 0o755);
copyFileSync(path.join(here, 'README.md'), path.join(wrapperDir, 'README.md'));
const wrapper = { ...template, version, optionalDependencies: {} };
for (const platform of Object.keys(PLATFORMS)) wrapper.optionalDependencies[`${SCOPE}/comparegraph-${platform}`] = version;
writeFileSync(path.join(wrapperDir, 'package.json'), JSON.stringify(wrapper, null, 2) + '\n');
built.push(wrapper.name);

process.stdout.write(built.join('\n') + '\n');
