#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const {
  defaultMatrixPath,
  loadMatrix,
  providerIdsForLane,
} = require("./linux_arm_provider_reliability_matrix.cjs");

const readJson = (filePath, label) => {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new Error(`failed to parse ${label} at ${filePath}: ${error?.message || String(error)}`);
  }
};

const normalizeText = (value) => String(value || "").trim();
const asArray = (value) => (Array.isArray(value) ? value : []);

const parseArgs = (argv) => {
  const opts = {
    lane: "critical",
    matrixManifestPath: defaultMatrixPath,
    reportPath: "",
    strictMissing: true,
    allowOverride: false,
    overrideTicket: "",
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
    if (arg === "--report") {
      opts.reportPath = normalizeText(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--allow-missing") {
      opts.strictMissing = false;
      continue;
    }
    if (arg === "--allow-override") {
      opts.allowOverride = true;
      continue;
    }
    if (arg === "--override-ticket") {
      opts.overrideTicket = normalizeText(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }

  if (!opts.help && !opts.reportPath) {
    throw new Error("--report is required");
  }
  if (opts.lane !== "critical" && opts.lane !== "nightly") {
    throw new Error("--lane must be critical or nightly");
  }
  return opts;
};

const printHelp = () => {
  console.log([
    "Usage: node core/scripts/linux_arm_provider_release_gate.cjs --report <path> [options]",
    "",
    "Options:",
    "  --report <path>               Runtime install smoke report",
    "  --lane <critical|nightly>     Lane to validate (default: critical)",
    "  --matrix-manifest <path>      Reliability matrix manifest",
    "  --allow-missing               Do not fail on missing provider rows",
    "  --allow-override              Allow manual override of failures (requires --override-ticket)",
    "  --override-ticket <ticket>    Approval ticket/id required when --allow-override is used",
    "  --help                        Show help",
  ].join("\n"));
};

const validateReleaseGate = ({ lane, matrixManifestPath, reportPath, strictMissing }) => {
  const { matrixPath, matrix } = loadMatrix(matrixManifestPath);
  const expectedProviders = providerIdsForLane(matrix, lane);
  const report = readJson(reportPath, "runtime install smoke report");

  const rows = asArray(report.results).map((entry) => (entry && typeof entry === "object" ? entry : {}));
  const rowByProvider = new Map(rows.map((row) => [normalizeText(row.provider_id), row]));

  const errors = [];
  const passed = [];

  for (const providerId of expectedProviders) {
    const row = rowByProvider.get(providerId);
    if (!row) {
      if (strictMissing) {
        errors.push(`missing report row for expected provider: ${providerId}`);
      }
      continue;
    }

    const result = normalizeText(row.result).toLowerCase();
    if (result !== "pass") {
      const stage = normalizeText(row.stage) || "unknown";
      const errorCode = normalizeText(row.error_code) || "none";
      const reason = normalizeText(row.reason) || "no reason provided";
      errors.push(`provider ${providerId} failed (${stage}, ${errorCode}): ${reason}`);
      continue;
    }

    passed.push(providerId);
  }

  return {
    lane,
    matrix_manifest_path: matrixPath,
    report_path: path.resolve(reportPath),
    expected_providers: expectedProviders,
    passed,
    errors,
    ok: errors.length === 0,
  };
};

if (require.main === module) {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    printHelp();
    process.exit(0);
  }

  const result = validateReleaseGate(opts);
  console.log(`linux-arm release gate: lane=${result.lane} expected=${result.expected_providers.length} passed=${result.passed.length}`);

  if (!result.ok) {
    if (opts.allowOverride) {
      if (!opts.overrideTicket) {
        console.error("error: --allow-override requires --override-ticket <ticket>");
        process.exit(1);
      }
      console.warn(
        `warn: linux-arm release gate override applied (ticket=${opts.overrideTicket}); critical failures are being bypassed for this run`,
      );
      for (const error of result.errors) {
        console.warn(`warn: ${error}`);
      }
      process.exit(0);
    }
    for (const error of result.errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }
}

module.exports = {
  validateReleaseGate,
};
