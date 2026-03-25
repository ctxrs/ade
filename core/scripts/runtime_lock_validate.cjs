#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const bundlesDir = path.join(coreRoot, "apps", "desktop", "src-tauri", "bundles");
const defaultLockPathV2 = path.join(bundlesDir, "runtime_lock.v2.json");
const defaultLockPathV1 = path.join(bundlesDir, "runtime_lock.v1.json");
const defaultManifestPath = path.join(bundlesDir, "manifest.json");
const defaultOverridesPath = path.join(coreRoot, "..", ".ctx", "local", "runtime_overrides.json");

const PROFILE_VALUES = new Set(["parity", "override", "source-all"]);
const SOURCE_TYPES = new Set(["ci", "vendor", "local"]);
const COMPONENT_KINDS = new Set(["provider", "runtime", "image", "machine_cache"]);

const isNonEmptyString = (value) => typeof value === "string" && value.trim().length > 0;

const readJson = (filePath, errors, label) => {
  try {
    const raw = fs.readFileSync(filePath, "utf8");
    return JSON.parse(raw);
  } catch (error) {
    errors.push(`failed to read ${label} at ${filePath}: ${error?.message ?? String(error)}`);
    return null;
  }
};

const normalizeManifestOs = (platform) => {
  if (platform === "darwin") return "macos";
  if (platform === "win32") return "windows";
  return platform;
};

const normalizeManifestArch = (arch) => {
  if (arch === "arm64") return "aarch64";
  if (arch === "x64") return "x86_64";
  return arch;
};

const normalizeTargetToken = (value, hostValue) => {
  const raw = String(value ?? "").trim();
  if (!raw) return raw;
  if (raw === "host") return hostValue;
  return raw;
};

const findManifestEntry = (entries, id, os, arch) =>
  entries.find((entry) => entry && entry.id === id && entry.os === os && entry.arch === arch);

const resolvePathFromManifestValue = (bundlesDirPath, rawValue) => {
  if (!isNonEmptyString(rawValue)) return null;
  const value = rawValue.trim();
  const candidate = path.isAbsolute(value) ? value : path.join(bundlesDirPath, value);
  return candidate;
};

const expectFilePath = (filePath, errors, label) => {
  if (!filePath) {
    errors.push(`${label} is missing`);
    return;
  }
  if (!fs.existsSync(filePath)) {
    errors.push(`${label} missing file: ${filePath}`);
    return;
  }
  if (!fs.statSync(filePath).isFile()) {
    errors.push(`${label} is not a file: ${filePath}`);
  }
};

const expectDirPath = (dirPath, errors, label) => {
  if (!dirPath) {
    errors.push(`${label} is missing`);
    return;
  }
  if (!fs.existsSync(dirPath)) {
    errors.push(`${label} missing directory: ${dirPath}`);
    return;
  }
  if (!fs.statSync(dirPath).isDirectory()) {
    errors.push(`${label} is not a directory: ${dirPath}`);
  }
};

const sha256File = (filePath) => {
  const data = fs.readFileSync(filePath);
  return crypto.createHash("sha256").update(data).digest("hex");
};

const parseProfile = (raw, errors) => {
  const value = String(raw || "").trim() || "parity";
  if (!PROFILE_VALUES.has(value)) {
    errors.push(`invalid runtime profile '${value}' (expected parity|override|source-all)`);
    return "parity";
  }
  return value;
};

const componentKey = (component, hostOs, hostArch) => {
  const os = normalizeTargetToken(component.os, hostOs);
  const arch = normalizeTargetToken(component.arch, hostArch);
  const variant = isNonEmptyString(component.variant) ? component.variant.trim() : "default";
  return `${component.kind}:${component.id}:${os}:${arch}:${variant}`;
};

const cloneJson = (value) => JSON.parse(JSON.stringify(value));

