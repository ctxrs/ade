const fs = require("node:fs");
const path = require("node:path");

const {
  CTX_HTTP_SHARED_SOURCE_GLOBS,
  CTX_HTTP_SUITES,
  validateCtxHttpSuites,
} = require("../ctx_http_suites.cjs");
const { FAMILIES, getFamiliesById } = require("./families.cjs");
const { sortEntries, validateEntry } = require("./schema.cjs");

const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");
const coreRoot = path.join(repoRoot, "core");
const packageJsonPath = path.join(coreRoot, "package.json");
const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
const providerMatrixPath = path.join(coreRoot, "apps", "desktop", "automation", "fixtures", "provider_auth_matrix.json");
const providerMatrix = JSON.parse(fs.readFileSync(providerMatrixPath, "utf8"));
const webRoot = path.join(coreRoot, "apps", "web");
const webE2ERoot = path.join(webRoot, "e2e");
const webSuiteRoot = path.join(webE2ERoot, "suites");
const webSuites = ["premerge_required", "release_required", "cross_platform", "visual", "soak", "load"];
const RELEASE_RELEVANT_GLOBS = [
  ".buildkite/pipelines/release.yml",
  "core/apps/desktop/**",
  "core/crates/ctx-http/**",
  "core/scripts/desktop_check_versions.cjs",
  "core/scripts/desktop_set_version.cjs",
  "core/scripts/desktop_version.cjs",
  "core/scripts/release_version.cjs",
  "scripts/buildbuddy/run_release*.sh",
  "scripts/release_*.sh",
  "tools/bazel/**",
];

function toPosix(value) {
  return value.split(path.sep).join("/");
}

function relativeToRepo(absPath) {
  return toPosix(path.relative(repoRoot, absPath));
}

function normalizeManifestSpec(line) {
  const cleaned = String(line || "").trim();
  if (!cleaned || cleaned.startsWith("#")) {
    return null;
  }
  return cleaned.startsWith("e2e/") ? cleaned : `e2e/${cleaned}`;
}

function readWebSuiteMap() {
  const map = new Map();
  for (const suite of webSuites) {
    const manifestPath = path.join(webSuiteRoot, `${suite}.txt`);
    const specs = [...new Set(fs
      .readFileSync(manifestPath, "utf8")
      .split(/\r?\n/u)
      .map(normalizeManifestSpec)
      .filter(Boolean)
    )].sort();
    map.set(suite, specs);
  }
  return map;
}

function classifyWebSpecFamily(specPath) {
  const base = path.basename(specPath);
  if (/attachment/u.test(base)) {
    return "attachments-artifacts";
  }
  if (/update|updater/u.test(base)) {
    return "updates-release";
  }
  if (/settings/u.test(base)) {
    return "settings-config";
  }
  if (/provider/u.test(base)) {
    return "provider-runtime";
  }
  if (/terminal/u.test(base)) {
    return "turns-terminal";
  }
  if (/ws-|snapshot|stream|session-gap/u.test(base)) {
    return "workspace-stream";
  }
  return "web-workbench";
}

