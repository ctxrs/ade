#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const defaultManifestPath = path.join(
  coreRoot,
  "apps",
  "desktop",
  "automation",
  "fixtures",
  "provider_auth_matrix.json",
);
const defaultReportPath = path.join(
  coreRoot,
  "apps",
  "desktop",
  "automation",
  "docs",
  "provider_auth_matrix.md",
);

const SUPPORT_VALUES = new Set(["supported", "deferred", "unsupported"]);
const LANE_VALUES = new Set(["required", "nightly", "none"]);
const RUNNER_KINDS = new Set(["desktop_wdio", "web_playwright", "none"]);
const DEFERRED_RUNNER_SKIP_REASONS = new Set([
  "missing_ci_secret_contract",
  "missing_seed_fixture",
  "provider_external_challenge",
]);
const REQUIRED_PROVIDER_IDS = new Set(["codex"]);
const REQUIRED_AUTH_MODES = new Set(["endpoint_api_key", "configure_later_then_connect"]);
const REQUIRED_DAEMON_LOCATIONS = new Set(["local"]);
const REQUIRED_EXECUTION_ENVIRONMENTS = new Set(["host", "sandbox"]);
const REQUIRED_ALLOWED_PREREQUISITES = new Set(["OPENROUTER_API_KEY"]);

const resolveInputPath = (raw, { fallbackPath = "", mustExist = false } = {}) => {
  const value = String(raw || "").trim();
  if (!value) return fallbackPath;
  if (path.isAbsolute(value)) return value;
  const fromCwd = path.resolve(process.cwd(), value);
  if (!mustExist || fs.existsSync(fromCwd)) return fromCwd;
  return path.resolve(coreRoot, value);
};

const parseArgs = (argv) => {
  const opts = {
    manifestPath: defaultManifestPath,
    reportPath: "",
    checkReport: false,
    checkReportPath: defaultReportPath,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--manifest") {
      opts.manifestPath = resolveInputPath(argv[i + 1], { fallbackPath: defaultManifestPath, mustExist: true });
      i += 1;
      continue;
    }
    if (arg === "--report") {
      opts.reportPath = resolveInputPath(argv[i + 1], { mustExist: false });
      i += 1;
      continue;
    }
    if (arg === "--check-report") {
      opts.checkReport = true;
      const next = argv[i + 1];
      if (next && !next.startsWith("-")) {
        opts.checkReportPath = resolveInputPath(next, { fallbackPath: defaultReportPath, mustExist: false });
        i += 1;
      } else if (opts.reportPath) {
        opts.checkReportPath = opts.reportPath;
      }
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }

  if (!opts.checkReportPath && opts.reportPath) {
    opts.checkReportPath = opts.reportPath;
  }

  return opts;
};

const asArray = (value) => (Array.isArray(value) ? value : []);
const asRecord = (value) =>
  value && typeof value === "object" && !Array.isArray(value) ? value : {};
const readString = (value) => (typeof value === "string" ? value : "");

const readJson = (filePath) => JSON.parse(fs.readFileSync(filePath, "utf8"));

const ensureUniqueIds = (items, label, errors) => {
  const seen = new Set();
  for (const item of items) {
    const id = readString(item.id).trim();
    if (!id) {
      errors.push(`${label} entry missing id`);
      continue;
    }
    if (seen.has(id)) {
      errors.push(`${label} has duplicate id: ${id}`);
      continue;
    }
    seen.add(id);
  }
  return seen;
};

const deriveExecutionTopology = (daemonLocation, executionEnvironment) => {
  const normalizedDaemonLocation = readString(daemonLocation).trim();
  const normalizedExecutionEnvironment = readString(executionEnvironment).trim();
  if (!normalizedDaemonLocation || !normalizedExecutionEnvironment) return "";
  return `${normalizedDaemonLocation}_${normalizedExecutionEnvironment}`;
};

const buildCellId = (providerId, authMode, daemonLocation, executionEnvironment) =>
  `${providerId}.${authMode}.${daemonLocation}.${executionEnvironment}`;