const validateRequiredArray = (value, label, errors, options = {}) => {
  const allowEmpty = options.allowEmpty === true;
  if (!Array.isArray(value)) {
    errors.push(`${label} must be an array`);
    return [];
  }
  if (!allowEmpty && value.length === 0) {
    errors.push(`${label} must be a non-empty array`);
    return [];
  }
  const out = [];
  for (const entry of value) {
    if (!isNonEmptyString(entry)) {
      errors.push(`${label} entries must be non-empty strings`);
      continue;
    }
    out.push(entry.trim());
  }
  return out;
};

const defaultProviderTargets = (hostOs, hostArch) => {
  if (hostOs === "macos" && hostArch === "aarch64") {
    return [
      { os: "macos", arch: "aarch64", label: "host macos/aarch64" },
      { os: "linux", arch: "aarch64", label: "container linux/aarch64" },
      { os: "linux", arch: "x86_64", label: "container linux/x86_64" },
    ];
  }
  return [
    { os: hostOs, arch: hostArch, label: `host ${hostOs}/${hostArch}` },
    { os: "linux", arch: hostArch, label: `container linux/${hostArch}` },
  ];
};

const defaultRuntimeTargets = (hostOs, hostArch) => {
  if (hostOs === "macos" && hostArch === "aarch64") {
    return [
      { os: "macos", arch: "aarch64", label: "host macos/aarch64" },
      { os: "linux", arch: "aarch64", label: "container linux/aarch64" },
      { os: "linux", arch: "x86_64", label: "container linux/x86_64" },
    ];
  }
  return [
    { os: hostOs, arch: hostArch, label: `host ${hostOs}/${hostArch}` },
    { os: "linux", arch: hostArch, label: `container linux/${hostArch}` },
  ];
};

const defaultImageTargets = (hostOs, hostArch) => {
  if (hostOs === "macos" && hostArch === "aarch64") {
    return [
      { os: "linux", arch: "aarch64", label: "linux/aarch64" },
      { os: "linux", arch: "x86_64", label: "linux/x86_64" },
    ];
  }
  return [{ os: "linux", arch: hostArch, label: `linux/${hostArch}` }];
};

const defaultMachineCacheTargets = (hostOs, hostArch) => {
  if (hostOs !== "macos") {
    return [];
  }
  return [{ os: "macos", arch: hostArch, label: `macos/${hostArch}` }];
};

const parseRequiredTargetEntry = (value, hostOs, hostArch, kind, errors) => {
  if (!isNonEmptyString(value)) {
    errors.push(`runtime lock required.targets.${kind} entries must be non-empty strings`);
    return null;
  }
  const [rawOs, rawArch] = String(value).split("/");
  if (!isNonEmptyString(rawOs) || !isNonEmptyString(rawArch)) {
    errors.push(`runtime lock required.targets.${kind} entry must be '<os>/<arch>' (got ${JSON.stringify(value)})`);
    return null;
  }
  const os = normalizeTargetToken(rawOs.trim(), hostOs);
  const arch = normalizeTargetToken(rawArch.trim(), hostArch);
  if (!isNonEmptyString(os) || !isNonEmptyString(arch)) {
    errors.push(`runtime lock required.targets.${kind} entry resolves to invalid os/arch: ${JSON.stringify(value)}`);
    return null;
  }
  return {
    os,
    arch,
    label: `${os}/${arch}`,
  };
};

const resolveRequiredTargets = ({ lock, hostOs, hostArch, kind, errors, allowEmpty = false }) => {
  const configuredTargets = lock?.required?.targets?.[kind];
  if (!Array.isArray(configuredTargets)) {
    if (allowEmpty) return [];
    if (kind === "provider") return defaultProviderTargets(hostOs, hostArch);
    if (kind === "runtime") return defaultRuntimeTargets(hostOs, hostArch);
    if (kind === "image") return defaultImageTargets(hostOs, hostArch);
    if (kind === "machine_cache") return defaultMachineCacheTargets(hostOs, hostArch);
    return [];
  }
  if (configuredTargets.length === 0) {
    if (!allowEmpty) {
      errors.push(`runtime lock required.targets.${kind} must contain at least one valid target`);
    }
    return [];
  }

  const out = [];
  const seen = new Set();
  for (const entry of configuredTargets) {
    const parsed = parseRequiredTargetEntry(entry, hostOs, hostArch, kind, errors);
    if (!parsed) continue;
    const dedupeKey = `${parsed.os}/${parsed.arch}`;
    if (seen.has(dedupeKey)) continue;
    seen.add(dedupeKey);
    out.push(parsed);
  }
  if (out.length === 0 && !allowEmpty) {
    errors.push(`runtime lock required.targets.${kind} must contain at least one valid target`);
  }
  return out;
};

