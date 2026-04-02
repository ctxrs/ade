#!/usr/bin/env node
import { mkdirSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";

import { buildClickScenario } from "./demo_desktop_probe.mjs";
import {
  buildAutomationAppIfNeeded,
  connectBrowser,
  parseArgs as parsePlaybackArgs,
  prepareAutomationAppForLaunch,
  runConductor,
  startCrabNebulaStack,
  terminateAutomationAppProcesses,
} from "./demo_ping_pong_playback.mjs";

const DEFAULT_TARGET_IDS = ["top-left", "top-center", "center", "bottom-center", "bottom-right"];
const APP_OPEN_SETTLE_MS = 3_000;
const APP_FOREGROUND_SETTLE_MS = 3_000;

function parseArgs(argv) {
  const base = parsePlaybackArgs(argv);
  const overrides = {
    targetIds: DEFAULT_TARGET_IDS,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--targets") {
      overrides.targetIds = String(next || "")
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean);
      index += 1;
    }
  }
  return {
    artifactDir: base.artifactDir,
    skipBuild: base.skipBuild,
    backendPort: base.backendPort,
    driverPort: base.driverPort,
    tauriTargetDir: base.tauriTargetDir,
    appPath: base.appPath,
    targetIds: overrides.targetIds.length > 0 ? overrides.targetIds : DEFAULT_TARGET_IDS,
  };
}

function ensureDir(dir) {
  mkdirSync(dir, { recursive: true });
  return dir;
}

function sleep(ms) {
  return new Promise((resolve) => {
    globalThis.setTimeout(resolve, ms);
  });
}

