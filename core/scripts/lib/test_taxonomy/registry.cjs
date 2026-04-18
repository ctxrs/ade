const fs = require("node:fs");
const path = require("node:path");

const {
  CTX_HTTP_SHARED_SOURCE_GLOBS,
  CTX_HTTP_SUITES,
  validateCtxHttpSuites,
} = require("../ctx_http_suites.cjs");
const {
  ROOT_RUST_INPUTS,
  buildWorkspaceGraph,
  getGateManagedCrates,
} = require("../rust_workspace_graph.cjs");
const {
  BAZEL_TEST_CRATES,
  SERIAL_CARGO_TEST_CRATES,
} = require("../rust_gate_plan.cjs");
const { FAMILIES, getFamiliesById } = require("./families.cjs");
const { sortEntries, validateEntry } = require("./schema.cjs");

const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");
const coreRoot = path.join(repoRoot, "core");
const rustWorkspaceGraph = buildWorkspaceGraph(coreRoot);
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
const RELEASE_CONTRACT_RELEVANT_GLOBS = [
  ".buildkite/pipelines/main.yml",
  ".buildkite/pipelines/release.yml",
  "core/apps/desktop/**",
  "core/scripts/desktop_check_versions.cjs",
  "core/scripts/desktop_set_version.cjs",
  "core/scripts/desktop_version.cjs",
  "core/scripts/release_version.cjs",
  "core/scripts/runtime_lock_*.cjs",
  "scripts/buildbuddy/run_release*.sh",
  "scripts/release_*.sh",
  "tools/bazel/**",
];
const PROVIDER_AUTH_VALIDATE_GLOBS = [
  ".buildkite/**",
  "buildbuddy.yaml",
  "core/apps/desktop/automation/**",
  "core/apps/desktop/scripts/run_provider_auth_matrix.sh",
  "core/package.json",
  "core/scripts/desktop_e2e_preflight.cjs",
  "core/scripts/desktop_e2e_secret_contract.cjs",
  "core/scripts/desktop_e2e_secret_contract_lib.cjs",
  "core/scripts/validate_provider_auth_matrix.cjs",
  "core/scripts/validate_provider_auth_matrix.test.cjs",
  "scripts/buildbuddy/run_provider_auth_matrix.sh",
];
const INSTALL_BOOTSTRAP_GLOBS = [
  ".buildkite/**",
  "buildbuddy.yaml",
  "install-site/**",
  "scripts/buildbuddy/run_install_bootstrap_contracts.sh",
];
const RUST_ROOT_SOURCE_GLOBS = ROOT_RUST_INPUTS.map((input) => `core/${input}`);
const RUST_FAMILY_BY_CRATE = {
  "codex-crp": "artifacts-provenance",
  "ctx-avf-linux-guest-agent": "sandbox-runtime",
  "ctx-avf-linux-runtime": "sandbox-runtime",
  "ctx-bundled-assets": "artifacts-provenance",
  "ctx-client": "web-workbench",
  "ctx-core": "build-graph",
  "ctx-desktop-ipc": "desktop-shell",
  "ctx-docs-mirror": "attachments-artifacts",
  "ctx-egress-proxy": "sandbox-runtime",
  "ctx-events": "workspace-stream",
  "ctx-execution-runtime": "sandbox-runtime",
  "ctx-fs": "repo-vcs",
  "ctx-harness-runtime": "sandbox-runtime",
  "ctx-harness-setup": "toolchain-bootstrap",
  "ctx-harness-sources": "toolchain-bootstrap",
  "ctx-linux-sandbox-runtime": "sandbox-runtime",
  "ctx-load-test": "resilience-performance",
  "ctx-lsp": "lsp-editing",
  "ctx-managed-installs": "distribution-install",
  "ctx-mcp": "subagents-orchestration",
  "ctx-merge-queue": "repo-vcs",
  "ctx-provider-accounts": "provider-auth",
  "ctx-provider-auth-import": "provider-auth",
  "ctx-provider-install": "provider-auth",
  "ctx-provider-matrix": "provider-auth",
  "ctx-provider-runtime": "provider-runtime",
  "ctx-providers": "provider-runtime",
  "ctx-runtime-assets": "distribution-install",
  "ctx-sandbox-container-runtime": "sandbox-runtime",
  "ctx-sandbox-contract": "sandbox-runtime",
  "ctx-sandbox-materialization": "sandbox-runtime",
  "ctx-session-tools": "turns-terminal",
  "ctx-storage-admission": "workspace-stream",
  "ctx-store": "workspace-stream",
  "ctx-transport-runtime": "subagents-orchestration",
  "ctx-tunnel-control-plane": "subagents-orchestration",
  "ctx-tunnel-relay": "subagents-orchestration",
  "ctx-tunnel-router": "subagents-orchestration",
  "ctx-worker-protocol": "subagents-orchestration",
  "ctx-worker-shim": "subagents-orchestration",
  "ctx-workspace-active-snapshot": "workspace-stream",
  "ctx-workspace-config": "settings-config",
  "ctx-workspace-container": "workspace-stream",
  "ctx-workspace-runtime": "workspace-stream",
  "ctx-workspace-services": "workspace-stream",
  "ctx-worktree-data-plane": "repo-vcs",
};

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
      "provider-runtime-simulated": "provider-runtime",
      "provider-runtime-live": "provider-runtime",
      "repo-vcs": "repo-vcs",
      lsp: "lsp-editing",
      "turns-terminal": "turns-terminal",
      "attachments-routing": "attachments-artifacts",
      "subagents-control": "subagents-orchestration",
      "subagents-local-runtime": "subagents-orchestration",
      "updates-release": "updates-release",
      "sandbox-runtime-simulated": "sandbox-runtime",
      "sandbox-runtime-container-e2e": "sandbox-runtime",
      "sandbox-runtime-resource-governance": "sandbox-runtime",
      "sandbox-runtime-memory-leak": "sandbox-runtime",
    };
    const worldBySuite = {
      base: "hermetic",
      "workspace-stream": "simulated",
      "provider-auth": "fake-provider",
      "provider-runtime-simulated": "simulated",
      "provider-runtime-live": "external-service",
      "repo-vcs": "simulated",
      lsp: "simulated",
      "turns-terminal": "simulated",
      "attachments-routing": "simulated",
      "subagents-control": "simulated",
      "subagents-local-runtime": "external-service",
      "updates-release": "simulated",
      "sandbox-runtime-simulated": "simulated",
      "sandbox-runtime-container-e2e": "external-service",
      "sandbox-runtime-resource-governance": "external-service",
      "sandbox-runtime-memory-leak": "external-service",
    };
    const surfaceBySuite = {
      base: "compile",
      "provider-runtime-live": "system",
      "subagents-local-runtime": "system",
      "sandbox-runtime-container-e2e": "system",
      "sandbox-runtime-resource-governance": "resilience",
      "sandbox-runtime-memory-leak": "performance",
    };
    const oracleBySuite = {
      base: "compiler",
      "provider-runtime-live": "golden-flow",
      "subagents-local-runtime": "golden-flow",
      "sandbox-runtime-container-e2e": "golden-flow",
      "sandbox-runtime-memory-leak": "budget",
    };
    const costBySuite = {
      base: "fast",
      "provider-runtime-live": "slow",
      "subagents-local-runtime": "slow",
      "sandbox-runtime-container-e2e": "medium",
      "sandbox-runtime-resource-governance": "medium",
      "sandbox-runtime-memory-leak": "soak",
    };
    const requirementsBySuite = {
      base: ["linux", "buildbuddy-rbe"],
      "workspace-stream": ["linux", "buildbuddy-rbe"],
      "provider-auth": ["linux", "buildbuddy-rbe"],
      "provider-runtime-simulated": ["linux", "buildbuddy-rbe"],
      "provider-runtime-live": ["linux", "buildbuddy-rbe", "network"],
      "repo-vcs": ["linux", "buildbuddy-rbe"],
      lsp: ["linux", "buildbuddy-rbe"],
      "turns-terminal": ["linux", "buildbuddy-rbe"],
      "attachments-routing": ["linux", "buildbuddy-rbe"],
      "subagents-control": ["linux", "buildbuddy-rbe"],
      "subagents-local-runtime": ["linux", "buildbuddy-rbe", "network"],
      "updates-release": ["linux", "buildbuddy-rbe"],
      "sandbox-runtime-simulated": ["linux", "buildbuddy-rbe"],
      "sandbox-runtime-container-e2e": ["linux", "buildbuddy-rbe", "docker", "network"],
      "sandbox-runtime-resource-governance": ["linux", "buildbuddy-rbe"],
      "sandbox-runtime-memory-leak": ["linux", "buildbuddy-rbe", "long-running"],
    };
    const stabilityBySuite = {
      base: "stable",
      "workspace-stream": "stable",
      "provider-auth": "stable",
      "provider-runtime-simulated": "stable",
      "provider-runtime-live": "quarantined",
      "repo-vcs": "stable",
      lsp: "stable",
      "turns-terminal": "stable",
      "attachments-routing": "stable",
      "subagents-control": "stable",
      "subagents-local-runtime": "quarantined",
      "updates-release": "stable",
      "sandbox-runtime-simulated": "stable",
      "sandbox-runtime-container-e2e": "quarantined",
      "sandbox-runtime-resource-governance": "quarantined",
      "sandbox-runtime-memory-leak": "quarantined",
    };
    const executionBySuite = {
      base: "bazel-rbe-preferred",
      "workspace-stream": "bazel-rbe-preferred",
      "provider-auth": "bazel-rbe-preferred",
      "provider-runtime-simulated": "bazel-rbe-preferred",
      "provider-runtime-live": "bazel-addressable",
      "repo-vcs": "bazel-rbe-preferred",
      lsp: "bazel-rbe-preferred",
      "turns-terminal": "bazel-rbe-preferred",
      "attachments-routing": "bazel-rbe-preferred",
      "subagents-control": "bazel-rbe-preferred",
      "subagents-local-runtime": "bazel-addressable",
      "updates-release": "bazel-addressable",
      "sandbox-runtime-simulated": "bazel-rbe-preferred",
      "sandbox-runtime-container-e2e": "bazel-addressable",
      "sandbox-runtime-resource-governance": "bazel-addressable",
      "sandbox-runtime-memory-leak": "bazel-addressable",
    };
    const world = worldBySuite[suite.name];
    const entry = {
      id: `ctx-http.${suite.name}`,
      title: `ctx-http: ${suite.name}`,
      family: familyBySuite[suite.name],
      entrypointType: "ctx-http-suite",
      entrypoint: suite.name,
      surface: surfaceBySuite[suite.name] || (suite.type === "base" ? "compile" : "integration"),
      oracle: oracleBySuite[suite.name] || (suite.type === "base" ? "compiler" : "direct-assertion"),
      world,
      cost: costBySuite[suite.name] || (suite.type === "base" ? "fast" : "fast"),
      requirements: requirementsBySuite[suite.name] || ["linux", "buildbuddy-rbe"],
      stability: stabilityBySuite[suite.name] || (world === "external-service" ? "quarantined" : "stable"),
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
  const nightlyCells = cells.filter((cell) => cell.lane === "nightly" && cell.support === "supported");
  const nightlyAuthModeCount = (authMode) => nightlyCells.filter((cell) => cell.auth_mode === authMode).length;
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
      id: "provider-auth-matrix.nightly.endpoint-write",
      title: "Provider auth matrix (nightly endpoint-write slice)",
      family: "provider-auth",
      entrypointType: "core-package-script",
      entrypoint: "verify:desktop:provider-auth-matrix:endpoint-write",
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
      notes: `${nightlyAuthModeCount("endpoint_api_key")} supported nightly endpoint-write cells currently live in the matrix.`,
      exception: "",
    },
    {
      id: "provider-auth-matrix.nightly.subscription-oauth",
      title: "Provider auth matrix (nightly subscription-oauth slice)",
      family: "provider-auth",
      entrypointType: "core-package-script",
      entrypoint: "verify:desktop:provider-auth-matrix:oauth-subscription",
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
      notes: `${nightlyAuthModeCount("subscription_oauth")} supported nightly subscription-oauth cells currently live in the matrix.`,
      exception: "",
    },
    {
      id: "provider-auth-matrix.nightly.auth-import",
      title: "Provider auth matrix (nightly auth-import slice)",
      family: "provider-auth",
      entrypointType: "core-package-script",
      entrypoint: "verify:desktop:provider-auth-matrix:auth-import",
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
      notes: `${nightlyAuthModeCount("auth_import")} supported nightly auth-import cells currently live in the matrix.`,
      exception: "",
    },
  ];
}