const findComponent = ({ components, kind, id, os, arch, hostOs, hostArch }) =>
  components.find(
    (component) =>
      component.kind === kind &&
      component.id === id &&
      normalizeTargetToken(component.os, hostOs) === os &&
      normalizeTargetToken(component.arch, hostArch) === arch,
  );

const validateProfileSources = ({ lock, profile, requiredComponents, errors }) => {
  const profileCfg = lock.profiles?.[profile];
  if (!profileCfg || typeof profileCfg !== "object") {
    errors.push(`runtime lock missing profiles.${profile}`);
    return new Set();
  }
  const allowedRaw = Array.isArray(profileCfg.allowed_source_types)
    ? profileCfg.allowed_source_types
    : [];
  if (allowedRaw.length === 0) {
    errors.push(`runtime lock profiles.${profile}.allowed_source_types must be non-empty`);
    return new Set();
  }
  const allowed = new Set();
  for (const entry of allowedRaw) {
    if (!isNonEmptyString(entry)) {
      errors.push(`runtime lock profiles.${profile}.allowed_source_types entries must be non-empty strings`);
      continue;
    }
    const sourceType = entry.trim();
    if (!SOURCE_TYPES.has(sourceType)) {
      errors.push(`runtime lock profiles.${profile}.allowed_source_types contains unsupported source '${sourceType}'`);
      continue;
    }
    allowed.add(sourceType);
  }

  for (const component of requiredComponents) {
    const sources = Array.isArray(component.sources) ? component.sources : [];
    const hasAllowedSource = sources.some((source) => isNonEmptyString(source?.source_type) && allowed.has(source.source_type.trim()));
    if (!hasAllowedSource) {
      errors.push(
        `component ${component.kind}/${component.id} ${component.os}/${component.arch} missing allowed source for profile '${profile}' (allowed=${[...allowed].join(",")})`,
      );
    }
  }
  return allowed;
};

const componentHasManagedDownloadSource = ({ component, allowedSourceTypes }) => {
  if (!component || typeof component !== "object") return false;
  const sources = Array.isArray(component.sources) ? component.sources : [];
  for (const source of sources) {
    const sourceType = String(source?.source_type || "").trim();
    if (!sourceType || sourceType === "local") continue;
    if (allowedSourceTypes instanceof Set && allowedSourceTypes.size > 0 && !allowedSourceTypes.has(sourceType)) {
      continue;
    }
    const uri = String(source?.uri || "").trim();
    const sha256 = String(source?.sha256 || "").trim();
    if (uri && sha256) return true;
  }
  return false;
};

const applyOverridesToManifest = ({ manifest, overrides, hostOs, hostArch, errors }) => {
  if (!Array.isArray(overrides) || overrides.length === 0) {
    return cloneJson(manifest);
  }
  const next = cloneJson(manifest);

  for (const override of overrides) {
    const kind = String(override.kind || "").trim();
    const id = String(override.id || "").trim();
    const os = normalizeTargetToken(override.os, hostOs);
    const arch = normalizeTargetToken(override.arch, hostArch);
    const absPath = String(override.path || "").trim();

    if (kind === "provider") {
      const entry = (next.providers || []).find((candidate) => candidate.id === id && candidate.os === os && candidate.arch === arch);
      if (!entry) {
        errors.push(`override target missing provider entry in manifest: ${id} ${os}/${arch}`);
        continue;
      }
      entry.command = absPath;
      if (Array.isArray(override.args)) entry.args = [...override.args];
      continue;
    }

    if (kind === "runtime") {
      const entry = (next.runtimes || []).find((candidate) => candidate.id === id && candidate.os === os && candidate.arch === arch);
      if (!entry) {
        errors.push(`override target missing runtime entry in manifest: ${id} ${os}/${arch}`);
        continue;
      }
      const stats = fs.statSync(absPath);
      if (stats.isDirectory()) {
        entry.root = absPath;
        if (isNonEmptyString(override.bin)) entry.bin = override.bin.trim();
      } else {
        entry.root = path.dirname(absPath);
        entry.bin = path.basename(absPath);
      }
      continue;
    }

    if (kind === "image") {
      const entry = (next.images || []).find((candidate) => candidate.id === id && candidate.os === os && candidate.arch === arch);
      if (!entry) {
        errors.push(`override target missing image entry in manifest: ${id} ${os}/${arch}`);
        continue;
      }
      entry.tar = absPath;
      continue;
    }

    if (kind === "machine_cache") {
      continue;
    }
  }

  return next;
};

