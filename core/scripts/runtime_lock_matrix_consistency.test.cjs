const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const assert = require('node:assert/strict');

const { validateLockMatrixConsistency } = require('./runtime_lock_matrix_consistency.cjs');

const writeJson = (filePath, value) => {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
};

const baseLock = () => ({
  version: 2,
  required: {
    targets: {
      provider: ['host/host', 'linux/aarch64', 'linux/x86_64'],
      runtime: ['host/host', 'linux/aarch64', 'linux/x86_64'],
      image: ['linux/aarch64', 'linux/x86_64'],
    },
    provider_ids: ['codex'],
    runtime_ids: ['node'],
    image_ids: ['ctx-harness'],
  },
  components: [
    { kind: 'provider', id: 'codex', os: 'host', arch: 'host' },
    { kind: 'provider', id: 'codex', os: 'linux', arch: 'aarch64' },
    { kind: 'provider', id: 'codex', os: 'linux', arch: 'x86_64' },
    { kind: 'runtime', id: 'node', os: 'host', arch: 'host' },
    { kind: 'runtime', id: 'node', os: 'linux', arch: 'aarch64' },
    { kind: 'runtime', id: 'node', os: 'linux', arch: 'x86_64' },
    { kind: 'image', id: 'ctx-harness', os: 'linux', arch: 'aarch64' },
    { kind: 'image', id: 'ctx-harness', os: 'linux', arch: 'x86_64' },
  ],
});

const baseMatrix = () => ({
  providers: [
    {
      id: 'codex',
      managed_install: {
        targets: {
          'linux-x86_64': { method: 'tarball' },
        },
      },
    },
  ],
});

test('lock and matrix consistency passes for valid contract', () => {
  const result = validateLockMatrixConsistency({
    lock: baseLock(),
    matrix: baseMatrix(),
    hostOs: 'macos',
    hostArch: 'aarch64',
  });

  assert.equal(result.ok, true);
  assert.equal(result.errors.length, 0);
});

test('fails when required provider target is missing in runtime lock components', () => {
  const lock = baseLock();
  lock.components = lock.components.filter(
    (component) => !(component.kind === 'provider' && component.id === 'codex' && component.os === 'linux' && component.arch === 'aarch64'),
  );

  const result = validateLockMatrixConsistency({
    lock,
    matrix: baseMatrix(),
    hostOs: 'macos',
    hostArch: 'aarch64',
  });

  assert.equal(result.ok, false);
  assert.match(result.errors.join('\n'), /missing provider component codex linux\/aarch64/);
});

test('fails when matrix-managed target lacks matching runtime lock component', () => {
  const matrix = baseMatrix();
  matrix.providers[0].managed_install.targets['linux-aarch64'] = { method: 'tarball' };

  const lock = baseLock();
  lock.components = lock.components.filter(
    (component) => !(component.kind === 'provider' && component.id === 'codex' && component.os === 'linux' && component.arch === 'aarch64'),
  );

  const result = validateLockMatrixConsistency({
    lock,
    matrix,
    hostOs: 'macos',
    hostArch: 'aarch64',
  });

  assert.equal(result.ok, false);
  assert.match(
    result.errors.join('\n'),
    /provider_matrix target codex linux\/aarch64 missing corresponding runtime_lock provider component/,
  );
});

test('cli path can evaluate fixture files', () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lock-matrix-consistency-'));
  const lockPath = path.join(tempDir, 'lock.json');
  const matrixPath = path.join(tempDir, 'matrix.json');
  writeJson(lockPath, baseLock());
  writeJson(matrixPath, baseMatrix());

  const lock = JSON.parse(fs.readFileSync(lockPath, 'utf8'));
  const matrix = JSON.parse(fs.readFileSync(matrixPath, 'utf8'));
  const result = validateLockMatrixConsistency({
    lock,
    matrix,
    hostOs: 'macos',
    hostArch: 'aarch64',
  });

  assert.equal(result.ok, true);
});
