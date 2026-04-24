#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const coreRoot = path.resolve(__dirname, "..");

const presetOrder = ["core", "extended", "full"];

function shellEscape(value) {
  return `'${String(value).replace(/'/gu, `'\\''`)}'`;
}

function isNonEmpty(value) {
  return typeof value === "string" && value.trim().length > 0;
}

function timestampForPath(date = new Date()) {
  return date.toISOString().replace(/[:.]/gu, "-");
}

function defaultOutRoot(env = process.env, date = new Date()) {
  const base =
    env.CTX_REDTEAM_OUT_DIR_BASE ||
    env.CTX_VOLATILE_TMPDIR ||
    path.join(os.homedir(), ".ctx", "volatile", "tmp");
  return path.join(base, `ctx-redteam-${timestampForPath(date)}`);
}

function commandExists(command, context = {}) {
  if (context.availableCommands instanceof Set) {
    return context.availableCommands.has(command);
  }
  const result = childProcess.spawnSync(
    "bash",
    ["-lc", `command -v ${shellEscape(command)} >/dev/null 2>&1`],
    {
      cwd: repoRoot,
      env: context.env || process.env,
      stdio: "ignore",
    },
  );
  return result.status === 0;
}

function getLaneDefinitions() {
  return [
    {
      id: "anomaly",
      title: "Fault-injection anomaly suite",
      description: "Exercises ctx-http and ctx-store failure-injection lanes.",
      minPreset: "core",
      cwd: coreRoot,
      command: "pnpm verify:anomaly",
      tags: ["hermetic", "simulated", "slow"],
    },
    {
      id: "fuzz-regression",
      title: "Corpus fuzz regression suite",
      description: "Runs provider, MCP, workspace payload, release manifest, and desktop IPC corpus regressions.",
      minPreset: "core",
      cwd: coreRoot,
      command: "pnpm verify:fuzz:regression",
      tags: ["hermetic", "adversarial", "slow"],
    },
    {
      id: "mutation-critical",
      title: "Critical mutation pilot",
      description: "Mutates high-risk code paths and expects targeted tests to kill those mutants.",
      minPreset: "core",
      cwd: coreRoot,
      command: "pnpm verify:mutation:critical",
      buildEnv: ({ laneDir }) => ({
        CTX_MUTATION_OUT_DIR: laneDir,
      }),
      artifactHints: ["report.json", "report.txt"],
      tags: ["local", "adversarial", "slow"],
    },
    {
      id: "provider-offline",
      title: "Offline provider fixture canaries",
      description: "Replays deterministic provider CRP fixtures through ctx-http.",
      minPreset: "core",
      cwd: coreRoot,
      command:
        "node scripts/run_with_ctx_cache_env.cjs --mode workspace --cwd . -- cargo test -p ctx-http --test provider_scenarios_offline",
      tags: ["hermetic", "provider", "medium"],
    },
    {
      id: "http-cross-platform",
      title: "HTTP cross-platform regressions",
      description: "Exercises reconnect, stream integrity, and archive HTTP behaviors.",
      minPreset: "core",
      cwd: coreRoot,
      command: "bash scripts/run-rust-suite.sh cross_platform",
      tags: ["hermetic", "http", "medium"],
    },
    {
      id: "updater-failure-safety",
      title: "Updater failure safety",
      description: "Validates interrupted transfer, checksum mismatch, manifest parse, and missing artifact handling.",
      minPreset: "core",
      cwd: coreRoot,
      command: "pnpm test:updates:failure-safety",
      tags: ["hermetic", "updater", "medium"],
    },
    {
      id: "release-manifest-race",
      title: "Release manifest race smoke",
      description: "Exercises concurrent release manifest merge and promotion behavior.",
      minPreset: "core",
      cwd: repoRoot,
      command: "bash scripts/tests/release_manifest_concurrency_smoke.sh",
      tags: ["hermetic", "release", "medium"],
    },
    {
      id: "release-promote-smoke",
      title: "Release promote smoke",
      description: "Validates stable latest promotion against a hermetic local Supabase-style store.",
      minPreset: "core",
      cwd: repoRoot,
      command: "bash scripts/tests/release_promote_supabase_latest_smoke.sh",
      tags: ["hermetic", "release", "medium"],
    },
    {
      id: "updater-contract-smoke",
      title: "Updater contract validator smoke",
      description: "Checks updater signature and pubkey validation against hermetic artifacts.",
      minPreset: "core",
      cwd: repoRoot,
      command: "bash scripts/tests/updater_contract_validate_smoke.sh",
      tags: ["hermetic", "updater", "fast"],
    },
    {
      id: "web-pretext-fuzz",
      title: "Workbench pretext fuzz parity",
      description: "Fuzzes pretext markdown/parity rendering through the web E2E targets.",
      minPreset: "extended",
      cwd: coreRoot,
      command: "pnpm bazel:web:e2e:pretext:fuzz && pnpm bazel:web:e2e:pretext:wrap-fuzz",
      tags: ["browser", "adversarial", "slow"],
    },
    {
      id: "load-smoke",
      title: "Daemon load smoke",
      description: "Exercises a fake-provider daemon under concurrent task and subagent load.",
      minPreset: "extended",
      cwd: coreRoot,
      command: "bash scripts/run-rust-suite.sh load",
      buildEnv: ({ laneDir }) => ({
        CTX_LOAD_SMOKE_KEEP_TMP: "1",
        CTX_LOAD_SMOKE_TMP_DIR: laneDir,
      }),
      artifactHints: ["out/load/summary.json", "out/daemon.log"],
      tags: ["local", "load", "slow"],
    },
    {
      id: "soak",
      title: "Workspace stream soak",
      description: "Runs ignored soak tests covering stream lag and memory leak scenarios.",
      minPreset: "extended",
      cwd: coreRoot,
      command: "bash scripts/run-rust-suite.sh soak",
      tags: ["local", "soak", "slow"],
    },
    {
      id: "perf-daemon-cpu",
      title: "Daemon perf smoke",
      description: "Boots the daemon and checks resource telemetry stays within the configured smoke thresholds.",
      minPreset: "extended",
      cwd: repoRoot,
      command: "bash scripts/perf_smoke_daemon_cpu.sh",
      buildEnv: ({ laneDir }) => ({
        CTX_PERF_DATA_DIR: path.join(laneDir, "data"),
        CTX_PERF_LOG_PATH: path.join(laneDir, "perf.log"),
      }),
      artifactHints: ["perf.log", "data/logs"],
      tags: ["local", "performance", "medium"],
    },
    {
      id: "sandbox-egress-guard",
      title: "Sandbox egress guard smoke",
      description: "Verifies the managed harness image enforces its allowlist egress policy for non-root traffic.",
      minPreset: "full",
      cwd: repoRoot,
      command: "bash scripts/smoke_ctx_harness_egress_guard.sh",
      skipReason(context) {
        const runtime = (context.env.CONTAINER_RUNTIME || "nerdctl").trim();
        if (!runtime) {
          return "requires CONTAINER_RUNTIME or nerdctl";
        }
        if (!commandExists(runtime, context)) {
          return `requires container runtime '${runtime}'`;
        }
        return null;
      },
      tags: ["container", "network", "opt-in"],
    },
    {
      id: "desktop-break-matrix",
      title: "Desktop break matrix",
      description: "Runs the macOS desktop break-matrix automation suite against failure-oriented cases.",
      minPreset: "full",
      cwd: coreRoot,
      command: "pnpm verify:desktop:break-matrix",
      skipReason(context) {
        if (context.platform !== "darwin") {
          return "requires macOS";
        }
        if (!isNonEmpty(context.env.CN_API_KEY)) {
          return "requires CN_API_KEY";
        }
        if (!isNonEmpty(context.env.OPENROUTER_API_KEY)) {
          return "requires OPENROUTER_API_KEY";
        }
        return null;
      },
      tags: ["mac", "desktop", "secrets", "opt-in"],
    },
    {
      id: "provider-live-canary",
      title: "Live provider canary",
      description: "Runs ignored live provider canaries against a real configured provider/model pair.",
      minPreset: "full",
      cwd: coreRoot,
      command:
        "node scripts/run_with_ctx_cache_env.cjs --mode workspace --cwd . -- cargo test -p ctx-http --test live_provider_canary -- --ignored --nocapture --test-threads=1",
      skipReason(context) {
        if (!isNonEmpty(context.env.CTX_LIVE_PROVIDER_ID)) {
          return "requires CTX_LIVE_PROVIDER_ID";
        }
        if (!isNonEmpty(context.env.CTX_LIVE_MODEL_ID)) {
          return "requires CTX_LIVE_MODEL_ID";
        }
        return null;
      },
      tags: ["network", "provider", "secrets", "opt-in"],
    },
  ];
}

function getLaneById() {
  return new Map(getLaneDefinitions().map((lane) => [lane.id, lane]));
}

function ensureKnownPreset(presetId) {
  if (!presetOrder.includes(presetId)) {
    throw new Error(`unknown preset: ${presetId}`);
  }
}

function getPresetLaneIds(presetId) {
  ensureKnownPreset(presetId);
  const currentIndex = presetOrder.indexOf(presetId);
  return getLaneDefinitions()
    .filter((lane) => presetOrder.indexOf(lane.minPreset) <= currentIndex)
    .map((lane) => lane.id);
}

function dedupe(values) {
  const seen = new Set();
  const ordered = [];
  for (const value of values) {
    if (!seen.has(value)) {
      seen.add(value);
      ordered.push(value);
    }
  }
  return ordered;
}

function resolveSelectedLanes(args) {
  const laneById = getLaneById();
  const requestedIds =
    args.laneIds.length > 0 ? dedupe(args.laneIds) : getPresetLaneIds(args.preset);

  for (const laneId of requestedIds) {
    if (!laneById.has(laneId)) {
      throw new Error(`unknown lane: ${laneId}`);
    }
  }
  for (const laneId of args.skipLaneIds) {
    if (!laneById.has(laneId)) {
      throw new Error(`unknown lane: ${laneId}`);
    }
  }

  const skipIds = new Set(args.skipLaneIds);
  return requestedIds
    .filter((laneId) => !skipIds.has(laneId))
    .map((laneId) => laneById.get(laneId));
}

function resolveLaneSkipReason(lane, context) {
  if (typeof lane.skipReason === "function") {
    return lane.skipReason(context);
  }
  return null;
}

function parseArgs(argv) {
  const args = {
    failFast: false,
    json: false,
    laneIds: [],
    list: false,
    outDir: "",
    preset: "core",
    skipLaneIds: [],
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--") {
      continue;
    } else if (arg === "--preset") {
      args.preset = argv[index + 1] || "";
      index += 1;
    } else if (arg === "--lane") {
      args.laneIds.push(argv[index + 1] || "");
      index += 1;
    } else if (arg === "--skip-lane") {
      args.skipLaneIds.push(argv[index + 1] || "");
      index += 1;
    } else if (arg === "--out-dir") {
      args.outDir = argv[index + 1] || "";
      index += 1;
    } else if (arg === "--list") {
      args.list = true;
    } else if (arg === "--json") {
      args.json = true;
    } else if (arg === "--fail-fast") {
      args.failFast = true;
    } else if (arg === "-h" || arg === "--help") {
      printUsage();
      process.exit(0);
    } else {
      throw new Error(`unknown arg: ${arg}`);
    }
  }

  ensureKnownPreset(args.preset);
  return args;
}

function printUsage() {
  process.stdout.write(
    [
      "usage: node scripts/red_team_runner.cjs [--preset core|extended|full] [--lane ID] [--skip-lane ID] [--out-dir DIR] [--list] [--json] [--fail-fast]",
      "",
      "notes:",
      "- default preset is core",
      "- if any --lane values are provided, they override preset selection",
      "- full preset includes opt-in lanes that may skip when local prerequisites are missing",
      "",
    ].join("\n"),
  );
}

function listLanes({ json = false } = {}) {
  const lanes = getLaneDefinitions().map((lane) => ({
    id: lane.id,
    title: lane.title,
    description: lane.description,
    min_preset: lane.minPreset,
    tags: lane.tags || [],
  }));
  if (json) {
    process.stdout.write(`${JSON.stringify(lanes, null, 2)}\n`);
    return;
  }
  for (const lane of lanes) {
    process.stdout.write(
      `${lane.id}\t${lane.min_preset}\t${lane.title}\t${lane.description}\n`,
    );
  }
}

function runLane(lane, runContext) {
  const laneDir = path.join(runContext.outDir, lane.id);
  fs.mkdirSync(laneDir, { recursive: true });

  const env = {
    ...process.env,
    ...(typeof lane.buildEnv === "function"
      ? lane.buildEnv({ laneDir, outDir: runContext.outDir })
      : {}),
  };

  const context = {
    env,
    platform: process.platform,
  };
  const skipReason = resolveLaneSkipReason(lane, context);
  if (skipReason) {
    process.stdout.write(`skip ${lane.id}: ${skipReason}\n`);
    return {
      artifact_hints: lane.artifactHints || [],
      duration_ms: 0,
      id: lane.id,
      log_path: null,
      started_at: null,
      status: "skipped",
      stop_reason: skipReason,
      title: lane.title,
    };
  }

  const logPath = path.join(laneDir, "run.log");
  const startedAt = new Date();
  process.stdout.write(`run ${lane.id}: ${lane.command}\n`);

  const result = childProcess.spawnSync(
    "bash",
    [
      "-lc",
      `set -euo pipefail\n{ ${lane.command}; } 2>&1 | tee ${shellEscape(logPath)}`,
    ],
    {
      cwd: lane.cwd,
      env,
      stdio: ["ignore", "inherit", "inherit"],
    },
  );

  const completedAt = new Date();
  const durationMs = completedAt.getTime() - startedAt.getTime();
  const status = result.status === 0 ? "passed" : "failed";
  process.stdout.write(`${status} ${lane.id} (${durationMs}ms)\n`);

  return {
    artifact_hints: lane.artifactHints || [],
    duration_ms: durationMs,
    exit_code: result.status,
    id: lane.id,
    log_path: logPath,
    started_at: startedAt.toISOString(),
    status,
    stop_reason:
      result.error ? String(result.error.message || result.error) : result.status === 0 ? null : `command exited ${result.status}`,
    title: lane.title,
  };
}

function buildTextSummary(summary) {
  const lines = [];
  lines.push(`preset=${summary.preset}`);
  lines.push(`out_dir=${summary.out_dir}`);
  lines.push(
    `passed=${summary.counts.passed} failed=${summary.counts.failed} skipped=${summary.counts.skipped} total=${summary.counts.total}`,
  );
  for (const lane of summary.results) {
    const details = [];
    if (lane.log_path) {
      details.push(`log=${lane.log_path}`);
    }
    if (lane.stop_reason) {
      details.push(`reason=${lane.stop_reason}`);
    }
    lines.push(`- ${lane.id}: ${lane.status}${details.length ? ` (${details.join("; ")})` : ""}`);
  }
  return `${lines.join("\n")}\n`;
}

function writeSummary(outDir, summary) {
  fs.mkdirSync(outDir, { recursive: true });
  const jsonPath = path.join(outDir, "summary.json");
  const txtPath = path.join(outDir, "summary.txt");
  fs.writeFileSync(jsonPath, `${JSON.stringify(summary, null, 2)}\n`, "utf8");
  fs.writeFileSync(txtPath, buildTextSummary(summary), "utf8");
  return { jsonPath, txtPath };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.list) {
    listLanes({ json: args.json });
    return;
  }

  const selectedLanes = resolveSelectedLanes(args);
  const outDir = args.outDir || defaultOutRoot(process.env);
  fs.mkdirSync(outDir, { recursive: true });

  const startedAt = new Date();
  const results = [];
  for (const lane of selectedLanes) {
    const result = runLane(lane, { outDir });
    results.push(result);
    if (args.failFast && result.status === "failed") {
      break;
    }
  }
  const completedAt = new Date();

  const counts = {
    failed: results.filter((lane) => lane.status === "failed").length,
    passed: results.filter((lane) => lane.status === "passed").length,
    skipped: results.filter((lane) => lane.status === "skipped").length,
    total: results.length,
  };

  const summary = {
    completed_at: completedAt.toISOString(),
    counts,
    out_dir: outDir,
    preset: args.preset,
    requested_lanes:
      args.laneIds.length > 0 ? dedupe(args.laneIds) : getPresetLaneIds(args.preset),
    results,
    selected_lanes: selectedLanes.map((lane) => lane.id),
    started_at: startedAt.toISOString(),
  };

  const { jsonPath, txtPath } = writeSummary(outDir, summary);
  process.stdout.write(
    `red-team summary: passed=${counts.passed} failed=${counts.failed} skipped=${counts.skipped}\n`,
  );
  process.stdout.write(`summary json: ${jsonPath}\n`);
  process.stdout.write(`summary txt:  ${txtPath}\n`);

  if (args.json) {
    process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
  }

  if (counts.failed > 0) {
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  defaultOutRoot,
  getLaneDefinitions,
  getPresetLaneIds,
  parseArgs,
  resolveLaneSkipReason,
  resolveSelectedLanes,
};