const loadOverrides = ({ profile, overridesPath, errors }) => {
  if (profile !== "override") return { overrides: [], appliedOverrides: [] };

  if (!overridesPath || !fs.existsSync(overridesPath)) {
    return { overrides: [], appliedOverrides: [] };
  }

  const parsed = readJson(overridesPath, errors, "runtime overrides");
  if (!parsed) return { overrides: [], appliedOverrides: [] };
  if (parsed.version !== 1) {
    errors.push(`runtime overrides version must be 1 (got ${JSON.stringify(parsed.version)})`);
  }

  const overrides = Array.isArray(parsed.overrides) ? parsed.overrides : [];
  const normalized = [];
  for (const override of overrides) {
    const kind = String(override?.kind || "").trim();
    const id = String(override?.id || "").trim();
    const os = String(override?.os || "").trim();
    const arch = String(override?.arch || "").trim();
    const variant = isNonEmptyString(override?.variant) ? override.variant.trim() : "default";
    const absPath = String(override?.path || "").trim();
    if (!COMPONENT_KINDS.has(kind)) {
      errors.push(`override kind must be provider|runtime|image|machine_cache (got ${JSON.stringify(override?.kind)})`);
      continue;
    }
    if (!id || !os || !arch || !absPath) {
      errors.push(`override entries must include kind,id,os,arch,path`);
      continue;
    }
    if (!path.isAbsolute(absPath)) {
      errors.push(`override path must be absolute: ${absPath}`);
      continue;
    }
    if (!fs.existsSync(absPath)) {
      errors.push(`override path missing: ${absPath}`);
      continue;
    }
    if (kind === "provider" || kind === "image" || kind === "machine_cache") {
      if (!fs.statSync(absPath).isFile()) {
        errors.push(`override path must be a file for ${kind}: ${absPath}`);
        continue;
      }
    }
    if (kind === "runtime") {
      const stats = fs.statSync(absPath);
      if (!stats.isFile() && !stats.isDirectory()) {
        errors.push(`override runtime path must be a file or directory: ${absPath}`);
        continue;
      }
    }
    if (isNonEmptyString(override?.sha256) && fs.statSync(absPath).isFile()) {
      const actual = sha256File(absPath);
      if (actual !== override.sha256.trim()) {
        errors.push(`override sha256 mismatch for ${kind}/${id} at ${absPath}`);
        continue;
      }
    }
    normalized.push({
      kind,
      id,
      os,
      arch,
      variant,
      path: absPath,
      bin: override?.bin,
      args: Array.isArray(override?.args) ? override.args.filter((value) => isNonEmptyString(value)).map((value) => value.trim()) : undefined,
    });
  }

  return { overrides: normalized, appliedOverrides: normalized.map((override) => ({ ...override })) };
};

