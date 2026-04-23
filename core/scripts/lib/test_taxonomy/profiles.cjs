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
      "pnpm -C core test:agent:minimal",
      "pnpm -C core test:agent:minimal:linux-rbe",
      "node core/scripts/run_test_taxonomy_profile.cjs --profile agent-minimal --touched-only --run --changed-file <repo-relative-path>",
    ],
    pipelines: ["local-agent-loop"],
    remoteStrategy: "Prefer Linux RBE-backed Bazel and crate-level tasks; this profile excludes Mac and live-service requirements.",
    currentExecution: "This profile remains the low-level narrow compile-and-contract floor; the public agent workflow now routes through verify:touched and verify:affected instead of asking agents to pick raw profile commands.",
    expansionRules: [
      "Path-gated tiny browser or fake-provider checks may be added on top when change risk justifies them.",
    ],
  },
  {
    id: "agent-default",
    title: "Agent Default",
    purpose: "Standard agent confidence loop for normal changes before pushing.",
    selector: {
      includeSurfaces: ["contract", "compile", "unit", "integration"],
      includeWorlds: ["hermetic", "simulated", "fake-provider"],
      includeCosts: ["tiny", "fast"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-rbe-preferred", "bazel-addressable", "script-local"],
      excludeRequirements: ["browser", "mac", "network", "single-mac", "long-running"],
    },
    currentCommands: [
      "pnpm -C core verify:touched",
      "pnpm -C core verify:touched:linux-rbe",
      "pnpm -C core verify:affected",
      "pnpm -C core verify:affected:linux-rbe",
    ],
    pipelines: ["local-agent-loop"],
    remoteStrategy: "Maximize Bazel-addressable and RBE-friendly work; escalate only to the touched families instead of broad megajobs.",
    currentExecution: "The public agent loop now routes through the verification router, which resolves changed files, applies global invariants, and then runs the taxonomy-backed affected plan for this profile.",
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
      "pnpm -C core verify:broader",
      "pnpm -C core verify:broader:linux-rbe",
    ],
    pipelines: ["local-agent-loop", "agent-remote-loop"],
    remoteStrategy: "Keep the expensive hermetic work on Linux/Bazel; only consume local packaged artifacts or browser flows when the change actually needs them.",
    currentExecution: "The public broader confidence pass now routes through verify:broader, which keeps selection changed-aware while allowing medium-cost path escalations within this profile.",
    expansionRules: [
      "Live-provider and published-artifact checks are still out of scope unless explicitly forced.",
    ],
  },
  {
    id: "checkin",
    title: "Checkin",
    purpose: "Exact-SHA post-push confidence for direct-to-main.",
    selector: {
      includeEntryIds: [
        "distribution-install.install-bootstrap-contracts",
        "provider-auth.provider-auth-validate",
        "build-graph.release-bundle-contracts-linux-x86_64",
        "distribution-install.desktop-runtime-lock-matrix",
        "distribution-install.desktop-runtime-lock-validate",
        "distribution-install.desktop-version-check",
      ],
      includeSurfaces: ["contract"],
      includeWorlds: ["hermetic"],
      includeCosts: ["tiny", "fast"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-addressable"],
    },
    currentCommands: [
      "Buildkite pipeline: ctx-main",
      "pnpm -C core testing:profile:run --profile install-bootstrap-contracts",
      "pnpm -C core testing:profile:run --profile provider-auth-validate",
      "pnpm -C core testing:profile:run --profile release-contracts",
    ],
    pipelines: ["ctx-main"],
    remoteStrategy: "Run deterministic Linux contract checks; the Mac install/bootstrap contract is path-gated.",
    currentExecution: "This profile now matches the checked-in ctx-main step graph exactly instead of describing a broader not-yet-wired exact-SHA lane.",
    expansionRules: [
      "If ctx-main grows beyond these contract gates, update this profile in the same change so the pipeline/profile mapping stays honest.",
    ],
  },
  {
    id: "mac-preview",
    title: "Mac Preview",
    purpose: "Build a fast signed Apple Silicon preview app on push to main without DMG, notarization, or publish work.",
    selector: {
      includeEntryIds: [
        "updates-release.mac-preview-macos-arm64",
      ],
      includeSurfaces: ["artifact"],
      includeWorlds: ["local-packaged-artifact"],
      includeCosts: ["medium", "slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["script-local"],
    },
    currentCommands: [
      "Buildkite pipeline: ctx-mac-preview",
      "pnpm -C core testing:profile:run --profile mac-preview",
    ],
    pipelines: ["ctx-mac-preview"],
    remoteStrategy: "Path-gate on Linux before building and validating the signed arm64 app on macOS.",
    currentExecution: "The pipeline uploads the Mac build step after a touched-files gate and produces a signed .app artifact.",
    expansionRules: [
      "Do not add DMG creation, notarization, updater packaging, or publish/promote work to this profile.",
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
      includeExecutions: ["bazel-addressable"],
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
    id: "desktop-system-parity-contracts",
    title: "Desktop System Parity Contracts",
    purpose: "Run the residual source-tree contract groups that the Linux desktop system parity lane still needs after runtime-lock wrapper removal.",
    selector: {
      includeEntryIds: [
        "distribution-install.desktop-runtime-lock-matrix",
        "distribution-install.desktop-version-check",
        "distribution-install.provider-matrix-archive-contracts",
        "desktop-shell.desktop-launch-mode-contracts",
        "distribution-install.desktop-bundle-contracts",
        "distribution-install.bundled-harness-dependency-contracts",
        "provider-auth.provider-auth-validate",
        "provider-auth.desktop-e2e-preflight-contracts",
        "sandbox-runtime.desktop-sync-resources-contracts",
        "provider-runtime.providers-e2e-bundle-contracts",
        "provider-runtime.linux-arm-provider-contracts",
        "toolchain-bootstrap.tauri-tools-lock-contracts",
        "toolchain-bootstrap.install-desktop-deps-contracts",
      ],
      includeSurfaces: ["contract"],
      includeWorlds: ["hermetic"],
      includeCosts: ["tiny", "fast"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-addressable"],
      excludeRequirements: ["mac", "single-mac", "long-running"],
    },
    currentCommands: [
      "BuildBuddy step: desktop-system-parity",
      "pnpm -C core testing:profile:run --profile desktop-system-parity-contracts",
    ],
    pipelines: ["buildbuddy-manual"],
    remoteStrategy: "Keep the parity lane Linux-only and contract-focused by composing first-class Bazel-backed contract groups instead of a cross-family runtime-lock wrapper.",
    currentExecution: "Runs the residual desktop parity contract groups directly after the runtime-lock umbrella was removed.",
    expansionRules: [
      "Do not add bundle staging, runtime prepare, or Docker cleanup here; this profile is only for the deterministic source-tree contract residue.",
    ],
  },
  {
    id: "releasetest",
    title: "Release Test",
    purpose: "Prove a pinned SHA is publishable using artifact-first staging and validation.",
    selector: {
      excludeEntryIds: [
        "updates-release.stable-promotion",
      ],
      forceIncludeEntryIds: [
        "updates-release.release-finalize",
      ],
      includeEntryIds: [
        "build-graph.release-bundle-contracts-linux-x86_64",
        "distribution-install.desktop-runtime-lock-matrix",
        "distribution-install.desktop-runtime-lock-validate",
        "distribution-install.desktop-version-check",
        "updates-release.updater-web-e2e-suite",
        "updates-release.updater-native-smoke",
        "updates-release.release-stage-linux-x64",
        "updates-release.release-stage-linux-arm64",
        "updates-release.release-stage-macos",
        "updates-release.release-finalize-macos",
        "updates-release.release-finalize",
      ],
      includeSurfaces: ["contract", "system", "artifact"],
      includeWorlds: ["hermetic", "local-packaged-artifact"],
      includeCosts: ["fast", "medium", "slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-addressable", "script-local", "artifact-tail"],
    },
    currentCommands: [
      "Buildkite pipeline: ctx-release",
      "pnpm -C core testing:profile:run --profile release-contracts",
      "pnpm -C core testing:profile:run --profile release-bundle-gate",
      "pnpm -C core testing:profile:run --profile release-updater-web-e2e",
      "pnpm -C core testing:profile:run --profile release-updater-smoke",
      "pnpm -C core testing:profile:run --profile release-stage-linux-x64",
      "pnpm -C core testing:profile:run --profile release-stage-linux-arm64",
      "pnpm -C core testing:profile:run --profile release-stage-macos",
      "pnpm -C core testing:profile:run --profile release-finalize-macos",
      "pnpm -C core testing:profile:run --profile release-finalize",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Push deterministic prep into Bazel-owned preflight entries first, then keep stage/finalize boundaries explicit and artifact-first on the release workers.",
    currentExecution: "This profile now describes the actual ctx-release dry-run graph end to end, including the Linux/mac stage and finalize boundaries.",
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
      includeExecutions: ["bazel-addressable"],
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
      includeExecutions: ["script-local", "bazel-addressable"],
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
    id: "release-updater-proof",
    title: "Release Updater Proof",
    purpose: "Prove published Linux app-update and remote-daemon update behavior against real artifacts and real services.",
    selector: {
      includeEntryIds: [
        "updates-release.updater-linux-proof",
        "updates-release.updater-remote-daemon-proof",
      ],
      includeSurfaces: ["promotion"],
      includeWorlds: ["external-service"],
      includeCosts: ["slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["artifact-tail"],
      excludeRequirements: ["mac", "single-mac"],
    },
    currentCommands: [
      "pnpm -C core testing:profile:run --profile release-updater-proof",
    ],
    pipelines: ["buildbuddy-manual"],
    remoteStrategy: "Run this on real Linux hosts against published artifacts; local bundle smoke and mocked updater UI are not substitutes for install/update truth.",
    currentExecution: "Direct profile for the real Linux updater proof lanes that canary-proof also consumes automatically through the promotion/external-service selector.",
    expansionRules: [
      "Keep these lanes artifact-first and real-host; do not rewrite them as hermetic mocks or pre-publish source-tree checks.",
    ],
  },
  {
    id: "release-stage-linux-x64",
    title: "Release Stage Linux X64",
    purpose: "Build the linux-x64 release stage archive and publish it as the step artifact boundary.",
    selector: {
      includeEntryIds: [
        "updates-release.release-stage-linux-x64",
      ],
      includeSurfaces: ["artifact"],
      includeWorlds: ["local-packaged-artifact"],
      includeCosts: ["slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["script-local"],
      excludeRequirements: ["mac", "single-mac"],
    },
    currentCommands: [
      "Buildkite step: Release stage linux-x64",
      "pnpm -C core testing:profile:run --profile release-stage-linux-x64",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Keep linux staging on the managed Linux queue and preserve the stage archive as the only interface to finalize.",
    currentExecution: "Uses a dedicated Buildkite wrapper script so the stage boundary is part of the taxonomy instead of hidden inline in the pipeline YAML.",
    expansionRules: [
      "This profile owns only the linux-x64 stage boundary and the matching artifact upload.",
    ],
  },
  {
    id: "release-stage-linux-arm64",
    title: "Release Stage Linux Arm64",
    purpose: "Build the linux-arm64 release stage archive and publish it as the step artifact boundary.",
    selector: {
      includeEntryIds: [
        "updates-release.release-stage-linux-arm64",
      ],
      includeSurfaces: ["artifact"],
      includeWorlds: ["local-packaged-artifact"],
      includeCosts: ["slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["script-local"],
      excludeRequirements: ["mac", "single-mac"],
    },
    currentCommands: [
      "Buildkite step: Release stage linux-arm64",
      "pnpm -C core testing:profile:run --profile release-stage-linux-arm64",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Keep arm64 staging isolated on the arm64 queue and preserve the stage archive as the only interface to finalize.",
    currentExecution: "Uses a dedicated Buildkite wrapper script so the arm64 stage boundary is taxonomy-governed too.",
    expansionRules: [
      "This profile owns only the linux-arm64 stage boundary and the matching artifact upload.",
    ],
  },
  {
    id: "release-stage-macos",
    title: "Release Stage Macos",
    purpose: "Build the combined macOS stage archives.",
    selector: {
      includeEntryIds: [
        "updates-release.release-stage-macos",
      ],
      includeSurfaces: ["artifact"],
      includeWorlds: ["local-packaged-artifact"],
      includeCosts: ["slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["script-local"],
    },
    currentCommands: [
      "Buildkite step: Release stage macOS",
      "pnpm -C core testing:profile:run --profile release-stage-macos",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Build combined macOS stage archives; preflight and publishing use separate profiles.",
    currentExecution: "Uses a dedicated Buildkite wrapper script so the combined Mac stage and artifact upload remain one explicit failure domain.",
    expansionRules: [
      "This profile owns only the combined Mac stage boundary and the matching artifact upload.",
    ],
  },
  {
    id: "release-finalize-macos",
    title: "Release Finalize Macos",
    purpose: "Finalize the staged macOS archives into notarized release artifacts.",
    selector: {
      includeEntryIds: [
        "updates-release.release-finalize-macos",
      ],
      includeSurfaces: ["artifact"],
      includeWorlds: ["local-packaged-artifact"],
      includeCosts: ["slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["artifact-tail"],
    },
    currentCommands: [
      "Buildkite step: Release finalize macOS",
      "pnpm -C core testing:profile:run --profile release-finalize-macos",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Consume staged macOS artifacts without rebuilding from source during finalize.",
    currentExecution: "Uses a dedicated Buildkite wrapper script that downloads staged mac artifacts, runs finalize, and re-uploads the finalized outputs.",
    expansionRules: [
      "This profile owns only the Mac finalize boundary and its artifact handoff.",
    ],
  },
  {
    id: "release-finalize",
    title: "Release Finalize",
    purpose: "Consume the staged platform artifacts, publish the versioned release state, and emit final evidence without rebuilding.",
    selector: {
      excludeEntryIds: [
        "updates-release.stable-promotion",
      ],
      forceIncludeEntryIds: [
        "updates-release.release-finalize",
      ],
      includeSurfaces: ["promotion"],
      includeWorlds: ["external-service"],
      includeCosts: ["medium"],
      includeStabilities: ["stable"],
      includeExecutions: ["artifact-tail"],
      excludeRequirements: ["mac", "single-mac"],
    },
    currentCommands: [
      "Buildkite step: Release finalize",
      "pnpm -C core testing:profile:run --profile release-finalize",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Finalize should consume stage artifacts only, publish versioned state, verify it, and emit release evidence on Linux without rebuilding.",
    currentExecution: "Uses a dedicated Buildkite wrapper script that downloads the platform stage artifacts and runs the publish/verify/promotion tail.",
    expansionRules: [
      "This profile owns only the final publish/verify boundary and must not absorb any source rebuild work.",
    ],
  },
  {
    id: "canary-proof",
    title: "Canary Proof",
    purpose: "Publish staged artifacts into an isolated release channel and verify the published result without rebuilding.",
    selector: {
      excludeEntryIds: [
        "updates-release.stable-promotion",
      ],
      forceIncludeEntryIds: [
        "updates-release.release-finalize",
      ],
      includeSurfaces: ["promotion"],
      includeWorlds: ["external-service"],
      includeCosts: ["medium", "slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["artifact-tail"],
    },
    currentCommands: [
      "Buildkite step: Release finalize (non-stable-publish modes)",
      "pnpm -C core testing:profile:run --profile canary-proof",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Consume the exact staged artifacts; keep publish and latest-manifest verification as the thin post-stage tail instead of re-running compile work.",
    currentExecution: "The checked-in ctx-release finalize step now dispatches this profile for canary, canary2, e2e, and stable dry-run modes.",
    expansionRules: [
      "Canary should never rebuild what releasetest already proved.",
    ],
  },
  {
    id: "stable-promotion",
    title: "Stable Promotion",
    purpose: "Promote exact canary-proven artifacts into stable without rebuilding.",
    selector: {
      excludeEntryIds: [
        "updates-release.release-finalize",
      ],
      forceIncludeEntryIds: [
        "updates-release.stable-promotion",
      ],
      includeSurfaces: ["promotion"],
      includeWorlds: ["external-service"],
      includeCosts: ["fast", "medium"],
      includeStabilities: ["stable"],
      includeExecutions: ["artifact-tail"],
    },
    currentCommands: [
      "Buildkite step: Stable artifact promotion",
      "pnpm -C core testing:profile:run --profile stable-promotion",
    ],
    pipelines: ["ctx-release"],
    remoteStrategy: "Promotion must verify the canary versioned manifests for the same source SHA, copy the exact artifact objects into stable storage, rewrite stable manifests, and move latest only.",
    currentExecution: "The checked-in ctx-release stable publish path skips staging/finalize jobs and runs only the stable artifact promotion wrapper.",
    expansionRules: [
      "This profile selects promotion entries; nightly-breadth is a separate profile.",
      "Stable must not rebuild desktop artifacts or recompute provider state; rerun canary if canary artifacts are not already proven.",
    ],
  },
  {
    id: "nightly-breadth",
    title: "Nightly Breadth",
    purpose: "Catch slow, broad, adversarial, or matrix-heavy failures outside the landing loop.",
    selector: {
      includeEntryIds: [
        "resilience-performance.anomaly.ctx-http-fault-matrix",
        "resilience-performance.anomaly.ctx-http-hot-endpoints-no-db",
        "resilience-performance.anomaly.ctx-store-fault-injection",
        "resilience-performance.fuzz.providers",
        "resilience-performance.fuzz.mcp",
        "resilience-performance.fuzz.workspace-payloads",
        "resilience-performance.fuzz.release-manifests",
        "resilience-performance.fuzz.desktop-ipc",
      ],
      includeSurfaces: ["resilience", "adversarial"],
      includeWorlds: ["hermetic", "simulated"],
      includeCosts: ["slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-addressable"],
    },
    currentCommands: [
      "pnpm -C core test:nightly:breadth",
    ],
    pipelines: ["ctx-nightly"],
    remoteStrategy: "The scheduled nightly-breadth selector includes stable, hermetic or simulated Bazel-addressable anomaly and fuzz entries.",
    currentExecution: "Canonical scheduled nightly breadth union; the checked-in nightly pipeline fans this explicit profile out into anomaly and fuzz slices plus separate benchmark evidence.",
    expansionRules: [
      "Quarantined Mac and live-provider checks are excluded from this profile.",
    ],
  },
  {
    id: "nightly-updater-proof",
    title: "Nightly Updater Proof",
    purpose: "Run scheduled Linux updater proof against published artifacts outside the landing loop.",
    selector: {
      includeEntryIds: [
        "updates-release.updater-linux-proof",
        "updates-release.updater-remote-daemon-proof",
      ],
      includeSurfaces: ["promotion"],
      includeWorlds: ["external-service"],
      includeCosts: ["slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["artifact-tail"],
      excludeRequirements: ["mac", "single-mac"],
    },
    currentCommands: [
      "pnpm -C core testing:profile:run --profile nightly-updater-proof",
    ],
    pipelines: ["ctx-nightly"],
    remoteStrategy: "Run on Linux using published artifacts, real providers, and live SSH hosts.",
    currentExecution: "Scheduled updater proof uses external services and is separate from nightly-breadth.",
    expansionRules: [
      "This external-service updater profile is separate from the hermetic and simulated nightly-breadth checks.",
    ],
  },
  {
    id: "nightly-linux-anomaly",
    title: "Nightly Linux Anomaly",
    purpose: "Run the Linux-only nightly anomaly subset.",
    selector: {
      includeEntryIds: [
        "resilience-performance.anomaly.ctx-http-fault-matrix",
        "resilience-performance.anomaly.ctx-http-hot-endpoints-no-db",
        "resilience-performance.anomaly.ctx-store-fault-injection",
      ],
      includeSurfaces: ["resilience"],
      includeWorlds: ["simulated"],
      includeCosts: ["slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-addressable"],
      excludeRequirements: ["mac", "browser", "network", "single-mac"],
    },
    currentCommands: [
      "Buildkite step: Nightly anomaly",
      "pnpm -C core testing:profile:run --profile nightly-linux-anomaly",
    ],
    pipelines: ["ctx-nightly"],
    remoteStrategy: "Run the scheduled Linux anomaly checks through Bazel-addressable entries.",
    currentExecution: "Runs the bounded Linux anomaly slice as one explicit nightly failure domain.",
    expansionRules: [
      "This selector excludes live-provider, browser, and Mac anomaly checks.",
    ],
  },
  {
    id: "nightly-linux-fuzz",
    title: "Nightly Linux Fuzz",
    purpose: "Run the Linux-only nightly fuzz regression subset.",
    selector: {
      includeEntryIds: [
        "resilience-performance.fuzz.providers",
        "resilience-performance.fuzz.mcp",
        "resilience-performance.fuzz.workspace-payloads",
        "resilience-performance.fuzz.release-manifests",
        "resilience-performance.fuzz.desktop-ipc",
      ],
      includeSurfaces: ["adversarial"],
      includeWorlds: ["hermetic"],
      includeCosts: ["slow"],
      includeStabilities: ["stable"],
      includeExecutions: ["bazel-addressable"],
      excludeRequirements: ["mac", "browser", "network", "single-mac"],
    },
    currentCommands: [
      "Buildkite step: Nightly fuzz regression",
      "pnpm -C core testing:profile:run --profile nightly-linux-fuzz",
    ],
    pipelines: ["ctx-nightly"],
    remoteStrategy: "Run the scheduled Linux fuzz regression checks through Bazel-addressable entries.",
    currentExecution: "Runs the bounded Linux fuzz regression slice as one explicit nightly failure domain.",
    expansionRules: [
      "This selector excludes live-provider, browser, and Mac fuzz checks.",
    ],
  },
  {
    id: "nightly-benchmark-evidence",
    title: "Nightly Benchmark Evidence",
    purpose: "Capture advisory nightly host-budget drift evidence without redefining it as a taxonomy family or blocking gate.",
    selector: {
      includeEntryIds: [
        "resilience-performance.agent-loop-main-band-benchmark",
      ],
      includeSurfaces: ["performance"],
      includeWorlds: ["hermetic"],
      includeCosts: ["medium"],
      includeStabilities: ["stable"],
      includeExecutions: ["script-local"],
      excludeRequirements: ["mac", "browser", "network", "single-mac"],
    },
    currentCommands: [
      "Buildkite step: Nightly benchmark evidence",
      "pnpm -C core testing:profile:run --profile nightly-benchmark-evidence",
    ],
    pipelines: ["ctx-nightly"],
    remoteStrategy: "Keep host-budget evidence on Linux and outside the main landing loop; the value is drift detection, not build graph reuse.",
    currentExecution: "Runs the checked-in main-band benchmark wrapper as a separate advisory nightly failure domain.",
    expansionRules: [
      "Benchmark drift evidence is advisory.",
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
