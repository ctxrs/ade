#!/usr/bin/env node

import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const profiles = [
  {
    name: "baseline-replay",
    args: [],
  },
  {
    name: "foreground-pressure",
    args: [
      "--synthesize-task-deltas",
      "2400",
      "--synthesize-task-ids",
      "task-b",
      "--synthesize-task-interval-ms",
      "1",
      "--synthesize-task-start-delay-ms",
      "50",
      "--reset-session-head-to-running",
      "session-a",
      "--wait-timeout-ms",
      "45000",
    ],
  },
  {
    name: "multi-session-pressure",
    args: [
      "--synthesize-task-deltas",
      "4800",
      "--synthesize-task-ids",
      "task-b",
      "--synthesize-task-interval-ms",
      "1",
      "--synthesize-task-start-delay-ms",
      "50",
      "--reset-session-head-to-running",
      "session-a",
      "--wait-timeout-ms",
      "45000",
    ],
  },
];

function usage() {
  console.error(
    "Usage: node ./scripts/replay-loadtest-profiles.mjs " +
      "[--out-dir dir] [--base-url url] [--fixture path] [--check] [--compare-baseline dir]",
  );
  process.exit(1);
}

function parseArgs(argv) {
  const parsed = {
    outDir: "",
    baseUrl: "",
    fixture: "",
    check: false,
    compareBaseline: "",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--out-dir") {
      parsed.outDir = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    if (arg === "--base-url") {
      parsed.baseUrl = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    if (arg === "--fixture") {
      parsed.fixture = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    if (arg === "--check") {
      parsed.check = true;
      continue;
    }
    if (arg === "--compare-baseline") {
      parsed.compareBaseline = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    usage();
  }
  return parsed;
}

function runNode(scriptPath, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [scriptPath, ...args], {
      stdio: "inherit",
      env: process.env,
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

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const outDir = path.resolve(args.outDir || path.join(process.cwd(), "tmp/replay-loadtest"));
  const replayScript = path.resolve(__dirname, "replay-loadtest.mjs");
  const compareScript = path.resolve(__dirname, "replay-loadtest-compare.mjs");
  await mkdir(outDir, { recursive: true });

  for (const profile of profiles) {
    const profileArgs = [
      ...(args.fixture ? ["--fixture", path.resolve(args.fixture)] : []),
      ...(args.baseUrl ? ["--base-url", args.baseUrl] : []),
      ...(args.check ? ["--check"] : []),
      "--out",
      path.join(outDir, `${profile.name}.json`),
      ...profile.args,
    ];
    await runNode(replayScript, profileArgs);
  }

  if (args.compareBaseline) {
    await runNode(compareScript, [
      "--baseline",
      path.resolve(args.compareBaseline),
      "--candidate",
      outDir,
      "--out-json",
      path.join(outDir, "comparison.json"),
      "--out-md",
      path.join(outDir, "comparison.md"),
    ]);
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