const validateManifestEntries = ({
  lock,
  manifest,
  manifestPath,
  hostOs,
  hostArch,
  providerTargets,
  runtimeTargets,
  imageTargets,
  requiredComponentMap = null,
  allowedSourceTypes = new Set(),
  allowEmptyRequired = {
    provider: false,
    runtime: false,
    image: true,
  },
  errors,
}) => {
  if (manifest.version !== 1) {
    errors.push(`bundle manifest version must be 1 (got ${JSON.stringify(manifest.version)})`);
  }

  if (!Array.isArray(manifest.providers)) errors.push("manifest.providers must be an array");
  if (!Array.isArray(manifest.runtimes)) errors.push("manifest.runtimes must be an array");
  if (!Array.isArray(manifest.images)) errors.push("manifest.images must be an array");

  const providerIds = validateRequiredArray(lock?.required?.provider_ids, "runtime lock required.provider_ids", errors, {
    allowEmpty: allowEmptyRequired.provider === true,
  });
  const runtimeIds = validateRequiredArray(lock?.required?.runtime_ids, "runtime lock required.runtime_ids", errors, {
    allowEmpty: allowEmptyRequired.runtime === true,
  });
  const imageIds = validateRequiredArray(
    lock?.required?.image_ids,
    "runtime lock required.image_ids",
    errors,
    { allowEmpty: allowEmptyRequired.image === true },
  );

  const bundlesRoot = path.dirname(manifestPath);

  for (const providerId of providerIds) {
    for (const target of providerTargets) {
      const entry = findManifestEntry(manifest.providers || [], providerId, target.os, target.arch);
      if (!entry) {
        errors.push(`missing provider bundle entry for ${providerId} (${target.label})`);
        continue;
      }
      const commandPath = resolvePathFromManifestValue(bundlesRoot, entry.command);
      expectFilePath(commandPath, errors, `provider command ${providerId} (${target.label})`);
    }
  }

  for (const runtimeId of runtimeIds) {
    for (const target of runtimeTargets) {
      const runtimeComponentKey = componentKey(
        {
          kind: "runtime",
          id: runtimeId,
          os: target.os,
          arch: target.arch,
          variant: "default",
        },
        hostOs,
        hostArch,
      );
      const runtimeComponent =
        requiredComponentMap instanceof Map ? requiredComponentMap.get(runtimeComponentKey) : null;
      const managedRuntimeAvailable = componentHasManagedDownloadSource({
        component: runtimeComponent,
        allowedSourceTypes,
      });
      const entry = findManifestEntry(manifest.runtimes || [], runtimeId, target.os, target.arch);
      if (!entry) {
        if (!managedRuntimeAvailable) {
          errors.push(`missing runtime bundle entry for ${runtimeId} (${target.label})`);
        }
        continue;
      }
      const rootPath = resolvePathFromManifestValue(bundlesRoot, entry.root);
      expectDirPath(rootPath, errors, `runtime root ${runtimeId} (${target.label})`);
      if (rootPath && isNonEmptyString(entry.bin)) {
        expectFilePath(path.join(rootPath, entry.bin.trim()), errors, `runtime bin ${runtimeId} (${target.label})`);
      } else {
        errors.push(`runtime bin ${runtimeId} (${target.label}) is missing`);
      }
    }
  }

  for (const imageId of imageIds) {
    for (const target of imageTargets) {
      const imageComponentKey = componentKey(
        {
          kind: "image",
          id: imageId,
          os: target.os,
          arch: target.arch,
          variant: "default",
        },
        hostOs,
        hostArch,
      );
      const imageComponent =
        requiredComponentMap instanceof Map ? requiredComponentMap.get(imageComponentKey) : null;
      const managedImageAvailable = componentHasManagedDownloadSource({
        component: imageComponent,
        allowedSourceTypes,
      });
      const entry = findManifestEntry(manifest.images || [], imageId, target.os, target.arch);
      if (!entry) {
        if (!managedImageAvailable) {
          errors.push(`missing image bundle entry for ${imageId} (${target.label})`);
        }
        continue;
      }
      const tarPath = resolvePathFromManifestValue(bundlesRoot, entry.tar);
      if (!tarPath || !fs.existsSync(tarPath)) {
        if (!managedImageAvailable) {
          expectFilePath(tarPath, errors, `image tar ${imageId} (${target.label})`);
        }
      }
    }
  }
};

