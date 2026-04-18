const { validateProfile } = require("./schema.cjs");

const PROFILES = [
  {
    id: "agent-minimal",
    title: "Agent Minimal",
    purpose: "Fast, mostly hermetic inner-loop confidence while editing.",
    selector: {
      includeSurfaces: ["contract", "compile"],
      includeWorlds: ["hermetic", "simulated"],
      includeCosts: ["tiny", "fast"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-rbe-preferred", "bazel-addressable", "script-local"],
      excludeRequirements: ["browser", "mac", "network", "single-mac", "long-running"],
    },
    currentCommands: [
      "pnpm -C core verify:agent:fast",
      "pnpm -C core verify:agent:fast:linux-rbe",
    ],
    pipelines: ["local-agent-loop"],
    remoteStrategy: "Prefer Linux RBE-backed Bazel and crate-level tasks; this profile excludes Mac and live-service requirements.",
    currentExecution: "Current entrypoint is the affected-tests fast gate, with RBE preferred on Linux hosts.",
    expansionRules: [
      "Path-gated tiny browser or fake-provider checks may be added on top when change risk justifies them.",
    ],
  },
  {
    id: "agent-default",
    title: "Agent Default",
    purpose: "Standard agent confidence loop for normal changes before pushing.",
    selector: {
      forceIncludeEntryIds: ["web-workbench.web-premerge-required"],
      includeSurfaces: ["contract", "compile", "unit", "integration"],
      includeWorlds: ["hermetic", "simulated", "fake-provider"],
      includeCosts: ["tiny", "fast"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-rbe-preferred", "bazel-addressable", "script-local"],
      excludeRequirements: ["browser", "mac", "network", "single-mac", "long-running"],
    },
    currentCommands: [
      "pnpm -C core verify:agent:fast",
      "pnpm -C core test:agent",
      "pnpm -C core test:agent:linux-rbe",
    ],
    pipelines: ["local-agent-loop"],
    remoteStrategy: "Maximize Bazel-addressable and RBE-friendly work; escalate only to the touched families instead of broad megajobs.",
    currentExecution: "Current path is affected-tests first, then the mixed Rust/web agent gate when risk rises.",
    expansionRules: [
      "Targeted browser or provider-auth smoke may be added for touched high-risk areas.",
    ],
  },
  {
    id: "agent-broader",
    title: "Agent Broader",
    purpose: "Voluntary broader validation for risky changes before or after a push.",
    selector: {
      includeSurfaces: ["contract", "compile", "unit", "integration", "system", "artifact"],
      includeWorlds: ["hermetic", "simulated", "fake-provider", "local-packaged-artifact"],
      includeCosts: ["tiny", "fast", "medium"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-rbe-preferred", "bazel-addressable", "script-local", "artifact-tail"],
      excludeRequirements: ["single-mac", "long-running"],
    },
    currentCommands: [
      "pnpm -C core test:agent",
      "pnpm -C core verify:e2e",
      "pnpm -C core verify:e2e:release",
    ],
    pipelines: ["local-agent-loop", "agent-remote-loop"],
    remoteStrategy: "Keep the expensive hermetic work on Linux/Bazel; only consume local packaged artifacts or browser flows when the change actually needs them.",
    currentExecution: "The broader loop uses the listed test:agent, verify:e2e, and verify:e2e:release commands.",
    expansionRules: [
      "Live-provider and published-artifact checks are still out of scope unless explicitly forced.",
    ],
  },
  {
    id: "checkin",
    title: "Checkin",
    purpose: "Exact-SHA post-push confidence for direct-to-main.",
    selector: {
      includeSurfaces: ["contract", "compile", "unit", "integration", "system", "artifact"],
      includeWorlds: ["hermetic", "simulated", "fake-provider", "local-packaged-artifact"],
      includeCosts: ["tiny", "fast", "medium"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-rbe-preferred", "bazel-addressable", "script-local", "artifact-tail"],
      excludeRequirements: ["single-mac", "long-running"],
    },
    currentCommands: [
      "Buildkite pipeline: ctx-main",
      "pnpm -C core verify:quick",
      "pnpm -C core verify:desktop:provider-auth-matrix:required",
    ],
    pipelines: ["ctx-main"],
    remoteStrategy: "Prefer Linux RBE-friendly contract, compile, unit, and integration checks; Mac checks are path-gated.",
    currentExecution: "Today ctx-main is intentionally tiny and only runs a narrow subset of this profile, with Mac checks added only for touched paths.",
    expansionRules: [
      "Mac-only checks stay path-gated even though they are part of checkin policy.",
    ],
  },
  {
    id: "install-bootstrap-contracts",
    title: "Install Bootstrap Contracts",
    purpose: "Direct install/bootstrap contract execution for the ctx-main linux and mac bootstrap lanes.",
    selector: {
      includeEntryIds: [
        "distribution-install.install-bootstrap-contracts",
      ],
      includeSurfaces: ["contract"],
      includeWorlds: ["hermetic"],
      includeCosts: ["fast"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-addressable"],
    },
    currentCommands: [
      "Buildkite step: Install bootstrap contracts",
      "pnpm -C core testing:profile:run --profile install-bootstrap-contracts",
    ],
    pipelines: ["ctx-main"],
    remoteStrategy: "Keep install/bootstrap validation tiny and Bazel-addressable so it can run cheaply on either linux or mac workers.",
    currentExecution: "Runs the install-site contract test bundle directly instead of a BuildBuddy-era wrapper.",
    expansionRules: [
      "Only install/bootstrap contract coverage belongs here; do not add unrelated release or provider checks.",
    ],
  },
  {
    id: "provider-auth-validate",
    title: "Provider Auth Validate",
    purpose: "Validate provider auth matrix and desktop secret-contract metadata on ctx-main.",
    selector: {
      includeEntryIds: [
        "provider-auth.provider-auth-validate",
      ],
      includeSurfaces: ["contract"],
      includeWorlds: ["hermetic"],
      includeCosts: ["tiny"],
      includeStabilities: ["stable"],
      includeExecutions: ["script-local"],
      excludeRequirements: ["mac", "single-mac"],
    },
    currentCommands: [
      "Buildkite step: Provider auth matrix (validate)",
      "pnpm -C core testing:profile:run --profile provider-auth-validate",
    ],
    pipelines: ["ctx-main"],
    remoteStrategy: "Run the deterministic metadata gate on Linux; Mac and single-Mac requirements are excluded.",
    currentExecution: "Runs the matrix-report and secret-contract checks directly instead of routing through the old provider-auth wrapper mode.",
    expansionRules: [
      "This profile is only for validation, not the required or nightly execution lanes.",
    ],
  },
  {
    id: "releasetest",
    title: "Release Test",
    purpose: "Prove a pinned SHA is publishable using artifact-first staging and validation.",
    selector: {
      includeSurfaces: ["system", "artifact", "promotion"],
      includeWorlds: ["simulated", "fake-provider", "local-packaged-artifact", "external-service"],
      includeCosts: ["fast", "medium", "slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-addressable", "script-local", "artifact-tail"],
    },
    currentCommands: [
      "Buildkite pipeline: ctx-release",
      "pnpm -C core verify:e2e:release",
      "pnpm -C core verify:e2e:updater:smoke:native",
      "pnpm -C core test:bundles:codex-archive-artifacts",
      "pnpm -C core test:bundles:codex-provenance",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Push all deterministic prep into Bazel-addressable stage inputs first, then keep the Mac and external tails narrowly serialized.",
    currentExecution: "The release flow uses shell/Tauri stages to build and validate artifacts.",
    expansionRules: [
      "Stage artifacts are the interface between jobs; finalize and publish steps must not rebuild from source.",
    ],
  },
  {
    id: "release-contracts",
    title: "Release Contracts",
    purpose: "Exact release-preflight contract checks before artifact staging begins.",
    selector: {
      includeEntryIds: [
        "build-graph.release-bundle-contracts-linux-x86_64",
        "distribution-install.desktop-runtime-lock-matrix",
        "distribution-install.desktop-runtime-lock-validate",
        "distribution-install.desktop-version-check",
      ],
      includeSurfaces: ["contract"],
      includeWorlds: ["hermetic"],
      includeCosts: ["tiny", "fast"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-addressable", "script-local"],
      excludeRequirements: ["browser", "mac", "single-mac", "long-running"],
    },
    currentCommands: [
      "Buildkite step: Release contracts",
      "pnpm -C core testing:profile:run --profile release-contracts",
    ],
    pipelines: ["ctx-main", "ctx-release"],
    remoteStrategy: "Keep release contract truth Linux-first and deterministic; use direct profile execution instead of a compatibility wrapper.",
    currentExecution: "Runs the exact desktop version, runtime lock, and bundle contract gates that current release preflight depends on.",
    expansionRules: [
      "This profile stays source-tree-only and must not grow stage/publish side effects.",
    ],
  },
  {
    id: "release-updater-web-e2e",
    title: "Release Updater Web E2E",
    purpose: "Run the release web/updater browser suite with developer-provided browser prerequisites.",
    selector: {
      includeEntryIds: [
        "updates-release.updater-web-e2e-suite",
      ],
      includeSurfaces: ["system"],
      includeWorlds: ["local-packaged-artifact"],
      includeCosts: ["medium"],
      includeStabilities: ["stable"],
      includeExecutions: ["script-local"],
      excludeRequirements: ["mac", "single-mac"],
    },
    currentCommands: [
      "Buildkite step: Release updater web e2e",
      "pnpm -C core testing:profile:run --profile release-updater-web-e2e",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Run the browser suite on Linux with the required browser dependencies installed.",
    currentExecution: "Runs the retry-wrapped Bazel release browser suite.",
    expansionRules: [
      "Keep release-specific updater browser coverage in this profile.",
    ],
  },
  {
    id: "release-updater-smoke",
    title: "Release Updater Smoke",
    purpose: "Run updater and native smoke validation against prepared release artifacts.",
    selector: {
      includeEntryIds: [
        "updates-release.updater-native-smoke",
      ],
      includeSurfaces: ["artifact"],
      includeWorlds: ["local-packaged-artifact"],
      includeCosts: ["medium"],
      includeStabilities: ["stable"],
      includeExecutions: ["script-local", "artifact-tail"],
      excludeRequirements: ["mac", "single-mac"],
    },
    currentCommands: [
      "Buildkite step: Release updater smoke",
      "pnpm -C core testing:profile:run --profile release-updater-smoke",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Run the updater checks on Linux with the required packaged artifacts available.",
    currentExecution: "Runs the product updater and native smoke checks through taxonomy execution.",
    expansionRules: [
      "Provide the required release artifacts before running the updater checks.",
    ],
  },
  {
    id: "canary-proof",
    title: "Canary Proof",
    purpose: "Validate published canary artifacts against live-but-safe release state.",
    selector: {
      includeSurfaces: ["artifact", "promotion"],
      includeWorlds: ["published-artifact", "external-service"],
      includeCosts: ["medium", "slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["artifact-tail", "script-local"],
    },
    currentCommands: [
      "Buildkite pipeline: ctx-release (canary mode)",
      "pnpm -C core release:e2e:publish",
      "pnpm -C core release:e2e:verify",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Consume the exact staged artifacts; keep live publish checks as a thin post-build tail instead of re-running compile work.",
    currentExecution: "Current canary execution is manual or agent-triggered and should remain artifact-only.",
    expansionRules: [
      "Canary should never rebuild what releasetest already proved.",
    ],
  },
  {
    id: "stable-promotion",
    title: "Stable Promotion",
    purpose: "Promote exact canary-proven artifacts without rebuilding.",
    selector: {
      includeSurfaces: ["promotion"],
      includeWorlds: ["published-artifact", "external-service"],
      includeCosts: ["fast", "medium"],
      includeStabilities: ["stable"],
      includeExecutions: ["artifact-tail"],
    },
    currentCommands: [
      "Buildkite pipeline: ctx-release (stable mode)",
      "pnpm -C core release:e2e:publish",
      "pnpm -C core release:e2e:verify",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Promotion should be metadata and channel movement only; all expensive build work must already be proven.",
    currentExecution: "Current stable logic still rides through the release pipeline mode switch and should keep reusing canary-proven artifacts.",
    expansionRules: [
      "This profile selects promotion entries; nightly-breadth is a separate profile.",
    ],
  },
  {
    id: "nightly-breadth",
    title: "Nightly Breadth",
    purpose: "Catch slow, broad, adversarial, or matrix-heavy failures outside the landing loop.",
    selector: {
      includeSurfaces: ["resilience", "performance", "adversarial", "system"],
      includeWorlds: ["hermetic", "simulated", "fake-provider", "local-packaged-artifact", "live-provider", "external-service"],
      includeCosts: ["medium", "slow", "soak"],
      includeStabilities: ["stable", "quarantined"],
      includeExecutions: ["bazel-addressable", "script-local", "artifact-tail"],
    },
    currentCommands: [
      "Buildkite pipeline: ctx-nightly",
      "pnpm -C core verify:nightly",
      "pnpm -C core verify:anomaly",
      "pnpm -C core verify:fuzz:regression",
    ],
    pipelines: ["ctx-nightly"],
    remoteStrategy: "Nightly can spend more time on broader matrices, but hermetic compile/integration pieces should still prefer Bazel and RBE where possible.",
    currentExecution: "Current nightly coverage is command-based and still mixes different families into broad umbrella lanes.",
    expansionRules: [
      "Nightly is allowed to exercise live-provider and wide-matrix flows that are intentionally excluded from the inner loop.",
    ],
  },
];

function getProfiles(familiesById) {
  return PROFILES.map((profile) => validateProfile({ ...profile }, familiesById));
}

function profileMatchesEntry(profile, entry) {
  const selector = profile.selector || {};
  if (selector.excludeEntryIds.includes(entry.id)) {
    return false;
  }
  if (selector.forceIncludeEntryIds.includes(entry.id)) {
    return true;
  }
  if (selector.includeEntryIds.length > 0 && !selector.includeEntryIds.includes(entry.id)) {
    return false;
  }
  if (selector.families.length > 0 && !selector.families.includes(entry.family)) {
    return false;
  }
  if (selector.includeSurfaces.length > 0 && !selector.includeSurfaces.includes(entry.surface)) {
    return false;
  }
  if (selector.includeWorlds.length > 0 && !selector.includeWorlds.includes(entry.world)) {
    return false;
  }
  if (selector.includeCosts.length > 0 && !selector.includeCosts.includes(entry.cost)) {
    return false;
  }
  if (selector.includeStabilities.length > 0 && !selector.includeStabilities.includes(entry.stability)) {
    return false;
  }
  if (selector.includeExecutions.length > 0 && !selector.includeExecutions.includes(entry.execution)) {
    return false;
  }
  for (const requirement of selector.excludeRequirements) {
    if (entry.requirements.includes(requirement)) {
      return false;
    }
  }
  return true;
}

module.exports = {
  getProfiles,
  profileMatchesEntry,
};