function buildRustEntries() {
  const crates = getGateManagedCrates(rustWorkspaceGraph).filter((crate) => crate.crateName !== "ctx-http");
  const rustGateCrates = crates.map((crate) => crate.crateName);
  const entries = [
    {
      id: "build-graph.rust-turbo-check",
      title: "Rust turbo check",
      family: "build-graph",
      entrypointType: "core-package-script",
      entrypoint: "rust:turbo:check",
      surface: "compile",
      oracle: "compiler",
      world: "hermetic",
      cost: "fast",
      requirements: ["linux"],
      stability: "stable",
      execution: "script-local",
      owner: "rust-workspace",
      sourceGlobs: RUST_ROOT_SOURCE_GLOBS,
      dependencyCrates: rustGateCrates,
      notes: "Shared Rust compile/task preflight for non-ctx-http workspace crates.",
      exception: "",
    },
  ];

  for (const crate of crates) {
    const family = RUST_FAMILY_BY_CRATE[crate.crateName];
    if (!family) {
      throw new Error(`missing taxonomy family mapping for Rust crate: ${crate.crateName}`);
    }
    const strategyNotes = [];
    if (BAZEL_TEST_CRATES.has(crate.crateName)) {
      strategyNotes.push("Bazel-covered Rust gate");
    } else if (SERIAL_CARGO_TEST_CRATES.has(crate.crateName)) {
      strategyNotes.push("Serial cargo-test Rust gate");
    } else {
      strategyNotes.push("Nextest-backed Rust gate");
    }
    entries.push({
      id: `${family}.rust-gate.${crate.crateName}`,
      title: `Rust gate: ${crate.crateName}`,
      family,
      entrypointType: "rust-crate-gate",
      entrypoint: crate.crateName,
      surface: "integration",
      oracle: "direct-assertion",
      world: "hermetic",
      cost: "fast",
      requirements: ["linux"],
      stability: "stable",
      execution: BAZEL_TEST_CRATES.has(crate.crateName) ? "bazel-rbe-preferred" : "script-local",
      owner: "rust-workspace",
      sourceGlobs: [`core/${crate.relDir}/**`],
      dependencyCrates: [crate.crateName],
      notes: strategyNotes.join(". "),
      exception: "",
    });
  }

  return entries;
}