const validateLockV2 = ({ lock, manifest, manifestPath, profile, overridesPath, hostOs, hostArch, errors }) => {
  if (lock.version !== 2) {
    errors.push(`runtime lock version must be 2 (got ${JSON.stringify(lock.version)})`);
    return { manifest, appliedOverrides: [] };
  }

  if (!lock.profiles || typeof lock.profiles !== "object") {
    errors.push("runtime lock profiles must be an object");
  }
  for (const profileName of PROFILE_VALUES) {
    const profileCfg = lock.profiles?.[profileName];
    if (!profileCfg || typeof profileCfg !== "object") {
      errors.push(`runtime lock missing profiles.${profileName}`);
      continue;
    }
    if (!Array.isArray(profileCfg.allowed_source_types) || profileCfg.allowed_source_types.length === 0) {
      errors.push(`runtime lock profiles.${profileName}.allowed_source_types must be a non-empty array`);
    }
  }

  const components = Array.isArray(lock.components) ? lock.components : [];
  if (components.length === 0) {
    errors.push("runtime lock components must be a non-empty array");
  }

  const componentMap = new Map();
  for (const component of components) {
    const kind = String(component?.kind || "").trim();
    const id = String(component?.id || "").trim();
    const os = String(component?.os || "").trim();
    const arch = String(component?.arch || "").trim();
    const version = String(component?.version || "").trim();

    if (!COMPONENT_KINDS.has(kind)) {
      errors.push(`component kind must be provider|runtime|image|machine_cache (got ${JSON.stringify(component?.kind)})`);
      continue;
    }
    if (!id || !os || !arch || !version) {
      errors.push(`component ${JSON.stringify(component)} missing required fields kind/id/os/arch/version`);
      continue;
    }
    if (!Array.isArray(component.sources) || component.sources.length === 0) {
      errors.push(`component ${kind}/${id} ${os}/${arch} must include non-empty sources`);
      continue;
    }

    const dedupeKey = componentKey(component, hostOs, hostArch);
    if (componentMap.has(dedupeKey)) {
      errors.push(`duplicate component key ${dedupeKey}`);
      continue;
    }
    componentMap.set(dedupeKey, component);

    for (const source of component.sources) {
      const sourceType = String(source?.source_type || "").trim();
      if (!SOURCE_TYPES.has(sourceType)) {
        errors.push(`component ${kind}/${id} has unsupported source_type '${sourceType}'`);
        continue;
      }
      if (sourceType !== "local") {
        if (!isNonEmptyString(source?.uri)) {
          errors.push(`component ${kind}/${id} source '${sourceType}' missing uri`);
        }
        if (!isNonEmptyString(source?.sha256)) {
          errors.push(`component ${kind}/${id} source '${sourceType}' missing sha256`);
        }
      }
    }
  }

  const requiredProviderIds = validateRequiredArray(
    lock?.required?.provider_ids,
    "runtime lock required.provider_ids",
    errors,
    { allowEmpty: true },
  );
  const requiredRuntimeIds = validateRequiredArray(
    lock?.required?.runtime_ids,
    "runtime lock required.runtime_ids",
    errors,
    { allowEmpty: true },
  );
  const requiredImageIds = validateRequiredArray(
    lock?.required?.image_ids,
    "runtime lock required.image_ids",
    errors,
    { allowEmpty: true },
  );
  const requiredMachineCacheIds = validateRequiredArray(
    lock?.required?.machine_cache_ids ?? [],
    "runtime lock required.machine_cache_ids",
    errors,
    { allowEmpty: true },
  );
  const providerTargets = resolveRequiredTargets({
    lock,
    hostOs,
    hostArch,
    kind: "provider",
    errors,
    allowEmpty: requiredProviderIds.length === 0,
  });
  const runtimeTargets = resolveRequiredTargets({
    lock,
    hostOs,
    hostArch,
    kind: "runtime",
    errors,
    allowEmpty: requiredRuntimeIds.length === 0,
  });
  const imageTargets = resolveRequiredTargets({
    lock,
    hostOs,
    hostArch,
    kind: "image",
    errors,
    allowEmpty: requiredImageIds.length === 0,
  });
  const machineCacheTargets = resolveRequiredTargets({
    lock,
    hostOs,
    hostArch,
    kind: "machine_cache",
    errors,
    allowEmpty: requiredMachineCacheIds.length === 0,
  });

  const requiredComponents = [];

  for (const providerId of requiredProviderIds) {
    for (const target of providerTargets) {
      const component = findComponent({
        components,
        kind: "provider",
        id: providerId,
        os: target.os,
        arch: target.arch,
        hostOs,
        hostArch,
      });
      if (!component) {
        errors.push(`runtime lock missing provider component ${providerId} for ${target.label}`);
      } else {
        requiredComponents.push(component);
      }
    }
  }

  for (const runtimeId of requiredRuntimeIds) {
    for (const target of runtimeTargets) {
      const component = findComponent({
        components,
        kind: "runtime",
        id: runtimeId,
        os: target.os,
        arch: target.arch,
        hostOs,
        hostArch,
      });
      if (!component) {
        errors.push(`runtime lock missing runtime component ${runtimeId} for ${target.label}`);
      } else {
        requiredComponents.push(component);
      }
    }
  }

  for (const imageId of requiredImageIds) {
    for (const target of imageTargets) {
      const component = findComponent({
        components,
        kind: "image",
        id: imageId,
        os: target.os,
        arch: target.arch,
        hostOs,
        hostArch,
      });
      if (!component) {
        errors.push(`runtime lock missing image component ${imageId} for ${target.label}`);
      } else {
        requiredComponents.push(component);
      }
    }
  }

  for (const machineCacheId of requiredMachineCacheIds) {
    for (const target of machineCacheTargets) {
      const component = findComponent({
        components,
        kind: "machine_cache",
        id: machineCacheId,
        os: target.os,
        arch: target.arch,
        hostOs,
        hostArch,
      });
      if (!component) {
        errors.push(`runtime lock missing machine_cache component ${machineCacheId} for ${target.label}`);
      } else {
        requiredComponents.push(component);
      }
    }
  }

  const allowedSourceTypes = validateProfileSources({ lock, profile, requiredComponents, errors });
  const requiredComponentMap = new Map(
    requiredComponents.map((component) => [componentKey(component, hostOs, hostArch), component]),
  );

  const { overrides, appliedOverrides } = loadOverrides({ profile, overridesPath, errors });
  for (const override of overrides) {
    const key = componentKey(override, hostOs, hostArch);
    if (!componentMap.has(key)) {
      errors.push(`override does not match any runtime lock component: ${key}`);
    }
  }

  const effectiveManifest = applyOverridesToManifest({
    manifest,
    overrides,
    hostOs,
    hostArch,
    errors,
  });

  validateManifestEntries({
    lock,
    manifest: effectiveManifest,
    manifestPath,
    hostOs,
    hostArch,
    providerTargets,
    runtimeTargets,
    imageTargets,
    requiredComponentMap,
    allowedSourceTypes,
    allowEmptyRequired: {
      provider: requiredProviderIds.length === 0,
      runtime: requiredRuntimeIds.length === 0,
      image: requiredImageIds.length === 0,
    },
    errors,
  });

  return { manifest: effectiveManifest, appliedOverrides };
};

