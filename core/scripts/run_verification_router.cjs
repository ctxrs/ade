#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { classifySourceFile } = require("./source_file_size_guard.cjs");
const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const {
  buildExecutionPlan,
  normalizeRepoRelativePath,
} = require("./lib/test_taxonomy/execution.cjs");
const {
  DEFAULT_BASE_REF,
  resolveIntentChangeSet,
} = require("./lib/verification_git_changes.cjs");
const { appendHostSample } = require("./lib/verification_host_sampler.cjs");
const {
  createRunArtifacts,
  finalizeRunArtifacts,
  readIndexedSummaries,
} = require("./lib/verification_run_store.cjs");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const MERGE_READY_COMMAND = "node scripts/run_test_taxonomy_profile.cjs --run --profile checkin";
const VERIFICATION_TOOLING_COMMAND = [
  "node",
  "--test",
  "scripts/verification_git_changes.test.cjs",
  "scripts/verification_router_contract.test.cjs",
  "scripts/verification_run_store.test.cjs",
  "scripts/sdlc_verify_metrics_report.test.cjs",
  "scripts/run_bazel_pilot.test.cjs",
  "scripts/testing_tiers_contract.test.cjs",
  "scripts/test_taxonomy_execution_contract.test.cjs",
  "scripts/affected_tests_contract.test.cjs",
].join(" ");

function parseArgs(argv) {
  const [intent = "", ...rest] = argv;
  const args = {
    base: DEFAULT_BASE_REF,
    changedFiles: [],
    dryRun: false,
    intent,
    json: false,
  };

  for (let index = 0; index < rest.length; index += 1) {
    const arg = rest[index];
    if (arg === "--base") {
      args.base = rest[index + 1] || "";
      index += 1;
    } else if (arg === "--changed-file") {
      args.changedFiles.push(rest[index + 1] || "");
      index += 1;
    } else if (arg === "--dry-run") {
      args.dryRun = true;
    } else if (arg === "--json") {
      args.json = true;
    } else if (arg === "-h" || arg === "--help") {
      usage(0);
    } else {
      throw new Error(`unknown arg: ${arg}`);
    }
  }

  if (!["touched", "affected", "broader", "merge-ready"].includes(args.intent)) {
    throw new Error("usage: run_verification_router.cjs <touched|affected|broader|merge-ready> [options]");
  }

  return args;
}

function usage(exitCode = 0) {
  const lines = [
    "usage: run_verification_router.cjs <touched|affected|broader|merge-ready> [options]",
    "",
    "options:",
    "  --base <git-ref>           base ref for affected/broader/merge-ready (default: origin/main)",
    "  --changed-file <path>      explicit repo-relative changed file; may be repeated",
    "  --dry-run                  print the selected command plan without executing",
    "  --json                     print plan JSON instead of shell lines",
  ];
  const stream = exitCode === 0 ? process.stdout : process.stderr;
  stream.write(`${lines.join("\n")}\n`);
  process.exit(exitCode);
}

function resolveIntentConfig(intent) {
  switch (intent) {
    case "touched":
      return {
        intent,
        entrypoint: "verify:touched",
        profileId: "agent-default",
      };
    case "affected":
      return {
        intent,
        entrypoint: "verify:affected",
        profileId: "agent-default",
      };
    case "broader":
      return {
        intent,
        entrypoint: "verify:broader",
        profileId: "agent-broader",
      };
    case "merge-ready":
      return {
        intent,
        entrypoint: "verify:merge-ready",
        profileId: "",
      };
    default:
      throw new Error(`unsupported verification intent: ${intent}`);
  }
}

function hasSupabaseMigrationChange(changedFiles) {
  return changedFiles.some((entry) => entry.startsWith("supabase/migrations/") && entry.endsWith(".sql"));
}

function hasSupabaseFunctionChange(changedFiles) {
  return changedFiles.some((entry) => entry.startsWith("supabase/functions/") && entry.endsWith(".ts"));
}

function hasProductionSourceChange(changedFiles) {
  return changedFiles.some((entry) => classifySourceFile(entry) === "production");
}

function dedupeCommands(commands) {
  return [...new Set(commands.filter(Boolean))];
}

