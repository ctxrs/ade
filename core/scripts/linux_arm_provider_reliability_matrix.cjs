#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const defaultMatrixPath = path.join(
  coreRoot,
  "apps",
  "web",
  "e2e",
  "fixtures",
  "linux_arm_provider_reliability_matrix.json",
);

const readJson = (filePath) => JSON.parse(fs.readFileSync(filePath, "utf8"));

const asArray = (value) => (Array.isArray(value) ? value : []);

const normalizeProviderRow = (row) => {
  const value = row && typeof row === "object" ? row : {};
  const providerId = String(value.provider_id || "").trim();
  const modelOverride = String(value.openrouter_model_override || "").trim();
  return {
    provider_id: providerId,
    openrouter_model_override: modelOverride,
  };
};

const assertUniqueProviderIds = (providers, label) => {
  const seen = new Set();
  for (const provider of providers) {
    const providerId = String(provider.provider_id || "").trim();
    if (!providerId) {
      throw new Error(`${label} contains an entry without provider_id`);
    }
    if (seen.has(providerId)) {
      throw new Error(`${label} contains duplicate provider_id: ${providerId}`);
    }
    seen.add(providerId);
  }
};

const validateMatrix = (matrix) => {
  if (!matrix || typeof matrix !== "object") {
    throw new Error("matrix must be an object");
  }
  const version = Number(matrix.version);
  if (!Number.isFinite(version) || version < 1) {
    throw new Error("matrix version must be >= 1");
  }

  const critical = asArray(matrix.critical_providers).map(normalizeProviderRow);
  const nightly = asArray(matrix.full_nightly_providers).map(normalizeProviderRow);

  if (critical.length === 0) {
    throw new Error("critical_providers must not be empty");
  }
  if (nightly.length === 0) {
    throw new Error("full_nightly_providers must not be empty");
  }

  assertUniqueProviderIds(critical, "critical_providers");
  assertUniqueProviderIds(nightly, "full_nightly_providers");

  const nightlySet = new Set(nightly.map((row) => row.provider_id));
  for (const row of critical) {
    if (!nightlySet.has(row.provider_id)) {
      throw new Error(`critical provider missing from full_nightly_providers: ${row.provider_id}`);
    }
  }

  return {
    ...matrix,
    critical_providers: critical,
    full_nightly_providers: nightly,
    deferred_providers: asArray(matrix.deferred_providers),
    unsupported_providers: asArray(matrix.unsupported_providers).map((value) => String(value || "").trim()).filter(Boolean),
  };
};

const loadMatrix = (inputPath = defaultMatrixPath) => {
  const matrixPath = path.isAbsolute(inputPath)
    ? inputPath
    : path.resolve(process.cwd(), inputPath);
  const raw = readJson(matrixPath);
  const validated = validateMatrix(raw);
  return { matrixPath, matrix: validated };
};

const providerIdsForLane = (matrix, lane) => {
  if (lane === "critical") {
    return matrix.critical_providers.map((row) => row.provider_id);
  }
  if (lane === "nightly") {
    return matrix.full_nightly_providers.map((row) => row.provider_id);
  }
  throw new Error(`unsupported lane: ${lane}`);
};

const modelOverrideMapForLane = (matrix, lane) => {
  const source = lane === "critical" ? matrix.critical_providers : matrix.full_nightly_providers;
  const out = {};
  for (const row of source) {
    if (row.openrouter_model_override) {
      out[row.provider_id] = row.openrouter_model_override;
    }
  }
  return out;
};

const parseArgs = (argv) => {
  const opts = {
    matrixPath: defaultMatrixPath,
    lane: "critical",
    json: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--matrix") {
      opts.matrixPath = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg === "--lane") {
      opts.lane = String(argv[i + 1] || "").trim().toLowerCase();
      i += 1;
      continue;
    }
    if (arg === "--json") {
      opts.json = true;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }
  return opts;
};

const printHelp = () => {
  console.log([
    "Usage: node core/scripts/linux_arm_provider_reliability_matrix.cjs [options]",
    "",
    "Options:",
    "  --matrix <path>            Matrix JSON path",
    "  --lane <critical|nightly>  Provider lane to emit",
    "  --json                     Print JSON payload instead of CSV ids",
    "  --help                     Show help",
  ].join("\n"));
};

if (require.main === module) {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    printHelp();
    process.exit(0);
  }
  const { matrixPath, matrix } = loadMatrix(opts.matrixPath);
  const providerIds = providerIdsForLane(matrix, opts.lane);
  const payload = {
    matrix_path: matrixPath,
    lane: opts.lane,
    provider_ids: providerIds,
    model_overrides: modelOverrideMapForLane(matrix, opts.lane),
    expected_environment: String(matrix.expected_environment || "").trim(),
    expected_network_mode: String(matrix.expected_network_mode || "").trim(),
    deferred_count: asArray(matrix.deferred_providers).length,
    unsupported_count: asArray(matrix.unsupported_providers).length,
  };
  if (opts.json) {
    process.stdout.write(`${JSON.stringify(payload, null, 2)}\n`);
  } else {
    process.stdout.write(`${providerIds.join(",")}\n`);
  }
}

module.exports = {
  defaultMatrixPath,
  loadMatrix,
  modelOverrideMapForLane,
  providerIdsForLane,
  validateMatrix,
};