const validateLockV1 = ({ lock, manifest, manifestPath, hostOs, hostArch, errors }) => {
  if (lock.version !== 1) {
    errors.push(`runtime lock version must be 1 (got ${JSON.stringify(lock.version)})`);
  }
  const providerTargets = resolveRequiredTargets({ lock, hostOs, hostArch, kind: "provider", errors });
  const runtimeTargets = resolveRequiredTargets({ lock, hostOs, hostArch, kind: "runtime", errors });
  const imageTargets = resolveRequiredTargets({ lock, hostOs, hostArch, kind: "image", errors });
  validateManifestEntries({
    lock,
    manifest,
    manifestPath,
    hostOs,
    hostArch,
    providerTargets,
    runtimeTargets,
    imageTargets,
    errors,
  });
  return { manifest, appliedOverrides: [] };
};

const resolveDefaultLockPath = () => (fs.existsSync(defaultLockPathV2) ? defaultLockPathV2 : defaultLockPathV1);

const validateRuntimeLock = ({
  lockPath = resolveDefaultLockPath(),
  manifestPath = defaultManifestPath,
  profile = "parity",
  overridesPath = defaultOverridesPath,
} = {}) => {
  const errors = [];
  const selectedProfile = parseProfile(profile, errors);

  if (!fs.existsSync(lockPath)) {
    errors.push(`missing runtime lock file: ${lockPath}`);
    return { ok: false, errors, profile: selectedProfile };
  }
  if (!fs.existsSync(manifestPath)) {
    errors.push(`missing bundle manifest file: ${manifestPath}`);
    return { ok: false, errors, profile: selectedProfile };
  }

  const lock = readJson(lockPath, errors, "runtime lock");
  const manifest = readJson(manifestPath, errors, "bundle manifest");
  if (!lock || !manifest) return { ok: false, errors, profile: selectedProfile };

  const hostOs = normalizeManifestOs(process.platform);
  const hostArch = normalizeManifestArch(process.arch);

  let effectiveManifest = manifest;
  let appliedOverrides = [];

  if (lock.version === 2) {
    const result = validateLockV2({
      lock,
      manifest,
      manifestPath,
      profile: selectedProfile,
      overridesPath,
      hostOs,
      hostArch,
      errors,
    });
    effectiveManifest = result.manifest;
    appliedOverrides = result.appliedOverrides;
  } else {
    const result = validateLockV1({
      lock,
      manifest,
      manifestPath,
      hostOs,
      hostArch,
      errors,
    });
    effectiveManifest = result.manifest;
    appliedOverrides = result.appliedOverrides;
  }

  return {
    ok: errors.length === 0,
    errors,
    lockVersion: lock.version,
    profile: selectedProfile,
    appliedOverrides,
    effectiveManifest,
  };
};