function buildOverlayCommands(changedFiles) {
  const commands = [];
  if (hasProductionSourceChange(changedFiles)) {
    commands.push("pnpm source:file-size:enforce");
  }
  if (changedFiles.some((entry) =>
    entry.startsWith("core/scripts/run_verification_router")
    || entry.startsWith("core/scripts/sdlc_verify_metrics_report")
    || entry.startsWith("core/scripts/verification_")
    || entry.startsWith("core/scripts/run_bazel_pilot")
    || entry.startsWith("core/scripts/run_test_taxonomy_profile")
    || entry.startsWith("core/scripts/affected_tests")
    || entry.startsWith("core/scripts/lib/verification_")
    || entry.startsWith("core/scripts/lib/test_taxonomy/")
  )) {
    commands.push(VERIFICATION_TOOLING_COMMAND);
  }
  if (hasSupabaseMigrationChange(changedFiles)) {
    commands.push("pnpm supabase:migrations:check");
  }
  if (hasSupabaseFunctionChange(changedFiles)) {
    commands.push("pnpm supabase:functions:check");
  }
  return commands;
}

function buildVerificationPlan(args) {
  const config = resolveIntentConfig(args.intent);
  const changeSet = resolveIntentChangeSet({
    cwd: repoRoot,
    intent: config.intent,
    baseRef: args.base,
    changedFiles: args.changedFiles.map(normalizeRepoRelativePath),
  });

  if (config.intent === "merge-ready") {
    const overlayCommands = buildOverlayCommands(changeSet.changedFiles);
    return {
      ...config,
      baseRef: changeSet.baseRef,
      changedFiles: changeSet.changedFiles,
      mergeBase: changeSet.mergeBase,
      overlayCommands,
      taxonomyEntries: [],
      taxonomyCommands: [],
      commands: dedupeCommands([...overlayCommands, MERGE_READY_COMMAND]),
    };
  }

  if (changeSet.changedFiles.length === 0) {
    return {
      ...config,
      baseRef: changeSet.baseRef,
      changedFiles: [],
      mergeBase: changeSet.mergeBase,
      overlayCommands: [],
      taxonomyEntries: [],
      taxonomyCommands: [],
      commands: [],
    };
  }

  const taxonomyPlan = buildExecutionPlan({
    profileId: config.profileId,
    changedFiles: changeSet.changedFiles,
    touchedOnly: true,
  });
  const overlayCommands = buildOverlayCommands(changeSet.changedFiles);
  const commands = dedupeCommands([...overlayCommands, ...taxonomyPlan.commands]);

  return {
    ...config,
    baseRef: changeSet.baseRef,
    changedFiles: changeSet.changedFiles,
    mergeBase: changeSet.mergeBase,
    overlayCommands,
    taxonomyEntries: taxonomyPlan.selectedEntries.map((entry) => entry.id),
    taxonomyCommands: taxonomyPlan.commands,
    commands,
  };
}

function printTextPlan(plan) {
  if (plan.commands.length === 0) {
    process.stdout.write("no verification commands selected\n");
    return;
  }
  for (const command of plan.commands) {
    process.stdout.write(`${command}\n`);
  }
}

function runCommand(command, env, hostSamplesPath, { appendHostSampleImpl = appendHostSample, spawnSyncImpl = childProcess.spawnSync } = {}) {
  if (hostSamplesPath) {
    appendHostSampleImpl(hostSamplesPath, `command:start:${command}`);
  }
  const startedAt = Date.now();
  const result = spawnSyncImpl("bash", ["-lc", command], {
    cwd: coreRoot,
    env,
    stdio: "inherit",
  });
  if (hostSamplesPath) {
    appendHostSampleImpl(hostSamplesPath, `command:end:${command}`);
  }
  return {
    command,
    durationMs: Date.now() - startedAt,
    error: result.error ? String(result.error.message || result.error) : "",
    signal: result.signal || "",
    status: typeof result.status === "number" ? result.status : result.error ? 1 : 0,
  };
}

function summarizeChildBazelRuns(entries, parentRunId) {
  return entries
    .filter((entry) => entry.kind === "bazel" && entry.parentRunId === parentRunId)
    .sort((left, right) => String(left.startedAt || "").localeCompare(String(right.startedAt || "")))
    .map((entry) => ({
      runId: entry.runId,
      entrypoint: entry.entrypoint,
      durationMs: entry.durationMs,
      remoteExecutionMode: entry.remoteExecutionMode || "",
      targets: Array.isArray(entry.targets) ? entry.targets : [],
      buildBuddyInvocations: Array.isArray(entry.buildBuddyInvocations)
        ? entry.buildBuddyInvocations
        : [],
    }));
}