function appleScriptStringLiteral(value) {
  return `"${String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function activateAutomationApp(appPath) {
  const result = spawnSync(
    "osascript",
    [
      "-e",
      `tell application (POSIX file ${appleScriptStringLiteral(appPath)} as text) to activate`,
    ],
    { stdio: "ignore" },
  );
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`failed to activate automation app at ${appPath}`);
  }
}

function buildGeometryHarnessAppEnv(targetIds) {
  const params = new URLSearchParams({ ctxE2E: "1" });
  if (targetIds.length > 0) {
    params.set("targets", targetIds.join(","));
  }
  return {
    ...process.env,
    CTX_DESKTOP_ALLOW_DEMO_COMMANDS: "1",
    CTX_DESKTOP_START_PATH: `/__geometry_harness?${params.toString()}`,
  };
}

async function waitForHarness(browser) {
  await browser.waitUntil(
    async () =>
      browser.execute(() => {
        return Boolean((window).__ctxGeometryHarness);
      }),
    {
      timeout: 30_000,
      interval: 200,
      timeoutMsg: "geometry harness bridge did not appear",
    },
  );
}

async function waitForBrowserFocus(browser) {
  await browser.waitUntil(
    async () =>
      await browser.execute(() => document.hasFocus() && document.visibilityState === "visible"),
    {
      timeout: 10_000,
      interval: 200,
      timeoutMsg: "geometry harness app did not become frontmost after activate",
    },
  );
}

async function measureTarget(browser, targetId) {
  return browser.execute(async (id) => {
    const bridge = window.__ctxGeometryHarness;
    if (!bridge) {
      throw new Error("geometry harness bridge unavailable");
    }
    return bridge.measureTarget(id);
  }, targetId);
}

async function resetProbe(browser) {
  return browser.execute(() => {
    const bridge = window.__ctxGeometryHarness;
    if (!bridge) {
      throw new Error("geometry harness bridge unavailable");
    }
    return bridge.resetProbe();
  });
}

async function readProbe(browser) {
  return browser.execute(() => {
    const bridge = window.__ctxGeometryHarness;
    if (!bridge) {
      throw new Error("geometry harness bridge unavailable");
    }
    return bridge.readProbe();
  });
}

function writeScenario(pathname, scenario) {
  writeFileSync(pathname, `${JSON.stringify(scenario, null, 2)}\n`, "utf8");
}

function analyzeResult(candidateId, target, probe) {
  const lastEvent = probe?.lastEvent ?? null;
  const actualX = typeof lastEvent?.clientX === "number" ? lastEvent.clientX : null;
  const actualY = typeof lastEvent?.clientY === "number" ? lastEvent.clientY : null;
  const dx = actualX === null ? null : Number((actualX - target.rect.centerX).toFixed(2));
  const dy = actualY === null ? null : Number((actualY - target.rect.centerY).toFixed(2));
  const distance =
    dx === null || dy === null ? null : Number(Math.hypot(dx, dy).toFixed(2));
  return {
    candidateId,
    probe,
    hitExpectedTarget: lastEvent?.targetId === target.id || lastEvent?.nearestTargetId === target.id,
    dx,
    dy,
    distance,
  };
}

export function summarizeGeometryResults(results) {
  const stats = new Map();
  for (const result of results) {
    const entry = stats.get(result.candidateId) ?? {
      candidateId: result.candidateId,
      totalDistance: 0,
      countedDistances: 0,
      expectedHits: 0,
      attempts: 0,
    };
    entry.attempts += 1;
    if (result.hitExpectedTarget) {
      entry.expectedHits += 1;
    }
    if (typeof result.distance === "number") {
      entry.totalDistance += result.distance;
      entry.countedDistances += 1;
    }
    stats.set(result.candidateId, entry);
  }
  const summary = Array.from(stats.values()).map((entry) => ({
    candidateId: entry.candidateId,
    attempts: entry.attempts,
    expectedHits: entry.expectedHits,
    meanDistance:
      entry.countedDistances > 0
        ? Number((entry.totalDistance / entry.countedDistances).toFixed(2))
        : null,
  }));
  summary.sort((left, right) => {
    if (right.expectedHits !== left.expectedHits) {
      return right.expectedHits - left.expectedHits;
    }
    if (left.meanDistance === null && right.meanDistance === null) return 0;
    if (left.meanDistance === null) return 1;
    if (right.meanDistance === null) return -1;
    return left.meanDistance - right.meanDistance;
  });
  return summary;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  ensureDir(options.artifactDir);

  let backendProc = null;
  let driverProc = null;
  let browser = null;

  try {
    buildAutomationAppIfNeeded(options.appPath, options.skipBuild, options.tauriTargetDir);
    const launchAppPath = prepareAutomationAppForLaunch(options.appPath, options.artifactDir);
    await terminateAutomationAppProcesses(launchAppPath);

    const cn = await startCrabNebulaStack({
      artifactDir: options.artifactDir,
      backendPort: options.backendPort,
      driverPort: options.driverPort,
      appEnv: buildGeometryHarnessAppEnv(options.targetIds),
    });
    backendProc = cn.backendProc;
    driverProc = cn.driverProc;
    browser = await connectBrowser({ driverPort: options.driverPort, appPath: launchAppPath });
    await waitForHarness(browser);
    await sleep(APP_OPEN_SETTLE_MS);
    activateAutomationApp(launchAppPath);
    await waitForBrowserFocus(browser);
    await sleep(APP_FOREGROUND_SETTLE_MS);

    const attemptResults = [];
    for (const targetId of options.targetIds) {
      const measurement = await measureTarget(browser, targetId);
      writeFileSync(
        path.join(options.artifactDir, `geometry-target-${targetId}.json`),
        `${JSON.stringify(measurement, null, 2)}\n`,
        "utf8",
      );
      for (const candidate of measurement.candidatePoints) {
        await resetProbe(browser);
        const scenarioPath = path.join(options.artifactDir, `scenario-${targetId}-${candidate.id}.json`);
        writeScenario(
          scenarioPath,
          buildClickScenario({ x: candidate.x, y: candidate.y }, { moveDurationMs: 260, waitDurationMs: 160 }),
        );
        await runConductor(scenarioPath);
        const probe = await readProbe(browser);
        attemptResults.push({
          targetId,
          targetLabel: measurement.target.label,
          targetCenter: {
            x: measurement.target.rect.centerX,
            y: measurement.target.rect.centerY,
          },
          candidate,
          ...analyzeResult(candidate.id, measurement.target, probe),
        });
      }
    }

    const report = {
      generatedAt: new Date().toISOString(),
      targetIds: options.targetIds,
      attempts: attemptResults,
      summary: summarizeGeometryResults(attemptResults),
    };
    const reportPath = path.join(options.artifactDir, "geometry-harness-report.json");
    writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
    process.stdout.write(`${reportPath}\n`);
  } finally {
    try {
      await browser?.deleteSession();
    } catch {}
    for (const proc of [driverProc, backendProc]) {
      if (!proc) continue;
      proc.kill("SIGTERM");
    }
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exitCode = 1;
  });
}