const parseCliArgs = (argv) => {
  const positional = [];
  let profile = process.env.CTX_RUNTIME_PROFILE || "parity";
  let overridesPath = process.env.CTX_RUNTIME_OVERRIDES_PATH || defaultOverridesPath;
  let effectiveManifestOut = null;

  for (let idx = 0; idx < argv.length; idx += 1) {
    const arg = argv[idx];
    if (arg === "--profile") {
      profile = argv[idx + 1] || profile;
      idx += 1;
      continue;
    }
    if (arg === "--overrides") {
      overridesPath = argv[idx + 1] || overridesPath;
      idx += 1;
      continue;
    }
    if (arg === "--effective-manifest-out") {
      effectiveManifestOut = argv[idx + 1] || effectiveManifestOut;
      idx += 1;
      continue;
    }
    positional.push(arg);
  }

  const lockPath = positional[0] ? path.resolve(positional[0]) : resolveDefaultLockPath();
  const manifestPath = positional[1] ? path.resolve(positional[1]) : defaultManifestPath;

  return {
    lockPath,
    manifestPath,
    profile,
    overridesPath,
    effectiveManifestOut: effectiveManifestOut ? path.resolve(effectiveManifestOut) : null,
  };
};

if (require.main === module) {
  const cli = parseCliArgs(process.argv.slice(2));
  const result = validateRuntimeLock({
    lockPath: cli.lockPath,
    manifestPath: cli.manifestPath,
    profile: cli.profile,
    overridesPath: cli.overridesPath,
  });

  if (!result.ok) {
    for (const error of result.errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }

  if (cli.effectiveManifestOut) {
    fs.mkdirSync(path.dirname(cli.effectiveManifestOut), { recursive: true });
    fs.writeFileSync(cli.effectiveManifestOut, `${JSON.stringify(result.effectiveManifest, null, 2)}\n`, "utf8");
  }

  console.log(
    `runtime_lock_validate: OK (${path.relative(coreRoot, cli.lockPath)}) lock=v${result.lockVersion} profile=${result.profile} overrides=${result.appliedOverrides.length}`,
  );
}

module.exports = {
  validateRuntimeLock,
  resolveDefaultLockPath,
};