function runVerificationPlan(plan, {
  appendHostSampleImpl = appendHostSample,
  buildCtxCacheEnvImpl = buildCtxCacheEnv,
  createRunArtifactsImpl = createRunArtifacts,
  finalizeRunArtifactsImpl = finalizeRunArtifacts,
  logErrorImpl = console.error,
  mkdirSyncImpl = fs.mkdirSync,
  readIndexedSummariesImpl = readIndexedSummaries,
  spawnSyncImpl = childProcess.spawnSync,
} = {}) {
  const { env, layout } = buildCtxCacheEnvImpl({
    cwd: coreRoot,
    env: process.env,
    mode: "workspace",
    mkdir: true,
  });
  const telemetryDisabled = ["1", "true", "yes", "on"].includes(
    String(env.CTX_DISABLE_VERIFICATION_TELEMETRY || "").trim().toLowerCase(),
  );
  let runArtifacts = null;
  if (!telemetryDisabled) {
    try {
      runArtifacts = createRunArtifactsImpl({
        cwd: coreRoot,
        env,
        entrypoint: plan.entrypoint,
        kind: "router",
        layout,
      });
      mkdirSyncImpl(runArtifacts.runDir, { recursive: true });
    } catch (error) {
      logErrorImpl(`warning: failed to initialize verification telemetry: ${String(error?.message || error)}`);
      runArtifacts = null;
    }
  }

  const startedAt = new Date().toISOString();
  const commandResults = [];
  let exitCode = 0;
  const commandEnv = runArtifacts == null
    ? env
    : {
        ...env,
        CTX_VERIFY_PARENT_ENTRYPOINT: plan.entrypoint,
        CTX_VERIFY_PARENT_RUN_ID: runArtifacts.runId,
      };

  try {
    if (runArtifacts != null) {
      appendHostSampleImpl(runArtifacts.hostSamplesPath, "router:start");
    }
    for (const command of plan.commands) {
      const result = runCommand(command, commandEnv, runArtifacts?.hostSamplesPath || "", {
        appendHostSampleImpl,
        spawnSyncImpl,
      });
      commandResults.push(result);
      if (result.status !== 0 || result.signal || result.error) {
        exitCode = result.status || 1;
        break;
      }
    }
    if (runArtifacts != null) {
      appendHostSampleImpl(runArtifacts.hostSamplesPath, "router:end");
    }
  } finally {
    if (runArtifacts != null) {
      try {
        const childBazelRuns = summarizeChildBazelRuns(
          readIndexedSummariesImpl({ cwd: coreRoot, env, layout }),
          runArtifacts.runId,
        );
        finalizeRunArtifactsImpl(runArtifacts, {
          startedAt,
          completedAt: new Date().toISOString(),
          durationMs: commandResults.reduce((total, entry) => total + entry.durationMs, 0),
          success: exitCode === 0,
          intent: plan.intent,
          baseRef: plan.baseRef,
          mergeBase: plan.mergeBase,
          profileId: plan.profileId,
          changedFiles: plan.changedFiles,
          overlayCommands: plan.overlayCommands,
          taxonomyEntries: plan.taxonomyEntries,
          taxonomyCommands: plan.taxonomyCommands,
          childBazelRuns,
          commands: commandResults,
        }, { env });
      } catch (error) {
        logErrorImpl(`warning: failed to record verification telemetry: ${String(error?.message || error)}`);
      }
    }
  }

  if (exitCode !== 0) {
    process.exit(exitCode);
  }
}

function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(String(error?.message || error));
    usage(2);
  }

  let plan;
  try {
    plan = buildVerificationPlan(args);
  } catch (error) {
    console.error(String(error?.message || error));
    process.exit(1);
  }

  if (args.json) {
    process.stdout.write(`${JSON.stringify(plan, null, 2)}\n`);
    return;
  }
  if (args.dryRun) {
    printTextPlan(plan);
    return;
  }
  if (plan.commands.length === 0) {
    process.stdout.write("no verification commands selected\n");
    return;
  }
  runVerificationPlan(plan);
}

if (require.main === module) {
  main();
}

module.exports = {
  MERGE_READY_COMMAND,
  buildOverlayCommands,
  buildVerificationPlan,
  dedupeCommands,
  hasProductionSourceChange,
  hasSupabaseFunctionChange,
  hasSupabaseMigrationChange,
  parseArgs,
  resolveIntentConfig,
  runVerificationPlan,
};