const validateManifest = (manifest) => {
  const errors = [];
  const warnings = [];

  const providers = asArray(manifest.providers).map(asRecord);
  const authModes = asArray(manifest.auth_modes).map(asRecord);
  const daemonLocations = asArray(manifest.daemon_locations).map(asRecord);
  const executionEnvironments = asArray(manifest.execution_environments).map(asRecord);
  const cells = asArray(manifest.cells).map(asRecord);
  const assertionDefs = asRecord(manifest.assertion_definitions);
  const assertionIds = new Set(Object.keys(assertionDefs));

  if (manifest.schema_version !== 1) {
    errors.push(`schema_version must be 1 (got ${JSON.stringify(manifest.schema_version)})`);
  }
  if (!readString(manifest.generated_at).trim()) {
    errors.push("generated_at is required");
  }
  if (!readString(manifest.summary).trim()) {
    warnings.push("summary is empty");
  }
  if (providers.length === 0) errors.push("providers must be non-empty");
  if (authModes.length === 0) errors.push("auth_modes must be non-empty");
  if (daemonLocations.length === 0) errors.push("daemon_locations must be non-empty");
  if (executionEnvironments.length === 0) errors.push("execution_environments must be non-empty");
  if (cells.length === 0) errors.push("cells must be non-empty");
  if (assertionIds.size === 0) errors.push("assertion_definitions must be non-empty");

  const providerIdSet = ensureUniqueIds(providers, "providers", errors);
  const authModeSet = ensureUniqueIds(authModes, "auth_modes", errors);
  const daemonLocationSet = ensureUniqueIds(daemonLocations, "daemon_locations", errors);
  const executionEnvironmentSet = ensureUniqueIds(executionEnvironments, "execution_environments", errors);

  const expectedIds = new Set();
  for (const providerId of providerIdSet) {
    for (const authMode of authModeSet) {
      for (const daemonLocation of daemonLocationSet) {
        for (const executionEnvironment of executionEnvironmentSet) {
          expectedIds.add(buildCellId(providerId, authMode, daemonLocation, executionEnvironment));
        }
      }
    }
  }

  const seenCellIds = new Set();
  const executionTopologySet = new Set();
  const laneCounts = { required: 0, nightly: 0, none: 0 };
  const supportCounts = { supported: 0, deferred: 0, unsupported: 0 };
  const providerCounts = new Map();

  for (const cell of cells) {
    const id = readString(cell.id).trim();
    if (!id) {
      errors.push("cell missing id");
      continue;
    }
    if (seenCellIds.has(id)) {
      errors.push(`duplicate cell id: ${id}`);
      continue;
    }
    seenCellIds.add(id);

    const providerId = readString(cell.provider_id).trim();
    const authMode = readString(cell.auth_mode).trim();
    const daemonLocation = readString(cell.daemon_location).trim();
    const executionEnvironment = readString(cell.execution_environment).trim();
    const executionTopology = deriveExecutionTopology(daemonLocation, executionEnvironment);
    const support = readString(cell.support).trim();
    const lane = readString(cell.lane).trim();
    const owner = readString(cell.owner).trim();
    const skipReason = readString(cell.skip_reason).trim();
    const runner = asRecord(cell.runner);
    const runnerKind = readString(runner.kind).trim();
    const requiredAssertions = asArray(cell.required_assertions).map((entry) => readString(entry).trim()).filter(Boolean);
    const prerequisites = asArray(cell.prerequisites).map((entry) => readString(entry).trim()).filter(Boolean);

    if (!providerIdSet.has(providerId)) errors.push(`cell ${id} has unknown provider_id '${providerId}'`);
    if (!authModeSet.has(authMode)) errors.push(`cell ${id} has unknown auth_mode '${authMode}'`);
    if (!daemonLocationSet.has(daemonLocation)) {
      errors.push(`cell ${id} has unknown daemon_location '${daemonLocation}'`);
    }
    if (!executionEnvironmentSet.has(executionEnvironment)) {
      errors.push(`cell ${id} has unknown execution_environment '${executionEnvironment}'`);
    }
    if (buildCellId(providerId, authMode, daemonLocation, executionEnvironment) !== id) {
      errors.push(`cell ${id} does not match provider/auth/location/environment tuple`);
    }

    if (!SUPPORT_VALUES.has(support)) errors.push(`cell ${id} has invalid support '${support}'`);
    if (!LANE_VALUES.has(lane)) errors.push(`cell ${id} has invalid lane '${lane}'`);
    if (!owner) warnings.push(`cell ${id} missing owner`);
    if (!RUNNER_KINDS.has(runnerKind)) errors.push(`cell ${id} has invalid runner.kind '${runnerKind}'`);

    if (support === "supported" && lane === "none") {
      errors.push(`cell ${id} is supported but lane=none`);
    }
    if (support !== "supported" && !skipReason) {
      errors.push(`cell ${id} must include skip_reason when support != supported`);
    }
    if (support === "supported" && skipReason) {
      errors.push(`cell ${id} should not include skip_reason when support=supported`);
    }
    if (lane === "required" && support !== "supported") {
      errors.push(`cell ${id} lane=required requires support=supported`);
    }

    if (requiredAssertions.length === 0) {
      errors.push(`cell ${id} required_assertions must be non-empty`);
    }
    for (const assertionId of requiredAssertions) {
      if (!assertionIds.has(assertionId)) {
        errors.push(`cell ${id} references unknown assertion '${assertionId}'`);
      }
    }
    if (support === "unsupported" && !requiredAssertions.includes("unsupported_contract")) {
      errors.push(`cell ${id} support=unsupported must include 'unsupported_contract' assertion`);
    }

    if (support === "supported" && runnerKind === "none") {
      errors.push(`cell ${id} support=supported requires a concrete runner`);
    }
    if (support === "deferred" && runnerKind !== "none" && !DEFERRED_RUNNER_SKIP_REASONS.has(skipReason)) {
      errors.push(
        `cell ${id} support=deferred with concrete runner requires skip_reason in ${JSON.stringify([...DEFERRED_RUNNER_SKIP_REASONS])}`,
      );
    }
    if (runnerKind === "desktop_wdio" && !readString(runner.spec).trim()) {
      errors.push(`cell ${id} desktop_wdio runner requires spec`);
    }
    if (runnerKind === "desktop_wdio" && !readString(runner.scenarios).trim()) {
      errors.push(`cell ${id} desktop_wdio runner requires scenarios`);
    }
    if (
      providerId === "codex"
      && REQUIRED_AUTH_MODES.has(authMode)
      && daemonLocation === "local"
      && executionEnvironment === "host"
      && runnerKind === "desktop_wdio"
      && !readString(runner.scenarios).split(",").map((entry) => entry.trim()).includes("local-codex-host-smoke")
    ) {
      errors.push(`cell ${id} local host Codex required coverage must include local-codex-host-smoke`);
    }
    if (
      providerId === "codex"
      && REQUIRED_AUTH_MODES.has(authMode)
      && daemonLocation === "local"
      && executionEnvironment === "sandbox"
      && runnerKind === "desktop_wdio"
      && !readString(runner.scenarios).split(",").map((entry) => entry.trim()).includes("local-codex-smoke")
    ) {
      errors.push(`cell ${id} local sandbox Codex required coverage must include local-codex-smoke`);
    }
    if (runnerKind === "web_playwright" && !readString(runner.spec).trim()) {
      errors.push(`cell ${id} web_playwright runner requires spec`);
    }
    for (const key of prerequisites) {
      if (!/^[A-Z0-9_]+$/.test(key)) {
        errors.push(`cell ${id} has invalid prerequisite env var '${key}'`);
      }
    }
    if (lane === "required") {
      if (!REQUIRED_PROVIDER_IDS.has(providerId)) {
        errors.push(`cell ${id} lane=required requires provider_id in ${JSON.stringify([...REQUIRED_PROVIDER_IDS])}`);
      }
      if (!REQUIRED_AUTH_MODES.has(authMode)) {
        errors.push(`cell ${id} lane=required requires auth_mode in ${JSON.stringify([...REQUIRED_AUTH_MODES])}`);
      }
      if (!REQUIRED_DAEMON_LOCATIONS.has(daemonLocation)) {
        errors.push(
          `cell ${id} lane=required requires daemon_location in ${JSON.stringify([...REQUIRED_DAEMON_LOCATIONS])}`,
        );
      }
      if (!REQUIRED_EXECUTION_ENVIRONMENTS.has(executionEnvironment)) {
        errors.push(
          `cell ${id} lane=required requires execution_environment in ${JSON.stringify([...REQUIRED_EXECUTION_ENVIRONMENTS])}`,
        );
      }
      if (runnerKind !== "desktop_wdio") {
        errors.push(`cell ${id} lane=required requires desktop_wdio runner`);
      }
      if (prerequisites.length === 0) {
        errors.push(`cell ${id} lane=required requires explicit prerequisites`);
      }
      for (const key of prerequisites) {
        if (!REQUIRED_ALLOWED_PREREQUISITES.has(key)) {
          errors.push(
            `cell ${id} lane=required prerequisite '${key}' is outside approved required-lane secret set ${JSON.stringify([...REQUIRED_ALLOWED_PREREQUISITES])}`,
          );
        }
      }
    }

    executionTopologySet.add(executionTopology);
    if (supportCounts[support] !== undefined) supportCounts[support] += 1;
    if (laneCounts[lane] !== undefined) laneCounts[lane] += 1;
    const existingProviderCounts = providerCounts.get(providerId) || {
      supported: 0,
      deferred: 0,
      unsupported: 0,
    };
    if (existingProviderCounts[support] !== undefined) {
      existingProviderCounts[support] += 1;
    }
    providerCounts.set(providerId, existingProviderCounts);
  }

  for (const expectedId of expectedIds) {
    if (!seenCellIds.has(expectedId)) {
      errors.push(`missing matrix cell: ${expectedId}`);
    }
  }
  for (const id of seenCellIds) {
    if (!expectedIds.has(id)) {
      errors.push(`unexpected extra matrix cell: ${id}`);
    }
  }

  return {
    errors,
    warnings,
    summary: {
      providers: providerIdSet.size,
      auth_modes: authModeSet.size,
      daemon_locations: daemonLocationSet.size,
      execution_environments: executionEnvironmentSet.size,
      execution_topologies: executionTopologySet.size,
      expected_cells: expectedIds.size,
      actual_cells: seenCellIds.size,
      support_counts: supportCounts,
      lane_counts: laneCounts,
      provider_counts: Object.fromEntries(
        [...providerCounts.entries()].sort(([a], [b]) => a.localeCompare(b)),
      ),
    },
  };
};

