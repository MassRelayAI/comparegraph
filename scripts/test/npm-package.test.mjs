// npm/build.mjs packages, installed with npm and run: the wrapper finds the platform package, an unsupported platform exits 2.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, copyFileSync, chmodSync, existsSync, readFileSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const binary = path.resolve(root, process.env.COMPAREGRAPH_BIN || 'target/debug/comparegraph');
const version = JSON.parse(readFileSync(path.join(root, 'npm/package.json'), 'utf8')).version;
const isLinuxX64 = process.platform === 'linux' && process.arch === 'x64';
const exe = process.platform === 'win32' ? '.exe' : '';

let work, out, install;

function sh(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { encoding: 'utf8', ...opts });
  assert.equal(r.status, 0, `${cmd} ${args.join(' ')} failed:\n${r.stdout}\n${r.stderr}`);
  return r;
}

before(() => {
  assert.ok(existsSync(binary), `no binary at ${binary}: run \`cargo build -p comparegraph\` or set COMPAREGRAPH_BIN`);
  work = mkdtempSync(path.join(tmpdir(), 'cg-npm-'));
  const bins = path.join(work, 'bins');
  out = path.join(work, 'out');
  mkdirSync(bins);
  copyFileSync(binary, path.join(bins, 'comparegraph-linux-x64'));
  const r = sh('node', [path.join(root, 'npm/build.mjs'), version, '--bins', bins, '--out', out]);
  assert.equal(r.stdout.trim(), '@massrelay-ai/comparegraph-linux-x64\n@massrelay-ai/comparegraph');
});

function pack(dir) {
  const r = sh('npm', ['pack', '--json', '--pack-destination', work], { cwd: dir });
  return { tgz: path.join(work, JSON.parse(r.stdout)[0].filename), files: JSON.parse(r.stdout)[0].files.map((f) => f.path).sort() };
}

test('the wrapper tarball holds only the launcher, package.json and README', () => {
  assert.deepEqual(pack(path.join(out, 'comparegraph')).files, ['README.md', 'bin/comparegraph.js', 'package.json']);
});

test('mcpName equals the registry entry name', () => {
  const pkg = JSON.parse(readFileSync(path.join(out, 'comparegraph/package.json'), 'utf8'));
  const server = JSON.parse(readFileSync(path.join(root, 'server.json'), 'utf8'));
  assert.equal(pkg.mcpName, server.name);
  assert.equal(pkg.version, version);
  for (const [name, v] of Object.entries(pkg.optionalDependencies)) assert.equal(v, version, name);
});

test('installed from tarballs, the bin runs the platform binary', { skip: !isLinuxX64 && 'needs a linux-x64 host' }, () => {
  const wrapper = pack(path.join(out, 'comparegraph')).tgz;
  const platform = pack(path.join(out, 'comparegraph-linux-x64')).tgz;
  install = path.join(work, 'install');
  mkdirSync(install);
  // The other four platform packages are not on any registry: npm must tolerate the missing optional dependencies.
  sh('npm', ['install', '--prefix', install, '--offline', '--no-audit', '--no-fund', '--ignore-scripts', wrapper, platform]);
  const r = sh(path.join(install, 'node_modules/.bin/comparegraph'), ['--version']);
  assert.equal(r.stdout.trim(), `comparegraph ${version}`);
  assert.ok(existsSync(path.join(install, `node_modules/@massrelay-ai/comparegraph-linux-x64/bin/comparegraph${exe}`)));
});

test('an unsupported platform exits 2 with one line naming it', () => {
  const launcher = path.join(out, 'comparegraph/bin/comparegraph.js');
  const r = spawnSync('node', [launcher, '--version'], { encoding: 'utf8', env: { ...process.env, COMPAREGRAPH_NPM_FORCE_PLATFORM: 'freebsd-x64' } });
  assert.equal(r.status, 2);
  assert.equal(r.stdout, '');
  assert.equal(r.stderr.trim().split('\n').length, 1);
  assert.match(r.stderr, /freebsd-x64/);
  assert.match(r.stderr, /install/);
});

test('a supported platform whose package is absent exits 2 too', () => {
  const launcher = path.join(out, 'comparegraph/bin/comparegraph.js');
  const r = spawnSync('node', [launcher, '--version'], { encoding: 'utf8', env: { ...process.env, COMPAREGRAPH_NPM_FORCE_PLATFORM: 'darwin-arm64' } });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /darwin-arm64/);
});

test('COMPAREGRAPH_BINARY overrides the lookup and the exit status passes through', () => {
  const launcher = path.join(out, 'comparegraph/bin/comparegraph.js');
  const ok = spawnSync('node', [launcher, '--version'], { encoding: 'utf8', env: { ...process.env, COMPAREGRAPH_BINARY: binary } });
  assert.equal(ok.status, 0);
  assert.equal(ok.stdout.trim(), `comparegraph ${version}`);
  const bad = spawnSync('node', [launcher, '--no-such-flag'], { encoding: 'utf8', env: { ...process.env, COMPAREGRAPH_BINARY: binary } });
  assert.notEqual(bad.status, 0);
  assert.notEqual(bad.status, null);
});