function buildCtxHttpEntries() {
  const entries = [];
  for (const suite of CTX_HTTP_SUITES) {
    const familyBySuite = {
      base: "build-graph",
      "workspace-stream": "workspace-stream",
      "provider-auth": "provider-auth",
      "provider-runtime": "provider-runtime",
      "repo-vcs": "repo-vcs",
      lsp: "lsp-editing",
      "turns-terminal": "turns-terminal",
      "attachments-routing": "attachments-artifacts",
      "subagents-control": "subagents-orchestration",
      "updates-release": "updates-release",
      "sandbox-cloud": "sandbox-runtime",
    };
    const worldBySuite = {
      base: "hermetic",
      "workspace-stream": "simulated",
      "provider-auth": "fake-provider",
      "provider-runtime": "external-service",
      "repo-vcs": "simulated",
      lsp: "simulated",
      "turns-terminal": "simulated",
      "attachments-routing": "simulated",
      "subagents-control": "simulated",
      "updates-release": "simulated",
      "sandbox-cloud": "external-service",
    };
    const executionBySuite = {
      base: "bazel-rbe-preferred",
      "workspace-stream": "bazel-rbe-preferred",
      "provider-auth": "bazel-rbe-preferred",
      "provider-runtime": "bazel-addressable",
      "repo-vcs": "bazel-rbe-preferred",
      lsp: "bazel-rbe-preferred",
      "turns-terminal": "bazel-rbe-preferred",
      "attachments-routing": "bazel-rbe-preferred",
      "subagents-control": "bazel-rbe-preferred",
      "updates-release": "bazel-addressable",
      "sandbox-cloud": "bazel-addressable",
    };
    const entry = {
      id: `ctx-http.${suite.name}`,
      title: `ctx-http: ${suite.name}`,
      family: familyBySuite[suite.name],
      entrypointType: "ctx-http-suite",
      entrypoint: suite.name,
      surface: suite.type === "base" ? "compile" : "integration",
      oracle: suite.type === "base" ? "compiler" : "direct-assertion",
      world: worldBySuite[suite.name],
      cost: suite.type === "base" ? "fast" : "fast",
      requirements: ["linux", "buildbuddy-rbe"],
      stability: worldBySuite[suite.name] === "external-service" ? "quarantined" : "stable",
      execution: executionBySuite[suite.name],
      owner: "ctx-http",
      sourceGlobs: [
        "core/scripts/lib/ctx_http_suites.cjs",
        ...(suite.type === "integration" ? CTX_HTTP_SHARED_SOURCE_GLOBS.map((glob) => `core/${glob}`) : []),
        ...suite.sourceGlobs.map((glob) => `core/${glob}`),
      ],
      dependencyCrates: suite.dependencyCrates,
      notes: suite.description,
      exception: "",
    };
    if (suite.name === "provider-runtime") {
      entry.exception = "Current suite still mixes offline/runtime coverage with live-provider canary behavior; split by world before making it a default blocking target.";
    }
    if (suite.name === "sandbox-cloud") {
      entry.exception = "Current suite spans sandbox/runtime behavior and external-service truth; keep it out of fast blocking profiles until split.";
    }
    entries.push(entry);
  }
  return entries;
}

function buildWebE2EEntries() {
  const suiteMap = readWebSuiteMap();
  const entries = [];
  const suiteMetadata = {
    premerge_required: {
      cost: "fast",
      stability: "stable",
      world: "simulated",
      execution: "bazel-addressable",
    },
    release_required: {
      cost: "medium",
      stability: "stable",
      world: "local-packaged-artifact",
      execution: "bazel-addressable",
    },
    cross_platform: {
      cost: "medium",
      stability: "stable",
      world: "simulated",
      execution: "bazel-addressable",
    },
    visual: {
      cost: "slow",
      stability: "quarantined",
      world: "simulated",
      execution: "bazel-addressable",
    },
    soak: {
      cost: "soak",
      stability: "quarantined",
      world: "simulated",
      execution: "bazel-addressable",
    },
    load: {
      cost: "slow",
      stability: "stable",
      world: "simulated",
      execution: "bazel-addressable",
    },
  };

  for (const [suite, specs] of suiteMap.entries()) {
    for (const spec of specs) {
      const meta = suiteMetadata[suite];
      entries.push({
        id: `web-e2e.${spec.replace(/^e2e\//u, "").replace(/\.spec\.ts$/u, "").replace(/[/.]/gu, "-")}`,
        title: `web-e2e: ${path.basename(spec)}`,
        family: classifyWebSpecFamily(spec),
        entrypointType: "web-e2e-spec",
        entrypoint: `core/apps/web/${spec}`,
        suite,
        surface: "system",
        oracle: "golden-flow",
        world: meta.world,
        cost: meta.cost,
        requirements: ["linux", "browser"],
        stability: meta.stability,
        execution: meta.execution,
        owner: "web-workbench",
        sourceGlobs: [
          `core/apps/web/${spec}`,
          `core/apps/web/e2e/suites/${suite}.txt`,
          "core/apps/web/scripts/run-e2e-suite.mjs",
        ],
        dependencyCrates: [],
        notes: `Primary suite: ${suite}`,
        exception: "",
      });
    }
  }

  return entries;
}

