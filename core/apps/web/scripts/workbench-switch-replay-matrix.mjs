#!/usr/bin/env node

import { mkdir } from "node:fs/promises";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const fullProfiles = [
  {
    name: "plain-cold-first-pass",
    fixture: { contentProfile: "plain", turnsPerSession: 24, messageBytes: 768 },
    switches: "task-count",
    warmup: 0,
    initialWaitMs: 3000,
    waitMs: 120,
  },
  {
    name: "plain-repeat-warm",
    fixture: { contentProfile: "plain", turnsPerSession: 24, messageBytes: 768 },
    switches: 75,
    warmup: "task-count",
    initialWaitMs: 3000,
    waitMs: 120,
  },
  {
    name: "plain-repeat-warm-off",
    fixture: { contentProfile: "plain", turnsPerSession: 24, messageBytes: 768 },
    env: { CTX_REPLAY_PRETEXT_WARM_MODE: "off" },
    switches: 75,
    warmup: "task-count",
    initialWaitMs: 3000,
    waitMs: 120,
  },
  {
    name: "markdown-heavy",
    fixture: { contentProfile: "markdown", turnsPerSession: 40, messageBytes: 1200 },
    switches: 75,
    warmup: "task-count",
    initialWaitMs: 3000,
    waitMs: 140,
  },
  {
    name: "code-heavy",
    fixture: { contentProfile: "code", turnsPerSession: 40, messageBytes: 1200 },
    switches: 75,
    warmup: "task-count",
    initialWaitMs: 3000,
    waitMs: 140,
  },
  {
    name: "summary-storm",
    fixture: { contentProfile: "mixed", turnsPerSession: 24, messageBytes: 768 },
    switches: 75,
    warmup: "task-count",
    initialWaitMs: 3000,
    replayArgs: [
      "--synthesize-summary-deltas",
      "2400",
      "--synthesize-summary-interval-ms",
      "1",
      "--synthesize-summary-start-delay-ms",
      "0",
      "--no-wait-for-synth",
    ],
    waitMs: 120,
  },
  {
    name: "head-delta-storm",
    fixture: { contentProfile: "mixed", turnsPerSession: 24, messageBytes: 768 },
    switches: 75,
    warmup: "task-count",
    initialWaitMs: 3000,
    replayArgs: [
      "--synthesize-deltas",
      "1200",
      "--synthesize-interval-ms",
      "1",
      "--synthesize-start-delay-ms",
      "0",
      "--no-wait-for-synth",
      "--no-track-foreground-final",
    ],
    waitMs: 120,
  },
];

function usage() {
  console.error(
    "Usage: node ./scripts/workbench-switch-replay-matrix.mjs " +
      "[--out-dir dir] [--base-url url] [--task-count n] [--switches n] " +
      "[--warmup n] [--initial-wait-ms n] [--check] [--quick] [--compare-baseline dir]",
  );
  process.exit(1);
}

function parseArgs(argv) {
  const parsed = {
    outDir: path.join(process.cwd(), "tmp/workbench-switch-matrix"),
    baseUrl: "",
    taskCount: 20,
    switches: 55,
    warmup: 5,
    initialWaitMs: 0,
    check: false,
    quick: false,
    compareBaseline: "",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--out-dir") {
      parsed.outDir = String(argv[i + 1] ?? parsed.outDir);
      i += 1;
      continue;
    }
    if (arg === "--base-url") {
      parsed.baseUrl = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    if (arg === "--task-count") {
      parsed.taskCount = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--switches") {
      parsed.switches = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--warmup") {
      parsed.warmup = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--initial-wait-ms") {
      parsed.initialWaitMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--check") {
      parsed.check = true;
      continue;
    }
    if (arg === "--quick") {
      parsed.quick = true;
      continue;
    }
    if (arg === "--compare-baseline") {
      parsed.compareBaseline = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      usage();
    }
    usage();
  }
  if (!Number.isInteger(parsed.taskCount) || parsed.taskCount < 2 || parsed.taskCount > 100) {
    throw new Error("--task-count must be an integer from 2 to 100.");
  }
  if (!Number.isInteger(parsed.switches) || parsed.switches < 1 || parsed.switches > 1000) {
    throw new Error("--switches must be an integer from 1 to 1000.");
  }
  if (!Number.isInteger(parsed.warmup) || parsed.warmup < 0 || parsed.warmup > parsed.switches) {
    throw new Error("--warmup must be a non-negative integer no greater than --switches.");
  }
  if (!Number.isInteger(parsed.initialWaitMs) || parsed.initialWaitMs < 0 || parsed.initialWaitMs > 60000) {
    throw new Error("--initial-wait-ms must be a non-negative integer no greater than 60000.");
  }
  return parsed;
}

