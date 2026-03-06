#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const {
  defaultMatrixPath,
  loadMatrix,
  providerIdsForLane,
} = require("./linux_arm_provider_reliability_matrix.cjs");

const coreRoot = path.resolve(__dirname, "..");
const defaultProviderMatrixPath = path.join(coreRoot, "crates", "ctx-http", "src", "provider_matrix.json");
const defaultRuntimeLockPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "bundles", "runtime_lock.v2.json");

const LINUX_ARCH_TARGET_KEY = "linux-aarch64";
const LINUX_OS = "linux";
const LINUX_ARCH = "aarch64";

const readJson = (filePath, label) => {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new Error(`failed to parse ${label} at ${filePath}: ${error?.message || String(error)}`);
  }
};

const isSha256 = (value) => /^[0-9a-f]{64}$/i.test(String(value || "").trim());
const normalizeText = (value) => String(value || "").trim();
const normalizeUri = (value) => String(value || "").trim().toLowerCase();

const uriHasArchMismatch = (uri) => {
  const value = normalizeUri(uri);
  if (!value) return false;
  return value.includes("x86_64") || value.includes("amd64") || value.includes("linux-x64") || value.includes("linux/x86_64");
};

const uriLooksLinuxArm = (uri) => {
  const value = normalizeUri(uri);
  if (!value) return false;
  return (
    value.includes("linux/aarch64")
    || value.includes("linux-aarch64")
    || value.includes("linux/arm64")
    || value.includes("linux-arm64")
  );
};