function buildProviderMatrixEntries() {
  const cells = Array.isArray(providerMatrix.cells) ? providerMatrix.cells : [];
  const requiredCount = cells.filter((cell) => cell.lane === "required" && cell.support === "supported").length;
  const nightlyCount = cells.filter((cell) => cell.lane === "nightly" && cell.support === "supported").length;
  return [
    {
      id: "provider-auth-matrix.required",
      title: "Provider auth matrix (required lane)",
      family: "provider-auth",
      entrypointType: "provider-matrix-lane",
      entrypoint: "core/apps/desktop/automation/fixtures/provider_auth_matrix.json#required",
      surface: "system",
      oracle: "golden-flow",
      world: "fake-provider",
      cost: "medium",
      requirements: ["mac", "browser", "network", "single-mac"],
      stability: "stable",
      execution: "script-local",
      owner: "provider-auth",
      sourceGlobs: [
        "core/apps/desktop/automation/fixtures/provider_auth_matrix.json",
        "core/apps/desktop/scripts/run_provider_auth_matrix.sh",
      ],
      dependencyCrates: [],
      notes: `${requiredCount} supported required cells currently live in the matrix.`,
      exception: "",
    },
    {
      id: "provider-auth-matrix.nightly",
      title: "Provider auth matrix (nightly lane)",
      family: "provider-auth",
      entrypointType: "provider-matrix-lane",
      entrypoint: "core/apps/desktop/automation/fixtures/provider_auth_matrix.json#nightly",
      surface: "system",
      oracle: "golden-flow",
      world: "live-provider",
      cost: "slow",
      requirements: ["mac", "browser", "network", "single-mac", "long-running"],
      stability: "quarantined",
      execution: "script-local",
      owner: "provider-auth",
      sourceGlobs: [
        "core/apps/desktop/automation/fixtures/provider_auth_matrix.json",
        "core/apps/desktop/scripts/run_provider_auth_matrix.sh",
      ],
      dependencyCrates: [],
      notes: `${nightlyCount} supported nightly cells currently live in the matrix.`,
      exception: "Wide live-provider coverage is intentionally excluded from fast blocking profiles and should stay nightlies-only until split and stabilized.",
    },
  ];
}