function runNode(scriptPath, args, env = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [scriptPath, ...args], {
      stdio: "inherit",
      env: { ...process.env, ...env },
    });
    child.on("exit", (code) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${path.basename(scriptPath)} exited with code ${code ?? "null"}`));
    });
    child.on("error", reject);
  });
}

function buildSwitchSequence(taskCount, switches) {
  return Array.from({ length: switches }, (_, index) => ((index + 1) % taskCount)).join(",");
}

function buildWaitSequence(switches, waitMs) {
  return Array.from({ length: Math.max(0, switches - 1) }, () => String(waitMs)).join(",");
}

function resolveProfileCount(value, args, fallback) {
  if (value === "task-count") return args.taskCount;
  if (Number.isInteger(value)) return value;
  return fallback;
}

function fixtureArgs(profile, args, fixturePath) {
  return [
    "--out",
    fixturePath,
    "--task-count",
    String(args.taskCount),
    "--turns-per-session",
    String(profile.fixture.turnsPerSession),
    "--content-profile",
    profile.fixture.contentProfile,
    "--message-bytes",
    String(profile.fixture.messageBytes),
  ];
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const outDir = path.resolve(args.outDir);
  const fixtureDir = path.join(outDir, "fixtures");
  const resultDir = path.join(outDir, "results");
  const generateScript = path.resolve(__dirname, "generate-workbench-switch-fixture.mjs");
  const replayScript = path.resolve(__dirname, "replay-loadtest.mjs");
  const compareScript = path.resolve(__dirname, "replay-loadtest-compare.mjs");
  const profiles = args.quick ? fullProfiles.slice(0, 1) : fullProfiles;
  await mkdir(fixtureDir, { recursive: true });
  await mkdir(resultDir, { recursive: true });

  for (const profile of profiles) {
    const profileSwitches = resolveProfileCount(profile.switches, args, args.switches);
    const profileWarmup = resolveProfileCount(profile.warmup, args, args.warmup);
    if (profileWarmup > profileSwitches) {
      throw new Error(`${profile.name}: warmup ${profileWarmup} exceeds switches ${profileSwitches}.`);
    }
    const fixturePath = path.join(fixtureDir, `${profile.name}.json`);
    await runNode(generateScript, fixtureArgs(profile, args, fixturePath));
    const replayArgs = [
      "--fixture",
      fixturePath,
      ...(args.baseUrl ? ["--base-url", args.baseUrl] : []),
      ...(args.check ? ["--check"] : []),
      "--out",
      path.join(resultDir, `${profile.name}.json`),
      ...(profile.replayArgs ?? []),
    ];
    const env = {
      CTX_REPLAY_CLICK_SEQUENCE: buildSwitchSequence(args.taskCount, profileSwitches),
      CTX_REPLAY_CLICK_WAITS_MS: buildWaitSequence(profileSwitches, profile.waitMs),
      CTX_REPLAY_SWITCH_WARMUP_COUNT: String(profileWarmup),
      CTX_REPLAY_INITIAL_WAIT_MS: String(profile.initialWaitMs ?? args.initialWaitMs),
      CTX_REPLAY_FINAL_WAIT_MS: "1000",
      ...(profile.env ?? {}),
    };
    await runNode(replayScript, replayArgs, env);
  }

  if (args.compareBaseline) {
    await runNode(compareScript, [
      "--baseline",
      path.resolve(args.compareBaseline),
      "--candidate",
      resultDir,
      "--out-json",
      path.join(outDir, "comparison.json"),
      "--out-md",
      path.join(outDir, "comparison.md"),
    ]);
  }

  console.log(`wrote matrix results to ${resultDir}`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
