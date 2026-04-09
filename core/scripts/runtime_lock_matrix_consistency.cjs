#!/usr/bin/env node

const fs = require('node:fs');
const path = require('node:path');

const coreRoot = path.resolve(__dirname, '..');

const defaultLockPath = path.join(coreRoot, 'apps', 'desktop', 'src-tauri', 'bundles', 'runtime_lock.v2.json');
const defaultMatrixPath = path.join(
  coreRoot,
  'crates',
  'ctx-provider-accounts',
  'src',
  'provider_matrix.json',
);

const normalizeOs = (value) => {
  if (value === 'darwin') return 'macos';
  if (value === 'win32') return 'windows';
  return value;
};

const normalizeArch = (value) => {
  if (value === 'arm64') return 'aarch64';
  if (value === 'x64') return 'x86_64';
  return value;
};

const normalizeToken = (value, hostValue) => {
  const raw = String(value || '').trim();
  if (!raw) return raw;
  if (raw === 'host') return hostValue;
  return raw;
};

const parseArgs = (argv) => {
  const options = {
    lockPath: defaultLockPath,
    matrixPath: defaultMatrixPath,
    hostOs: normalizeOs(process.platform),
    hostArch: normalizeArch(process.arch),
  };

  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (flag === '--lock') {
      if (!value) throw new Error('missing value for --lock');
      options.lockPath = path.resolve(value);
      i += 1;
      continue;
    }
    if (flag === '--matrix') {
      if (!value) throw new Error('missing value for --matrix');
      options.matrixPath = path.resolve(value);
      i += 1;
      continue;
    }
    if (flag === '--host-os') {
      if (!value) throw new Error('missing value for --host-os');
      options.hostOs = String(value).trim();
      i += 1;
      continue;
    }
    if (flag === '--host-arch') {
      if (!value) throw new Error('missing value for --host-arch');
      options.hostArch = String(value).trim();
      i += 1;
      continue;
    }
    throw new Error(`unknown argument: ${flag}`);
  }

  if (!options.hostOs) throw new Error('host os resolved empty');
  if (!options.hostArch) throw new Error('host arch resolved empty');
  return options;
};

const readJson = (filePath, label) => {
  try {
    return JSON.parse(fs.readFileSync(filePath, 'utf8'));
  } catch (error) {
    throw new Error(`failed to read ${label} at ${filePath}: ${error?.message ?? error}`);
  }
};

const parseTarget = ({ raw, hostOs, hostArch, errors, label }) => {
  const [osRaw, archRaw] = String(raw || '').split('/');
  if (!osRaw || !archRaw) {
    errors.push(`invalid ${label} target '${raw}' (expected <os>/<arch>)`);
    return null;
  }
  const os = normalizeToken(osRaw.trim(), hostOs);
  const arch = normalizeToken(archRaw.trim(), hostArch);
  if (!os || !arch) {
    errors.push(`invalid ${label} target '${raw}' (resolved empty os/arch)`);
    return null;
  }
  return { os, arch };
};

