#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const defaultManifestPath = path.join(
  coreRoot,
  "apps",
  "desktop",
  "automation",
  "fixtures",
  "harness_install_matrix.json",
);
const defaultReportPath = path.join(
  coreRoot,
  "apps",
  "desktop",
  "automation",
  "docs",
  "harness_install_matrix.md",
);
const defaultProviderMatrixPath = path.join(
  coreRoot,
  "crates",
  "ctx-provider-accounts",
  "src",
  "provider_matrix.json",
);

const REQUIRED_PLATFORM_TARGETS = Object.freeze([
  { id: "macos.host", platform: "macos", execution_target: "host", install_target: "host" },
  { id: "macos.sandbox", platform: "macos", execution_target: "sandbox", install_target: "container" },
  { id: "linux.host", platform: "linux", execution_target: "host", install_target: "host" },
  { id: "linux.sandbox", platform: "linux", execution_target: "sandbox", install_target: "container" },
]);
const REQUIRED_PLATFORM_TARGET_IDS = new Set(REQUIRED_PLATFORM_TARGETS.map((target) => target.id));
const SUPPORT_VALUES = new Set(["supported", "unsupported"]);
const LANE_VALUES = new Set(["preview", "release", "nightly"]);
const RUNNER_KINDS = new Set(["release_runtime_install_smoke", "none"]);
const REQUIRED_ASSERTIONS = Object.freeze([
  "canonical_provider_id",
  "catalog_install_supported",
  "install_started",
  "install_succeeded",
  "status_installed",
  "runtime_command_configured",
  "no_alias_or_fallback",
]);
const DISALLOWED_PROVIDER_IDS = new Set([
  "codex-crp",
]);

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
    providerMatrixPath: defaultProviderMatrixPath,
    reportPath: "",
    checkReport: false,
    checkReportPath: defaultReportPath,
    writeDefaultManifest: false,
    help: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--manifest") {
      opts.manifestPath = resolveInputPath(argv[i + 1], { fallbackPath: defaultManifestPath, mustExist: false });
      i += 1;
      continue;
    }
    if (arg === "--provider-matrix") {
      opts.providerMatrixPath = resolveInputPath(argv[i + 1], {
        fallbackPath: defaultProviderMatrixPath,
        mustExist: true,
      });
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
    if (arg === "--write-default-manifest") {
      opts.writeDefaultManifest = true;
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

const asArray = (value) => (Array.isArray(value) ? value : []);
const asRecord = (value) =>
  value && typeof value === "object" && !Array.isArray(value) ? value : {};
const readString = (value) => (typeof value === "string" ? value : "");

const readJson = (filePath) => JSON.parse(fs.readFileSync(filePath, "utf8"));

const relativeRepoPath = (filePath) => path.relative(repoRoot, filePath).split(path.sep).join("/");

const normalizeProviderMatrixProvider = (entry) => {
  const provider = asRecord(entry);
  const managedInstall = asRecord(provider.managed_install);
  const id = readString(provider.id).trim();
  return {
    id,
    display_name: readString(provider.display_name).trim() || id,
    kind: readString(provider.kind).trim() || "provider",
    tier: readString(provider.tier).trim(),
    command: readString(asRecord(provider.command).command).trim(),
    install_kind: readString(managedInstall.kind).trim(),
    managed_install: managedInstall,
  };
};

const collectManagedProviders = (providerMatrix) => {
  const providers = asArray(providerMatrix.providers)
    .map(normalizeProviderMatrixProvider)
    .filter((provider) => provider.id && provider.install_kind)
    .sort((a, b) => a.id.localeCompare(b.id));
  const productProviders = providers.filter((provider) => provider.kind !== "dependency");
  const dependencyProviders = providers.filter((provider) => provider.kind === "dependency");
  return {
    productProviders,
    dependencyProviders,
    allManagedProviders: providers,
  };
};

const buildCellId = (providerId, platformTargetId) => `${providerId}.${platformTargetId}`;

const defaultLanesForTarget = (target) => {
  const lanes = ["release", "nightly"];
  if (target.platform === "macos") lanes.unshift("preview");
  return lanes;
};

const buildDefaultManifest = ({ providerMatrix, generatedAt = "2026-04-29" }) => {
  const { productProviders } = collectManagedProviders(providerMatrix);
  return {
    schema_version: 1,
    generated_at: generatedAt,
    summary: "Harness provider installability contract across platform and execution target. Provider ids are product ids; adapter/runtime ids are not provider aliases.",
    source_provider_matrix: "core/crates/ctx-provider-accounts/src/provider_matrix.json",
    platform_targets: REQUIRED_PLATFORM_TARGETS,
    assertion_definitions: {
      canonical_provider_id: "The cell uses the product provider id from provider_matrix.json, never an adapter/runtime alias.",
      catalog_install_supported: "The provider catalog reports install support for the requested target.",
      install_started: "The install API returns an install id for the requested provider/target.",
      install_succeeded: "The install reaches succeeded before timeout.",
      status_installed: "Post-install status reports installed=true and install_running=false.",
      runtime_command_configured: "The installed provider has a configured runtime command for the canonical provider id.",
      no_alias_or_fallback: "No fallback provider id, adapter id, or alternate install path is accepted as success.",
    },
    providers: productProviders.map((provider) => ({
      id: provider.id,
      display_name: provider.display_name,
      owner: `provider-${provider.id}`,
      install_kind: provider.install_kind,
      command: provider.command,
      cells: Object.fromEntries(REQUIRED_PLATFORM_TARGETS.map((target) => [
        target.id,
        {
          support: "supported",
          lanes: defaultLanesForTarget(target),
          install_target: target.install_target,
          required_assertions: REQUIRED_ASSERTIONS,
          runner: {
            kind: "release_runtime_install_smoke",
            script: "scripts/release_runtime_install_smoke.sh",
          },
          product_reason: "",
        },
      ])),
    })),
  };
};

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

const validateManifest = (manifest, providerMatrix) => {
  const errors = [];
  const warnings = [];
  const { productProviders, dependencyProviders } = collectManagedProviders(providerMatrix);
  const expectedProviderIds = new Set(productProviders.map((provider) => provider.id));
  const dependencyProviderIds = new Set(dependencyProviders.map((provider) => provider.id));
  const expectedProviderById = new Map(productProviders.map((provider) => [provider.id, provider]));
  const providers = asArray(manifest.providers).map(asRecord);
  const platformTargets = asArray(manifest.platform_targets).map(asRecord);
  const assertionDefinitions = asRecord(manifest.assertion_definitions);
  const assertionIds = new Set(Object.keys(assertionDefinitions));

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
  if (platformTargets.length === 0) errors.push("platform_targets must be non-empty");
  if (assertionIds.size === 0) errors.push("assertion_definitions must be non-empty");

  const platformTargetIds = ensureUniqueIds(platformTargets, "platform_targets", errors);
  for (const target of REQUIRED_PLATFORM_TARGETS) {
    if (!platformTargetIds.has(target.id)) {
      errors.push(`missing required platform target: ${target.id}`);
    }
  }
  for (const target of platformTargets) {
    const id = readString(target.id).trim();
    if (!REQUIRED_PLATFORM_TARGET_IDS.has(id)) {
      errors.push(`unexpected platform target: ${id}`);
    }
    const platform = readString(target.platform).trim();
    const executionTarget = readString(target.execution_target).trim();
    const installTarget = readString(target.install_target).trim();
    if (!["macos", "linux"].includes(platform)) {
      errors.push(`platform target ${id} has invalid platform '${platform}'`);
    }
    if (!["host", "sandbox"].includes(executionTarget)) {
      errors.push(`platform target ${id} has invalid execution_target '${executionTarget}'`);
    }
    if (!["host", "container"].includes(installTarget)) {
      errors.push(`platform target ${id} has invalid install_target '${installTarget}'`);
    }
  }

  const providerIds = ensureUniqueIds(providers, "providers", errors);
  for (const expectedProviderId of expectedProviderIds) {
    if (!providerIds.has(expectedProviderId)) {
      errors.push(`missing installable harness provider: ${expectedProviderId}`);
    }
  }

  const summary = {
    providers: providers.length,
    expected_providers: expectedProviderIds.size,
    platform_targets: platformTargetIds.size,
    expected_platform_targets: REQUIRED_PLATFORM_TARGETS.length,
    cells: 0,
    expected_cells: expectedProviderIds.size * REQUIRED_PLATFORM_TARGETS.length,
    lane_counts: { preview: 0, release: 0, nightly: 0 },
    support_counts: { supported: 0, unsupported: 0 },
    provider_counts: {},
  };

  for (const provider of providers) {
    const providerId = readString(provider.id).trim();
    const providerLabel = providerId || "<missing-provider-id>";
    if (DISALLOWED_PROVIDER_IDS.has(providerId)) {
      errors.push(`provider ${providerId} is an adapter/runtime id and must not appear as a harness provider id`);
    }
    if (dependencyProviderIds.has(providerId)) {
      errors.push(`provider ${providerId} is a dependency provider, not a user-selectable harness provider`);
    }
    if (!expectedProviderIds.has(providerId)) {
      errors.push(`provider ${providerLabel} is not an installable harness provider in provider_matrix.json`);
    }
    const expectedProvider = expectedProviderById.get(providerId);
    const installKind = readString(provider.install_kind).trim();
    if (expectedProvider && installKind !== expectedProvider.install_kind) {
      errors.push(
        `provider ${providerId} install_kind '${installKind}' does not match provider_matrix.json '${expectedProvider.install_kind}'`,
      );
    }

    const cells = asRecord(provider.cells);
    const providerCounts = { supported: 0, unsupported: 0 };
    for (const target of REQUIRED_PLATFORM_TARGETS) {
      const cell = asRecord(cells[target.id]);
      const cellId = buildCellId(providerId, target.id);
      if (Object.keys(cell).length === 0) {
        errors.push(`missing matrix cell: ${cellId}`);
        continue;
      }
      summary.cells += 1;
      const support = readString(cell.support).trim();
      const lanes = asArray(cell.lanes).map((lane) => readString(lane).trim()).filter(Boolean);
      const requiredAssertions = asArray(cell.required_assertions)
        .map((assertion) => readString(assertion).trim())
        .filter(Boolean);
      const runner = asRecord(cell.runner);
      const runnerKind = readString(runner.kind).trim();
      const script = readString(runner.script).trim();
      const installTarget = readString(cell.install_target).trim();
      const productReason = readString(cell.product_reason).trim();

      if (!SUPPORT_VALUES.has(support)) errors.push(`cell ${cellId} has invalid support '${support}'`);
      if (summary.support_counts[support] !== undefined) summary.support_counts[support] += 1;
      if (providerCounts[support] !== undefined) providerCounts[support] += 1;
      if (installTarget !== target.install_target) {
        errors.push(`cell ${cellId} install_target '${installTarget}' must be '${target.install_target}'`);
      }
      for (const lane of lanes) {
        if (!LANE_VALUES.has(lane)) errors.push(`cell ${cellId} has invalid lane '${lane}'`);
        if (summary.lane_counts[lane] !== undefined) summary.lane_counts[lane] += 1;
      }
      if (!RUNNER_KINDS.has(runnerKind)) {
        errors.push(`cell ${cellId} has invalid runner.kind '${runnerKind}'`);
      }
      for (const assertionId of requiredAssertions) {
        if (!assertionIds.has(assertionId)) {
          errors.push(`cell ${cellId} references unknown assertion '${assertionId}'`);
        }
      }

      if (support === "supported") {
        for (const requiredAssertion of REQUIRED_ASSERTIONS) {
          if (!requiredAssertions.includes(requiredAssertion)) {
            errors.push(`cell ${cellId} missing required assertion '${requiredAssertion}'`);
          }
        }
        if (!lanes.includes("release")) errors.push(`cell ${cellId} support=supported must include release lane`);
        if (!lanes.includes("nightly")) errors.push(`cell ${cellId} support=supported must include nightly lane`);
        if (target.platform === "macos" && !lanes.includes("preview")) {
          errors.push(`cell ${cellId} macOS support=supported must include preview lane`);
        }
        if (target.platform !== "macos" && lanes.includes("preview")) {
          errors.push(`cell ${cellId} non-macOS cell must not include preview lane`);
        }
        if (runnerKind !== "release_runtime_install_smoke") {
          errors.push(`cell ${cellId} support=supported requires release_runtime_install_smoke runner`);
        }
        if (script !== "scripts/release_runtime_install_smoke.sh") {
          errors.push(`cell ${cellId} runner.script must be scripts/release_runtime_install_smoke.sh`);
        }
      } else if (support === "unsupported") {
        if (!productReason) errors.push(`cell ${cellId} support=unsupported requires product_reason`);
        if (runnerKind !== "none") errors.push(`cell ${cellId} support=unsupported requires runner.kind=none`);
        if (lanes.includes("release")) errors.push(`cell ${cellId} support=unsupported must not include release lane`);
      }
    }

    for (const targetId of Object.keys(cells)) {
      if (!REQUIRED_PLATFORM_TARGET_IDS.has(targetId)) {
        errors.push(`provider ${providerLabel} has unexpected cell target '${targetId}'`);
      }
    }
    summary.provider_counts[providerId] = providerCounts;
  }

  return { errors, warnings, summary };
};

const flattenCells = (manifest) => {
  const platformTargets = new Map(asArray(manifest.platform_targets).map((target) => {
    const row = asRecord(target);
    return [readString(row.id).trim(), row];
  }));
  const rows = [];
  for (const provider of asArray(manifest.providers).map(asRecord)) {
    const providerId = readString(provider.id).trim();
    for (const [targetId, rawCell] of Object.entries(asRecord(provider.cells))) {
      const target = platformTargets.get(targetId) || {};
      const cell = asRecord(rawCell);
      rows.push({
        id: buildCellId(providerId, targetId),
        provider_id: providerId,
        display_name: readString(provider.display_name).trim() || providerId,
        platform_target: targetId,
        platform: readString(target.platform).trim(),
        execution_target: readString(target.execution_target).trim(),
        install_target: readString(cell.install_target).trim() || readString(target.install_target).trim(),
        support: readString(cell.support).trim(),
        lanes: asArray(cell.lanes).map((lane) => readString(lane).trim()).filter(Boolean),
        runner: asRecord(cell.runner),
      });
    }
  }
  return rows.sort((a, b) => a.id.localeCompare(b.id));
};

const buildReport = (manifest, validation) => {
  const rows = flattenCells(manifest);
  const summary = validation.summary;
  const lines = [];
  lines.push("# Harness Install Matrix");
  lines.push("");
  lines.push("Generated from `core/apps/desktop/automation/fixtures/harness_install_matrix.json`.");
  lines.push("");
  lines.push(`- Generated at: ${readString(manifest.generated_at)}`);
  lines.push(`- Source provider matrix: \`${readString(manifest.source_provider_matrix) || "core/crates/ctx-provider-accounts/src/provider_matrix.json"}\``);
  lines.push(`- Providers: ${summary.providers}/${summary.expected_providers}`);
  lines.push(`- Platform targets: ${summary.platform_targets}/${summary.expected_platform_targets}`);
  lines.push(`- Cells: ${summary.cells}/${summary.expected_cells}`);
  lines.push("");
  lines.push("## Lane Counts");
  lines.push("");
  lines.push(`- preview: ${summary.lane_counts.preview}`);
  lines.push(`- release: ${summary.lane_counts.release}`);
  lines.push(`- nightly: ${summary.lane_counts.nightly}`);
  lines.push("");
  lines.push("## Support Counts");
  lines.push("");
  lines.push(`- supported: ${summary.support_counts.supported}`);
  lines.push(`- unsupported: ${summary.support_counts.unsupported}`);
  lines.push("");
  lines.push("## Cells");
  lines.push("");
  lines.push("| cell | provider | platform | execution target | install target | support | lanes | runner |");
  lines.push("| --- | --- | --- | --- | --- | --- | --- | --- |");
  for (const row of rows) {
    lines.push(
      `| ${row.id} | ${row.provider_id} | ${row.platform} | ${row.execution_target} | ${row.install_target} | ${row.support} | ${row.lanes.join(", ") || "-"} | ${readString(row.runner.kind) || "-"} |`,
    );
  }
  lines.push("");
  lines.push("## Contract");
  lines.push("");
  lines.push("- Provider ids are canonical product ids from `provider_matrix.json`; adapter/runtime ids such as `codex-crp` do not satisfy a provider cell.");
  lines.push("- `preview` covers macOS host and sandbox installability for the produced preview `.app`.");
  lines.push("- `release` covers every installable provider on macOS host, macOS sandbox, Linux host, and Linux sandbox before stable promotion.");
  lines.push("- `nightly` repeats installability and may add launch/probe/first-turn breadth where deterministic credentials exist.");
  lines.push("- Live runners must continue through all selected cells and report every failed provider/target before exiting non-zero.");
  lines.push("");

  return `${lines.join("\n")}\n`;
};

const printHelp = () => {
  console.log([
    "Usage: node core/scripts/validate_harness_install_matrix.cjs [options]",
    "",
    "Options:",
    "  --manifest <path>              Harness install matrix JSON path",
    "  --provider-matrix <path>       provider_matrix.json path",
    "  --report <path>                Write generated markdown report",
    "  --check-report [path]          Verify report file content is up-to-date",
    "  --write-default-manifest       Write a manifest generated from provider_matrix.json",
    "  --help                         Show help",
  ].join("\n"));
};

const main = () => {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    printHelp();
    return;
  }

  const providerMatrix = readJson(opts.providerMatrixPath);
  if (opts.writeDefaultManifest) {
    const manifest = buildDefaultManifest({ providerMatrix });
    fs.mkdirSync(path.dirname(opts.manifestPath), { recursive: true });
    fs.writeFileSync(opts.manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
    console.log(`wrote manifest: ${opts.manifestPath}`);
  }

  const manifest = readJson(opts.manifestPath);
  const validation = validateManifest(manifest, providerMatrix);
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
          `report out of date: ${opts.checkReportPath} (run with --report ${relativeRepoPath(opts.checkReportPath)})`,
        );
      }
    }
  }

  console.log(`providers=${validation.summary.providers}/${validation.summary.expected_providers}`);
  console.log(`platform_targets=${validation.summary.platform_targets}/${validation.summary.expected_platform_targets}`);
  console.log(`cells=${validation.summary.cells}/${validation.summary.expected_cells}`);
  console.log(`lane_counts=${JSON.stringify(validation.summary.lane_counts)}`);
  console.log(`support_counts=${JSON.stringify(validation.summary.support_counts)}`);

  for (const warning of validation.warnings) {
    console.warn(`warn: ${warning}`);
  }
  if (validation.errors.length > 0) {
    for (const error of validation.errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }

  console.log("ok: harness install matrix validation passed");
};

if (require.main === module) {
  main();
}

module.exports = {
  REQUIRED_PLATFORM_TARGETS,
  REQUIRED_ASSERTIONS,
  buildDefaultManifest,
  buildReport,
  collectManagedProviders,
  flattenCells,
  validateManifest,
};