function buildStaticEntries() {
  return [
    {
      id: "repo-contracts.buildkite-pipeline",
      title: "Buildkite pipeline contracts",
      family: "repo-contracts",
      entrypointType: "file",
      entrypoint: "core/scripts/buildkite_pipeline_contract.test.cjs",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "tiny",
      requirements: ["linux"],
      stability: "stable",
      execution: "script-local",
      owner: "ci-release",
      sourceGlobs: [
        "core/scripts/buildkite_pipeline_contract.test.cjs",
        ".buildkite/pipelines/*.yml",
      ],
      dependencyCrates: [],
      notes: "Pipeline and orchestration contract coverage.",
      exception: "",
    },
    {
      id: "build-graph.release-bundle-contracts",
      title: "Release bundle Bazel contracts",
      family: "build-graph",
      entrypointType: "file",
      entrypoint: "core/scripts/release_bundle_contracts_bazel_contract.test.cjs",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "tiny",
      requirements: ["linux", "buildbuddy-rbe"],
      stability: "stable",
      execution: "bazel-rbe-preferred",
      owner: "ci-release",
      sourceGlobs: [
        "core/scripts/release_bundle_contracts_bazel_contract.test.cjs",
        "core/tools/bazel/**",
      ],
      dependencyCrates: [],
      notes: "Bazel-side release bundle and artifact contract coverage.",
      exception: "",
    },
    {
      id: "distribution-install.install-site-contracts",
      title: "Install-site Bazel contracts",
      family: "distribution-install",
      entrypointType: "file",
      entrypoint: "core/scripts/install_site_bazel_contract.test.cjs",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "tiny",
      requirements: ["linux", "buildbuddy-rbe"],
      stability: "stable",
      execution: "bazel-rbe-preferred",
      owner: "distribution",
      sourceGlobs: [
        "core/scripts/install_site_bazel_contract.test.cjs",
        "install-site/**",
      ],
      dependencyCrates: [],
      notes: "Install-site contract coverage.",
      exception: "",
    },
    {
      id: "distribution-install.desktop-runtime-lock",
      title: "Desktop runtime lock contracts",
      family: "distribution-install",
      entrypointType: "core-package-script",
      entrypoint: "desktop:runtime:lock:test",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "tiny",
      requirements: ["linux"],
      stability: "stable",
      execution: "script-local",
      owner: "distribution",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/testing_tiers_contract.test.cjs",
      ],
      dependencyCrates: [],
      notes: "Runtime lock validation suite used by release-facing contract checks.",
      exception: "",
    },
    {
      id: "web-workbench.web-unit-tests",
      title: "Web unit tests",
      family: "web-workbench",
      entrypointType: "core-package-script",
      entrypoint: "bazel:web:test",
      surface: "unit",
      oracle: "direct-assertion",
      world: "hermetic",
      cost: "fast",
      requirements: ["linux", "buildbuddy-rbe"],
      stability: "stable",
      execution: "bazel-rbe-preferred",
      owner: "web-workbench",
      sourceGlobs: [
        "core/apps/web/src/**",
        "core/apps/web/package.json",
        "core/apps/web/scripts/**",
      ],
      dependencyCrates: [],
      notes: "Primary hermetic web unit and component gate.",
      exception: "",
      alwaysOnProfiles: ["checkin"],
    },
    {
      id: "web-workbench.web-premerge-required",
      title: "Web premerge required suite",
      family: "web-workbench",
      entrypointType: "core-package-script",
      entrypoint: "verify:e2e",
      surface: "system",
      oracle: "golden-flow",
      world: "simulated",
      cost: "fast",
      requirements: ["linux", "browser"],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "web-workbench",
      sourceGlobs: [
        "core/apps/web/src/state/**",
        "core/apps/web/src/api/**",
        "core/apps/web/e2e/**",
        "core/apps/web/e2e/suites/premerge_required.txt",
      ],
      dependencyCrates: [],
      notes: "Current premerge-required browser suite.",
      exception: "",
    },
    {
      id: "desktop-shell.image-automation",
      title: "Desktop image automation",
      family: "desktop-shell",
      entrypointType: "core-package-script",
      entrypoint: "verify:desktop:images",
      surface: "system",
      oracle: "golden-flow",
      world: "simulated",
      cost: "medium",
      requirements: ["mac", "single-mac"],
      stability: "stable",
      execution: "script-local",
      owner: "desktop",
      sourceGlobs: [
        "core/package.json",
        "core/apps/desktop/automation/**",
      ],
      dependencyCrates: [],
      notes: "Current desktop image automation gate.",
      exception: "",
    },
    {
      id: "distribution-install.desktop-smoke",
      title: "Desktop smoke",
      family: "distribution-install",
      entrypointType: "core-package-script",
      entrypoint: "verify:desktop-smoke",
      surface: "system",
      oracle: "golden-flow",
      world: "local-packaged-artifact",
      cost: "medium",
      requirements: ["mac", "network", "single-mac"],
      stability: "stable",
      execution: "script-local",
      owner: "desktop",
      sourceGlobs: [
        "core/package.json",
        "scripts/desktop_smoke_with_infisical.sh",
      ],
      dependencyCrates: [],
      notes: "Desktop/local install smoke path.",
      exception: "",
    },
    {
      id: "updates-release.updater-native-smoke",
      title: "Updater native smoke",
      family: "updates-release",
      entrypointType: "core-package-script",
      entrypoint: "verify:e2e:updater:smoke:native",
      surface: "artifact",
      oracle: "artifact-integrity",
      world: "local-packaged-artifact",
      cost: "medium",
      requirements: ["mac", "network", "single-mac"],
      stability: "stable",
      execution: "artifact-tail",
      owner: "updates-release",
      sourceGlobs: [
        "core/package.json",
        "scripts/tests/release_manifest_concurrency_smoke.sh",
        "scripts/tests/release_promote_supabase_latest_smoke.sh",
        "scripts/tests/updater_contract_validate_smoke.sh",
        "scripts/tests/release_verify_supabase_updater_smoke.sh",
        ...RELEASE_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Release/updater native smoke and manifest integrity checks.",
      exception: "",
    },
    {
      id: "artifacts-provenance.codex-archive-artifacts",
      title: "Codex archive artifact gate",
      family: "artifacts-provenance",
      entrypointType: "core-package-script",
      entrypoint: "test:bundles:codex-archive-artifacts",
      surface: "artifact",
      oracle: "artifact-integrity",
      world: "local-packaged-artifact",
      cost: "fast",
      requirements: ["linux"],
      stability: "stable",
      execution: "script-local",
      owner: "ci-release",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/provider_matrix_archive_artifact_gate.cjs",
        ...RELEASE_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Archive artifact presence and merge integrity gate.",
      exception: "",
    },
    {
      id: "artifacts-provenance.codex-provenance",
      title: "Codex provenance gate",
      family: "artifacts-provenance",
      entrypointType: "core-package-script",
      entrypoint: "test:bundles:codex-provenance",
      surface: "artifact",
      oracle: "artifact-integrity",
      world: "local-packaged-artifact",
      cost: "fast",
      requirements: ["linux"],
      stability: "stable",
      execution: "script-local",
      owner: "ci-release",
      sourceGlobs: [
        "core/package.json",
        "scripts/tests/codex_release_provenance_policy.sh",
        ...RELEASE_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Provenance policy enforcement for bundled artifacts.",
      exception: "",
    },
    {
      id: "updates-release.release-e2e-publish",
      title: "Release E2E publish",
      family: "updates-release",
      entrypointType: "core-package-script",
      entrypoint: "release:e2e:publish",
      surface: "promotion",
      oracle: "live-publish",
      world: "published-artifact",
      cost: "slow",
      requirements: ["network", "single-mac"],
      stability: "stable",
      execution: "artifact-tail",
      owner: "updates-release",
      sourceGlobs: [
        "core/package.json",
        "scripts/release_e2e_with_infisical.sh",
        ...RELEASE_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Live publish path for release E2E.",
      exception: "",
    },
    {
      id: "updates-release.release-e2e-verify",
      title: "Release E2E verify",
      family: "updates-release",
      entrypointType: "core-package-script",
      entrypoint: "release:e2e:verify",
      surface: "promotion",
      oracle: "live-publish",
      world: "published-artifact",
      cost: "medium",
      requirements: ["network"],
      stability: "stable",
      execution: "artifact-tail",
      owner: "updates-release",
      sourceGlobs: [
        "core/package.json",
        "scripts/release_e2e_with_infisical.sh",
        ...RELEASE_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Published artifact verification path.",
      exception: "",
    },
    {
      id: "resilience-performance.anomaly",
      title: "Anomaly suite",
      family: "resilience-performance",
      entrypointType: "core-package-script",
      entrypoint: "verify:anomaly",
      surface: "resilience",
      oracle: "fault-injection",
      world: "simulated",
      cost: "slow",
      requirements: ["linux", "long-running"],
      stability: "stable",
      execution: "script-local",
      owner: "reliability",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/run-anomaly-suite.sh",
      ],
      dependencyCrates: [],
      notes: "Anomaly and failure-path suite.",
      exception: "",
    },
    {
      id: "resilience-performance.fuzz-regression",
      title: "Fuzz regression suite",
      family: "resilience-performance",
      entrypointType: "core-package-script",
      entrypoint: "verify:fuzz:regression",
      surface: "adversarial",
      oracle: "fault-injection",
      world: "hermetic",
      cost: "slow",
      requirements: ["linux", "long-running"],
      stability: "stable",
      execution: "script-local",
      owner: "reliability",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/run-fuzz-regression.sh",
      ],
      dependencyCrates: [],
      notes: "Fuzz regression coverage.",
      exception: "",
    },
  ];
}

function getCorePackageScripts() {
  return new Set(Object.keys(packageJson.scripts || {}));
}

function validateEntrypoint(entry) {
  switch (entry.entrypointType) {
    case "file": {
      const abs = path.join(repoRoot, entry.entrypoint);
      if (!fs.existsSync(abs)) {
        throw new Error(`missing file entrypoint for ${entry.id}: ${entry.entrypoint}`);
      }
      return;
    }
    case "core-package-script": {
      if (!getCorePackageScripts().has(entry.entrypoint)) {
        throw new Error(`missing core package script for ${entry.id}: ${entry.entrypoint}`);
      }
      return;
    }
    case "ctx-http-suite": {
      if (!CTX_HTTP_SUITES.some((suite) => suite.name === entry.entrypoint)) {
        throw new Error(`missing ctx-http suite for ${entry.id}: ${entry.entrypoint}`);
      }
      return;
    }
    case "provider-matrix-lane": {
      const [, lane] = entry.entrypoint.split("#");
      if (!lane || !providerMatrix.cells.some((cell) => cell.lane === lane)) {
        throw new Error(`missing provider matrix lane for ${entry.id}: ${entry.entrypoint}`);
      }
      return;
    }
    case "web-e2e-spec": {
      const abs = path.join(repoRoot, entry.entrypoint);
      if (!fs.existsSync(abs)) {
        throw new Error(`missing web e2e spec for ${entry.id}: ${entry.entrypoint}`);
      }
      return;
    }
    default:
      throw new Error(`unknown entrypoint type for ${entry.id}: ${entry.entrypointType}`);
  }
}

function buildTaxonomyRegistry() {
  const familiesById = getFamiliesById();
  const entries = sortEntries([
    ...buildStaticEntries(),
    ...buildCtxHttpEntries(),
    ...buildProviderMatrixEntries(),
    ...buildWebE2EEntries(),
  ].map((entry) => validateEntry(entry, familiesById)));

  const ids = new Set();
  for (const entry of entries) {
    if (ids.has(entry.id)) {
      throw new Error(`duplicate taxonomy entry id: ${entry.id}`);
    }
    ids.add(entry.id);
  }
  return entries;
}

function validateTaxonomyRegistry(entries = buildTaxonomyRegistry()) {
  for (const entry of entries) {
    validateEntrypoint(entry);
  }

  const ctxHttpValidation = validateCtxHttpSuites(coreRoot);
  if (ctxHttpValidation.duplicates.length > 0 || ctxHttpValidation.missing.length > 0 || ctxHttpValidation.unknown.length > 0) {
    throw new Error(`ctx-http suite inventory invalid: ${JSON.stringify(ctxHttpValidation)}`);
  }

  const suiteMap = readWebSuiteMap();
  const allAssignedSpecs = new Set();
  for (const specs of suiteMap.values()) {
    for (const spec of specs) {
      allAssignedSpecs.add(spec);
    }
  }
  const actualSpecs = fs
    .readdirSync(webE2ERoot)
    .filter((entry) => entry.endsWith(".spec.ts"))
    .map((entry) => `e2e/${entry}`);
  for (const spec of actualSpecs) {
    if (!allAssignedSpecs.has(spec)) {
      throw new Error(`web e2e spec missing taxonomy-backed suite assignment: ${spec}`);
    }
  }

  return {
    entryCount: entries.length,
    familyCount: FAMILIES.length,
  };
}

module.exports = {
  FAMILIES,
  buildTaxonomyRegistry,
  getCorePackageScripts,
  providerMatrix,
  readWebSuiteMap,
  validateTaxonomyRegistry,
  webSuites,
};