function buildStaticEntries() {
  return [
    {
      id: "repo-contracts.buildkite-pipeline",
      title: "Buildkite pipeline contracts",
      family: "repo-contracts",
      entrypointType: "core-package-script",
      entrypoint: "bazel:buildkite:pipeline:test",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "tiny",
      requirements: ["linux"],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "ci-release",
      sourceGlobs: [
        "core/scripts/buildkite_pipeline_contract.test.cjs",
        ".buildkite/**",
        "scripts/buildkite/**",
        "scripts/ci/**",
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
      id: "build-graph.release-bundle-contracts-linux-x86_64",
      title: "Release bundle contracts (linux-x86_64)",
      family: "build-graph",
      entrypointType: "core-package-script",
      entrypoint: "release:bundle:contracts:linux-x86_64",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "fast",
      requirements: ["linux"],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "ci-release",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/release_bundle_contracts_bazel_contract.test.cjs",
        ...RELEASE_CONTRACT_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Executable release bundle contract gate used by release preflight.",
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
      id: "distribution-install.install-bootstrap-contracts",
      title: "Install bootstrap contracts",
      family: "distribution-install",
      entrypointType: "core-package-script",
      entrypoint: "install:bootstrap:contracts",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "fast",
      requirements: [],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "distribution",
      sourceGlobs: INSTALL_BOOTSTRAP_GLOBS,
      dependencyCrates: [],
      notes: "Bootstrap/install contract execution used by ctx-main linux and mac gate steps.",
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
      cost: "fast",
      requirements: ["linux"],
      stability: "stable",
      execution: "script-local",
      owner: "distribution",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/provider_matrix_archive_gap_report.test.cjs",
        "core/scripts/provider_matrix_required_targets_gate.test.cjs",
        "core/scripts/desktop_mode_contract.test.cjs",
        "core/scripts/desktop_prepare.test.cjs",
        "core/scripts/desktop_tauri_entry.test.cjs",
        "core/scripts/providers_e2e_bundle_isolation.test.cjs",
        "core/scripts/desktop_import_bundles.test.cjs",
        "core/scripts/desktop_normalize_bundle_permissions.test.cjs",
        "core/scripts/desktop_icon_reps.test.cjs",
        "core/scripts/linux_arm_provider_reliability_matrix.test.cjs",
        "core/scripts/linux_arm_provider_preflight.test.cjs",
        "core/scripts/linux_arm_provider_report_summary.test.cjs",
        "core/scripts/linux_arm_provider_release_gate.test.cjs",
        "scripts/tests/providers_e2e_darwin_codex_fallback_guard.sh",
        "scripts/tests/ensure_bundled_harnesses_droid_dependencies.sh",
      ],
      dependencyCrates: [],
      notes: "Legacy compatibility wrapper still consumed by desktop system parity; only the residual distribution/install parity checks remain here after the narrower provider-auth, sandbox-runtime, updates-release, and toolchain slices were split into subcommands.",
      exception: "Compatibility wrapper for run_desktop_system_parity.sh while the remaining parity residue is migrated to family-owned taxonomy entries.",
    },
    {
      id: "provider-auth.provider-auth-validate",
      title: "Provider auth matrix validation",
      family: "provider-auth",
      entrypointType: "core-package-script",
      entrypoint: "bazel:provider-auth:validate",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "tiny",
      requirements: ["linux"],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "provider-auth",
      sourceGlobs: PROVIDER_AUTH_VALIDATE_GLOBS,
      dependencyCrates: [],
      notes: "Provider auth matrix manifest and secret-contract validation used in ctx-main.",
      exception: "",
    },
    {
      id: "distribution-install.desktop-version-check",
      title: "Desktop version check",
      family: "distribution-install",
      entrypointType: "core-package-script",
      entrypoint: "bazel:desktop:check:versions",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "tiny",
      requirements: ["linux"],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "distribution",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/desktop_check_versions.cjs",
        ...RELEASE_CONTRACT_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Version alignment gate for release-facing desktop artifacts.",
      exception: "",
    },
    {
      id: "distribution-install.desktop-runtime-lock-matrix",
      title: "Desktop runtime lock matrix check",
      family: "distribution-install",
      entrypointType: "core-package-script",
      entrypoint: "bazel:desktop:runtime:lock:check-matrix",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "tiny",
      requirements: ["linux"],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "distribution",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/runtime_lock_matrix_consistency.cjs",
        ...RELEASE_CONTRACT_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Matrix consistency gate for desktop runtime lock metadata.",
      exception: "",
    },
    {
      id: "distribution-install.desktop-runtime-lock-validate",
      title: "Desktop runtime lock validate",
      family: "distribution-install",
      entrypointType: "core-package-script",
      entrypoint: "bazel:desktop:runtime:lock:validate",
      surface: "contract",
      oracle: "static-contract",
      world: "hermetic",
      cost: "tiny",
      requirements: ["linux"],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "distribution",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/runtime_lock_validate.cjs",
        ...RELEASE_CONTRACT_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Final runtime lock validation gate for release preflight.",
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
      id: "desktop-shell.image-paste",
      title: "Desktop image paste automation",
      family: "desktop-shell",
      entrypointType: "core-package-script",
      entrypoint: "verify:desktop:image-paste",
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
        "core/apps/desktop/package.json",
        "core/apps/desktop/automation/specs/workbench-image-paste.spec.cjs",
        "scripts/desktop_smoke_with_infisical.sh",
      ],
      dependencyCrates: [],
      notes: "Desktop image paste automation flow.",
      exception: "",
    },
    {
      id: "desktop-shell.image-drag-drop",
      title: "Desktop image drag/drop automation",
      family: "desktop-shell",
      entrypointType: "core-package-script",
      entrypoint: "verify:desktop:image-drag-drop",
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
        "core/apps/desktop/package.json",
        "core/apps/desktop/automation/specs/workbench-image-drag-drop.spec.cjs",
        "scripts/desktop_smoke_with_infisical.sh",
      ],
      dependencyCrates: [],
      notes: "Desktop image drag/drop automation flow.",
      exception: "",
    },
    {
      id: "distribution-install.desktop-smoke-local",
      title: "Desktop smoke local install",
      family: "distribution-install",
      entrypointType: "core-package-script",
      entrypoint: "verify:desktop:smoke:local",
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
      id: "distribution-install.desktop-remote-contracts",
      title: "Desktop remote docker contracts",
      family: "distribution-install",
      entrypointType: "core-package-script",
      entrypoint: "verify:desktop:remote-contracts",
      surface: "system",
      oracle: "golden-flow",
      world: "local-packaged-artifact",
      cost: "medium",
      requirements: ["mac", "docker", "network", "single-mac"],
      stability: "stable",
      execution: "script-local",
      owner: "desktop",
      sourceGlobs: [
        "core/package.json",
        "core/apps/desktop/package.json",
        "core/apps/desktop/scripts/test_remote_docker_contracts.sh",
      ],
      dependencyCrates: [],
      notes: "Desktop remote-contract Docker validation paired with local smoke.",
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
      requirements: ["linux", "network"],
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
      id: "updates-release.updater-web-e2e-suite",
      title: "Release updater web e2e suite",
      family: "updates-release",
      entrypointType: "core-package-script",
      entrypoint: "release:e2e:web:release:retry",
      surface: "system",
      oracle: "golden-flow",
      world: "local-packaged-artifact",
      cost: "medium",
      requirements: ["linux", "browser"],
      stability: "stable",
      execution: "script-local",
      owner: "updates-release",
      sourceGlobs: [
        "core/package.json",
        "scripts/ci_retry.sh",
        "core/apps/web/e2e/**",
        "core/apps/web/e2e/suites/release_required.txt",
        ...RELEASE_RELEVANT_GLOBS,
      ],
      dependencyCrates: [],
      notes: "Retry-wrapped release-required browser suite once Linux webkit prerequisites are present.",
      exception: "",
    },
    {
      id: "artifacts-provenance.codex-archive-artifacts",
      title: "Codex archive artifact gate",
      family: "artifacts-provenance",
      entrypointType: "core-package-script",
      entrypoint: "bazel:bundles:codex-archive-artifacts",
      surface: "artifact",
      oracle: "artifact-integrity",
      world: "local-packaged-artifact",
      cost: "fast",
      requirements: ["linux"],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "ci-release",
      sourceGlobs: [
        "core/package.json",
        "core/scripts/provider_matrix_archive_artifact_gate.cjs",
        "tools/bazel/codex_archive_artifact_gate.sh",
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
      entrypoint: "bazel:bundles:codex-provenance",
      surface: "artifact",
      oracle: "artifact-integrity",
      world: "local-packaged-artifact",
      cost: "fast",
      requirements: ["linux"],
      stability: "stable",
      execution: "bazel-addressable",
      owner: "ci-release",
      sourceGlobs: [
        "core/package.json",
        "scripts/tests/codex_release_provenance_policy.sh",
        "tools/bazel/codex_provenance_policy.sh",
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
    case "repo-shell-script": {
      const abs = path.join(repoRoot, entry.entrypoint);
      if (!fs.existsSync(abs)) {
        throw new Error(`missing repo shell script entrypoint for ${entry.id}: ${entry.entrypoint}`);
      }
      return;
    }
    case "ctx-http-suite": {
      if (!CTX_HTTP_SUITES.some((suite) => suite.name === entry.entrypoint)) {
        throw new Error(`missing ctx-http suite for ${entry.id}: ${entry.entrypoint}`);
      }
      return;
    }
    case "rust-crate-gate": {
      if (entry.entrypoint === "ctx-http") {
        throw new Error(`ctx-http must stay taxonomy-addressed via suite entries, not rust-crate-gate: ${entry.id}`);
      }
      if (!rustWorkspaceGraph.cratesByName.has(entry.entrypoint)) {
        throw new Error(`missing Rust crate gate for ${entry.id}: ${entry.entrypoint}`);
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
    ...buildRustEntries(),
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