const parseRequiredTargets = ({ lock, kind, hostOs, hostArch, errors, allowEmpty = false }) => {
  const rawTargets = lock?.required?.targets?.[kind];
  if (!Array.isArray(rawTargets)) {
    if (allowEmpty) return [];
    errors.push(`runtime lock required.targets.${kind} must be a non-empty array`);
    return [];
  }
  if (rawTargets.length === 0) {
    if (allowEmpty) return [];
    errors.push(`runtime lock required.targets.${kind} must be a non-empty array`);
    return [];
  }
  const out = [];
  const seen = new Set();
  for (const raw of rawTargets) {
    const parsed = parseTarget({ raw, hostOs, hostArch, errors, label: `required.targets.${kind}` });
    if (!parsed) continue;
    const key = `${parsed.os}/${parsed.arch}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(parsed);
  }
  if (!allowEmpty && out.length === 0) {
    errors.push(`runtime lock required.targets.${kind} must contain at least one valid target`);
  }
  return out;
};

const parseMatrixTargetKey = (rawKey) => {
  const raw = String(rawKey || '').trim();
  if (!raw) return null;
  const normalized = raw.includes('/') ? raw : raw.replace('-', '/');
  const [osRaw, archRaw] = normalized.split('/');
  const os = normalizeOs(String(osRaw || '').trim());
  const arch = normalizeArch(String(archRaw || '').trim());
  if (!os || !arch) return null;
  return { os, arch };
};

const validateLockMatrixConsistency = ({ lock, matrix, hostOs, hostArch }) => {
  const errors = [];

  const providers = Array.isArray(matrix?.providers) ? matrix.providers : [];
  const providerIds = new Set(providers.map((entry) => String(entry?.id || '').trim()).filter(Boolean));

  const components = Array.isArray(lock?.components) ? lock.components : [];
  const requiredProviderIds = Array.isArray(lock?.required?.provider_ids)
    ? [...new Set(lock.required.provider_ids.map((id) => String(id || '').trim()).filter(Boolean))]
    : [];
  const requiredRuntimeIds = Array.isArray(lock?.required?.runtime_ids)
    ? [...new Set(lock.required.runtime_ids.map((id) => String(id || '').trim()).filter(Boolean))]
    : [];
  const requiredImageIds = Array.isArray(lock?.required?.image_ids)
    ? [...new Set(lock.required.image_ids.map((id) => String(id || '').trim()).filter(Boolean))]
    : [];

  const providerTargets = parseRequiredTargets({
    lock,
    kind: 'provider',
    hostOs,
    hostArch,
    errors,
    allowEmpty: requiredProviderIds.length === 0,
  });
  const runtimeTargets = parseRequiredTargets({
    lock,
    kind: 'runtime',
    hostOs,
    hostArch,
    errors,
    allowEmpty: requiredRuntimeIds.length === 0,
  });
  const imageTargets = parseRequiredTargets({
    lock,
    kind: 'image',
    hostOs,
    hostArch,
    errors,
    allowEmpty: requiredImageIds.length === 0,
  });
  const requiredProviderTargetKeys = new Set(providerTargets.map((target) => `${target.os}/${target.arch}`));

  const hasComponent = ({ kind, id, os, arch }) =>
    components.some(
      (component) =>
        component?.kind === kind &&
        String(component?.id || '').trim() === id &&
        normalizeToken(component?.os, hostOs) === os &&
        normalizeToken(component?.arch, hostArch) === arch,
    );

  for (const providerId of requiredProviderIds) {
    if (!providerIds.has(providerId)) {
      errors.push(`runtime lock required provider '${providerId}' is missing from provider_matrix`);
      continue;
    }
    for (const target of providerTargets) {
      if (!hasComponent({ kind: 'provider', id: providerId, os: target.os, arch: target.arch })) {
        errors.push(`runtime lock missing provider component ${providerId} ${target.os}/${target.arch}`);
      }
    }
  }

  for (const runtimeId of requiredRuntimeIds) {
    for (const target of runtimeTargets) {
      if (!hasComponent({ kind: 'runtime', id: runtimeId, os: target.os, arch: target.arch })) {
        errors.push(`runtime lock missing runtime component ${runtimeId} ${target.os}/${target.arch}`);
      }
    }
  }

  for (const imageId of requiredImageIds) {
    for (const target of imageTargets) {
      if (!hasComponent({ kind: 'image', id: imageId, os: target.os, arch: target.arch })) {
        errors.push(`runtime lock missing image component ${imageId} ${target.os}/${target.arch}`);
      }
    }
  }

  for (const provider of providers) {
    const providerId = String(provider?.id || '').trim();
    if (!providerId || !requiredProviderIds.includes(providerId)) continue;
    const targets = provider?.managed_install?.targets;
    if (!targets || typeof targets !== 'object') continue;

    for (const key of Object.keys(targets)) {
      const parsed = parseMatrixTargetKey(key);
      if (!parsed) {
        errors.push(`provider_matrix managed target '${key}' for ${providerId} is invalid`);
        continue;
      }
      if (!requiredProviderTargetKeys.has(`${parsed.os}/${parsed.arch}`)) continue;
      if (!hasComponent({ kind: 'provider', id: providerId, os: parsed.os, arch: parsed.arch })) {
        errors.push(
          `provider_matrix target ${providerId} ${parsed.os}/${parsed.arch} missing corresponding runtime_lock provider component`,
        );
      }
    }
  }

  return {
    ok: errors.length === 0,
    errors,
  };
};

const main = () => {
  const options = parseArgs(process.argv.slice(2));
  const lock = readJson(options.lockPath, 'runtime lock');
  const matrix = readJson(options.matrixPath, 'provider matrix');
  const result = validateLockMatrixConsistency({
    lock,
    matrix,
    hostOs: options.hostOs,
    hostArch: options.hostArch,
  });

  if (!result.ok) {
    for (const error of result.errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }

  console.log(`runtime_lock_matrix_consistency: OK (lock=${path.relative(coreRoot, options.lockPath)})`);
};

if (require.main === module) {
  main();
}

module.exports = {
  parseArgs,
  validateLockMatrixConsistency,
};