const buildReport = (manifest, validationSummary) => {
  const cells = asArray(manifest.cells).map(asRecord).sort((a, b) => readString(a.id).localeCompare(readString(b.id)));
  const requiredCells = cells.filter((cell) => readString(cell.lane) === "required");
  const deferredRunnerCells = cells.filter((cell) => {
    const runner = asRecord(cell.runner);
    return readString(cell.support) === "deferred" && readString(runner.kind) !== "none";
  });
  const summary = validationSummary.summary;

  const lines = [];
  lines.push("# Provider Auth Matrix");
  lines.push("");
  lines.push("Generated from `core/apps/desktop/automation/fixtures/provider_auth_matrix.json`.");
  lines.push("");
  lines.push(`- Generated at: ${readString(manifest.generated_at)}`);
  lines.push(`- Providers: ${summary.providers}`);
  lines.push(`- Auth modes: ${summary.auth_modes}`);
  lines.push(`- Daemon locations: ${summary.daemon_locations}`);
  lines.push(`- Execution environments: ${summary.execution_environments}`);
  lines.push(`- Execution topologies: ${summary.execution_topologies}`);
  lines.push(`- Cells: ${summary.actual_cells}`);
  lines.push("");
  lines.push("## Support Counts");
  lines.push("");
  lines.push(`- supported: ${summary.support_counts.supported}`);
  lines.push(`- deferred: ${summary.support_counts.deferred}`);
  lines.push(`- unsupported: ${summary.support_counts.unsupported}`);
  lines.push("");
  lines.push("## Lane Counts");
  lines.push("");
  lines.push(`- required: ${summary.lane_counts.required}`);
  lines.push(`- nightly: ${summary.lane_counts.nightly}`);
  lines.push(`- none: ${summary.lane_counts.none}`);
  lines.push("");
  lines.push("## Required Lane Cells");
  lines.push("");
  lines.push("| cell_id | provider | auth_mode | daemon_location | execution_environment | execution_topology | runner | prerequisites |");
  lines.push("| --- | --- | --- | --- | --- | --- | --- | --- |");
  for (const cell of requiredCells) {
    const runner = asRecord(cell.runner);
    const prereq = asArray(cell.prerequisites).map((entry) => readString(entry)).filter(Boolean).join(", ");
    lines.push(
      `| ${readString(cell.id)} | ${readString(cell.provider_id)} | ${readString(cell.auth_mode)} | ${readString(cell.daemon_location)} | ${readString(cell.execution_environment)} | ${deriveExecutionTopology(cell.daemon_location, cell.execution_environment)} | ${readString(runner.kind)} | ${prereq || "-"} |`,
    );
  }
  if (requiredCells.length === 0) {
    lines.push("| _none_ | - | - | - | - | - | - | - |");
  }
  lines.push("");
  lines.push("## Deferred Cells With Concrete Runners");
  lines.push("");
  lines.push("| cell_id | provider | auth_mode | daemon_location | execution_environment | execution_topology | runner | blocker | prerequisites |");
  lines.push("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
  for (const cell of deferredRunnerCells) {
    const runner = asRecord(cell.runner);
    const prereq = asArray(cell.prerequisites).map((entry) => readString(entry)).filter(Boolean).join(", ");
    lines.push(
      `| ${readString(cell.id)} | ${readString(cell.provider_id)} | ${readString(cell.auth_mode)} | ${readString(cell.daemon_location)} | ${readString(cell.execution_environment)} | ${deriveExecutionTopology(cell.daemon_location, cell.execution_environment)} | ${readString(runner.kind)} | ${readString(cell.skip_reason) || "-"} | ${prereq || "-"} |`,
    );
  }
  if (deferredRunnerCells.length === 0) {
    lines.push("| _none_ | - | - | - | - | - | - | - | - |");
  }
  lines.push("");
  lines.push("## Provider Coverage Summary");
  lines.push("");
  lines.push("| provider | supported | deferred | unsupported |");
  lines.push("| --- | ---: | ---: | ---: |");
  for (const [providerId, counts] of Object.entries(summary.provider_counts)) {
    lines.push(
      `| ${providerId} | ${counts.supported} | ${counts.deferred} | ${counts.unsupported} |`,
    );
  }
  lines.push("");
  lines.push("## Notes");
  lines.push("");
  lines.push("- `supported` means the cell currently has automated execution coverage.");
  lines.push("- `deferred` means provider/auth behavior is expected, but automated coverage is still a known gap.");
  lines.push("- `unsupported` means the provider/auth combination is intentionally out of supported product behavior.");
  lines.push("- `execution_topology` is derived from `daemon_location` and `execution_environment` for human-readable reports only.");
  lines.push("- Deferred cells with concrete runners must carry a blocker classification in `skip_reason`.");
  lines.push("");

  return `${lines.join("\n")}\n`;
};

const printHelp = () => {
  console.log([
    "Usage: node core/scripts/validate_provider_auth_matrix.cjs [options]",
    "",
    "Options:",
    "  --manifest <path>       Matrix manifest JSON path",
    "  --report <path>         Write generated markdown report",
    "  --check-report [path]   Verify report file content is up-to-date",
    "  --help                  Show help",
  ].join("\n"));
};

const main = () => {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    printHelp();
    return;
  }

  const manifest = readJson(opts.manifestPath);
  const validation = validateManifest(manifest);

  const reportBody = buildReport(manifest, validation);
  if (opts.reportPath) {
    fs.mkdirSync(path.dirname(opts.reportPath), { recursive: true });
    fs.writeFileSync(opts.reportPath, reportBody, "utf8");
    console.log(`wrote report: ${opts.reportPath}`);
  }

  if (opts.checkReport) {
    if (!fs.existsSync(opts.checkReportPath)) {
      validation.errors.push(`report file not found for --check-report: ${opts.checkReportPath}`);
    } else {
      const existing = fs.readFileSync(opts.checkReportPath, "utf8");
      if (existing !== reportBody) {
        validation.errors.push(
          `report out of date: ${opts.checkReportPath} (run with --report ${opts.checkReportPath})`,
        );
      }
    }
  }

  console.log(`providers=${validation.summary.providers}`);
  console.log(`auth_modes=${validation.summary.auth_modes}`);
  console.log(`daemon_locations=${validation.summary.daemon_locations}`);
  console.log(`execution_environments=${validation.summary.execution_environments}`);
  console.log(`execution_topologies=${validation.summary.execution_topologies}`);
  console.log(`cells=${validation.summary.actual_cells}/${validation.summary.expected_cells}`);
  console.log(`support_counts=${JSON.stringify(validation.summary.support_counts)}`);
  console.log(`lane_counts=${JSON.stringify(validation.summary.lane_counts)}`);

  for (const warning of validation.warnings) {
    console.warn(`warn: ${warning}`);
  }
  if (validation.errors.length > 0) {
    for (const error of validation.errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }

  console.log("ok: provider auth matrix manifest validation passed");
};

main();