const parseArgs = (argv) => {
  const opts = {
    lane: "critical",
    matrixManifestPath: defaultMatrixPath,
    providerMatrixPath: defaultProviderMatrixPath,
    runtimeLockPath: defaultRuntimeLockPath,
    strictSizeBytes: false,
    reportPath: "",
    printJson: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--lane") {
      opts.lane = normalizeText(argv[i + 1]).toLowerCase();
      i += 1;
      continue;
    }
    if (arg === "--matrix-manifest") {
      opts.matrixManifestPath = normalizeText(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--provider-matrix") {
      opts.providerMatrixPath = normalizeText(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--runtime-lock") {
      opts.runtimeLockPath = normalizeText(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--strict-size-bytes") {
      opts.strictSizeBytes = true;
      continue;
    }
    if (arg === "--report") {
      opts.reportPath = normalizeText(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--json") {
      opts.printJson = true;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }

  if (opts.lane !== "critical" && opts.lane !== "nightly") {
    throw new Error("--lane must be critical or nightly");
  }

  return opts;
};

const printHelp = () => {
  console.log([
    "Usage: node core/scripts/linux_arm_provider_preflight.cjs [options]",
    "",
    "Options:",
    "  --lane <critical|nightly>       Provider lane to validate",
    "  --matrix-manifest <path>        Reliability matrix manifest path",
    "  --provider-matrix <path>        Provider matrix path",
    "  --runtime-lock <path>           Runtime lock path",
    "  --strict-size-bytes             Treat missing size_bytes as error",
    "  --report <path>                 Write JSON report to file",
    "  --json                          Print JSON report to stdout",
    "  --help                          Show help",
  ].join("\n"));
};

const providerComponentForLinuxArm = (runtimeLock, providerId) => {
  const components = Array.isArray(runtimeLock?.components) ? runtimeLock.components : [];
  return components.find(
    (component) => component
      && component.kind === "provider"
      && normalizeText(component.id) === providerId
      && normalizeText(component.os) === LINUX_OS
      && normalizeText(component.arch) === LINUX_ARCH,
  ) || null;
};

const runtimeComponentForLinuxArm = (runtimeLock, kind, id) => {
  const components = Array.isArray(runtimeLock?.components) ? runtimeLock.components : [];
  return components.find(
    (component) => component
      && component.kind === kind
      && normalizeText(component.id) === id
      && normalizeText(component.os) === LINUX_OS
      && normalizeText(component.arch) === LINUX_ARCH,
  ) || null;
};

const validateArchiveTargetForLinuxArm = ({
  targetEntry,
  label,
  errors,
  warnings,
  strictSizeBytes,
}) => {
  if (!targetEntry || typeof targetEntry !== "object") {
    errors.push(`missing ${label} target ${LINUX_ARCH_TARGET_KEY}`);
    return;
  }

  const url = normalizeText(targetEntry.url);
  const archive = normalizeText(targetEntry.archive);
  const binPath = normalizeText(targetEntry.bin_path);
  const sha256 = normalizeText(targetEntry.sha256);
  const sizeBytes = Number(targetEntry.size_bytes);

  if (!url) errors.push(`${label} target missing url`);
  if (!archive) errors.push(`${label} target missing archive type`);
  if (!binPath) errors.push(`${label} target missing bin_path`);
  if (binPath.endsWith(".exe")) errors.push(`${label} target bin_path points to Windows executable`);
  if (!isSha256(sha256)) errors.push(`${label} target sha256 missing/invalid`);
  if (url && uriHasArchMismatch(url)) errors.push(`${label} target url suggests non-arm64 artifact: ${url}`);
  if (url && !uriLooksLinuxArm(url)) {
    warnings.push(`${label} target url does not explicitly include linux arm token: ${url}`);
  }
  if (!Number.isFinite(sizeBytes) || sizeBytes <= 0) {
    const msg = `${label} target missing/invalid size_bytes`;
    if (strictSizeBytes) {
      errors.push(msg);
    } else {
      warnings.push(msg);
    }
  }
};

const validateProviderEntry = ({ providerId, providerMatrixById, runtimeLock, strictSizeBytes }) => {
  const errors = [];
  const warnings = [];

  const entry = providerMatrixById.get(providerId);
  if (!entry) {
    return {
      provider_id: providerId,
      status: "failed",
      stage: "preflight",
      error_code: "matrix_provider_missing",
      errors: [`provider not found in provider_matrix.json: ${providerId}`],
      warnings,
    };
  }

  const managedInstall = entry?.managed_install && typeof entry.managed_install === "object"
    ? entry.managed_install
    : null;
  if (!managedInstall) {
    errors.push("managed_install missing");
  } else {
    const kind = normalizeText(managedInstall.kind).toLowerCase();
    if (kind !== "archive") {
      errors.push(`managed_install.kind expected archive but found ${kind || "<empty>"}`);
    }
    const targets = managedInstall.targets && typeof managedInstall.targets === "object"
      ? managedInstall.targets
      : {};
    validateArchiveTargetForLinuxArm({
      targetEntry: targets[LINUX_ARCH_TARGET_KEY],
      label: "managed_install",
      errors,
      warnings,
      strictSizeBytes,
    });
  }

  const dependencies = Array.isArray(entry.dependencies) ? entry.dependencies : [];
  for (const dependency of dependencies) {
    const dependencyId = normalizeText(dependency?.id);
    const install = dependency?.install && typeof dependency.install === "object"
      ? dependency.install
      : null;
    const kind = normalizeText(install?.kind).toLowerCase();
    if (kind !== "archive") {
      continue;
    }
    const targets = install.targets && typeof install.targets === "object"
      ? install.targets
      : {};
    validateArchiveTargetForLinuxArm({
      targetEntry: targets[LINUX_ARCH_TARGET_KEY],
      label: `dependency ${dependencyId || "<unnamed>"}`,
      errors,
      warnings,
      strictSizeBytes,
    });
  }

  const providerComponent = providerComponentForLinuxArm(runtimeLock, providerId);
  if (!providerComponent) {
    errors.push(`runtime lock missing provider component ${providerId} linux/aarch64`);
  } else {
    const sources = Array.isArray(providerComponent.sources) ? providerComponent.sources : [];
    if (sources.length === 0) {
      errors.push(`runtime lock provider component ${providerId} has no sources`);
    }
    const uriSources = sources
      .map((source) => normalizeText(source && typeof source === "object" ? source.uri : ""))
      .filter(Boolean);
    if (uriSources.some(uriHasArchMismatch)) {
      errors.push(`runtime lock provider component ${providerId} contains non-arm64 source uri`);
    }
  }

  return {
    provider_id: providerId,
    status: errors.length === 0 ? "passed" : "failed",
    stage: "preflight",
    error_code: errors.length === 0 ? "" : "preflight_failed",
    errors,
    warnings,
  };
};

const validateRuntimeComponents = (runtimeLock) => {
  const checks = [];
  const errors = [];

  const expected = [
    { kind: "runtime", id: "node" },
    { kind: "runtime", id: "python" },
    { kind: "image", id: "ctx-harness" },
  ];

  for (const item of expected) {
    const component = runtimeComponentForLinuxArm(runtimeLock, item.kind, item.id);
    if (!component) {
      errors.push(`missing runtime lock component ${item.kind}/${item.id} for linux/aarch64`);
      checks.push({ kind: item.kind, id: item.id, status: "failed", error: "component_missing" });
      continue;
    }
    const sources = Array.isArray(component.sources) ? component.sources : [];
    if (sources.length === 0) {
      errors.push(`${item.kind}/${item.id} linux/aarch64 has no sources`);
      checks.push({ kind: item.kind, id: item.id, status: "failed", error: "sources_missing" });
      continue;
    }

    const externalSources = sources.filter((source) => {
      const sourceType = normalizeText(source && typeof source === "object" ? source.source_type : "").toLowerCase();
      return sourceType !== "local";
    });
    const hasNonLocalSource = externalSources.length > 0;
    const hasArchMismatch = externalSources.some((source) => uriHasArchMismatch(source.uri));

    if (hasArchMismatch) {
      errors.push(`${item.kind}/${item.id} linux/aarch64 includes non-arm64 source uri`);
      checks.push({ kind: item.kind, id: item.id, status: "failed", error: "source_arch_mismatch" });
      continue;
    }
    if (!hasNonLocalSource) {
      checks.push({ kind: item.kind, id: item.id, status: "warning", warning: "only_local_source" });
      continue;
    }
    checks.push({ kind: item.kind, id: item.id, status: "passed" });
  }

  return { checks, errors };
};

const runPreflight = (options) => {
  const { matrixPath, matrix } = loadMatrix(options.matrixManifestPath);
  const providerMatrix = readJson(options.providerMatrixPath, "provider matrix");
  const runtimeLock = readJson(options.runtimeLockPath, "runtime lock");
  const laneProviders = providerIdsForLane(matrix, options.lane);

  const providerMatrixById = new Map(
    asArray(providerMatrix.providers)
      .filter((entry) => entry && typeof entry === "object")
      .map((entry) => [normalizeText(entry.id), entry]),
  );

  const providerResults = laneProviders.map((providerId) =>
    validateProviderEntry({
      providerId,
      providerMatrixById,
      runtimeLock,
      strictSizeBytes: options.strictSizeBytes,
    }),
  );

  const runtime = validateRuntimeComponents(runtimeLock);
  const providerErrors = providerResults.flatMap((row) => row.errors.map((message) => `${row.provider_id}: ${message}`));
  const providerWarnings = providerResults.flatMap((row) => row.warnings.map((message) => `${row.provider_id}: ${message}`));

  const report = {
    generated_at: new Date().toISOString(),
    lane: options.lane,
    matrix_manifest_path: matrixPath,
    provider_matrix_path: path.resolve(options.providerMatrixPath),
    runtime_lock_path: path.resolve(options.runtimeLockPath),
    providers_checked: laneProviders,
    provider_results: providerResults,
    runtime_checks: runtime.checks,
    errors: [...providerErrors, ...runtime.errors],
    warnings: providerWarnings,
    summary: {
      providers_total: providerResults.length,
      providers_passed: providerResults.filter((row) => row.status === "passed").length,
      providers_failed: providerResults.filter((row) => row.status === "failed").length,
      runtime_failed: runtime.checks.filter((row) => row.status === "failed").length,
      runtime_warning: runtime.checks.filter((row) => row.status === "warning").length,
      errors_total: providerErrors.length + runtime.errors.length,
      warnings_total: providerWarnings.length,
    },
  };

  return report;
};

const asArray = (value) => (Array.isArray(value) ? value : []);

if (require.main === module) {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printHelp();
    process.exit(0);
  }

  const report = runPreflight(options);

  if (options.reportPath) {
    const outputPath = path.isAbsolute(options.reportPath)
      ? options.reportPath
      : path.resolve(process.cwd(), options.reportPath);
    fs.mkdirSync(path.dirname(outputPath), { recursive: true });
    fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
    console.log(`wrote linux-arm preflight report: ${outputPath}`);
  }

  if (options.printJson) {
    console.log(JSON.stringify(report, null, 2));
  } else {
    console.log(
      `linux-arm preflight: lane=${report.lane} providers=${report.summary.providers_total} passed=${report.summary.providers_passed} failed=${report.summary.providers_failed} runtime_failed=${report.summary.runtime_failed}`,
    );
    for (const warning of report.warnings) {
      console.warn(`warn: ${warning}`);
    }
  }

  if (report.summary.errors_total > 0) {
    for (const error of report.errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }
}

module.exports = {
  runPreflight,
  validateRuntimeComponents,
};
