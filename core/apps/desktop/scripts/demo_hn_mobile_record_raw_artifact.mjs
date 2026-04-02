#!/usr/bin/env node
import { createWriteStream, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

import {
  DEFAULT_HN_DEMO_SCENARIO_ID,
  HN_DEMO_SCENARIO_QUERY_KEY,
} from "../automation/fixtures/demo-workspaces/hn-mobile-baseline/demo_scenarios.js";
import { waitForHttpOk } from "./demo_lib.mjs";
import { buildFrameSequenceFfmpegArgs } from "./demo_hn_mobile_playback.mjs";
import {
  ensurePlaywrightCoreRuntime,
  GOOGLE_CHROME_EXECUTABLE,
  pickUnusedPortSync,
  runChecked,
} from "./demo_hn_mobile_artifact_lib.mjs";

const DEFAULT_VIEWPORT_WIDTH = 390;
const DEFAULT_VIEWPORT_HEIGHT = 844;
const DEFAULT_DEVICE_SCALE_FACTOR = 3;
const DEFAULT_CAPTURE_FPS = 18;

function parseArgs(argv) {
  const options = {
    workspaceRoot: "",
    outputPath: "",
    port: pickUnusedPortSync(4173),
    scenarioId: DEFAULT_HN_DEMO_SCENARIO_ID,
    viewportWidth: DEFAULT_VIEWPORT_WIDTH,
    viewportHeight: DEFAULT_VIEWPORT_HEIGHT,
    deviceScaleFactor: DEFAULT_DEVICE_SCALE_FACTOR,
    captureFps: DEFAULT_CAPTURE_FPS,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--workspace-root") {
      options.workspaceRoot = path.resolve(next);
      index += 1;
    } else if (arg === "--output") {
      options.outputPath = path.resolve(next);
      index += 1;
    } else if (arg === "--port") {
      options.port = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--scenario") {
      options.scenarioId = next;
      index += 1;
    } else if (arg === "--viewport-width") {
      options.viewportWidth = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--viewport-height") {
      options.viewportHeight = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--device-scale-factor") {
      options.deviceScaleFactor = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--fps") {
      options.captureFps = Number.parseInt(next, 10);
      index += 1;
    }
  }

  if (!options.workspaceRoot) {
    throw new Error("--workspace-root is required");
  }
  if (!options.outputPath) {
    throw new Error("--output is required");
  }
  return options;
}

export function buildRawCaptureUrl(previewUrl, scenarioId = DEFAULT_HN_DEMO_SCENARIO_ID) {
  const captureUrl = new URL(previewUrl);
  captureUrl.searchParams.set(HN_DEMO_SCENARIO_QUERY_KEY, scenarioId);
  return captureUrl.href;
}

function buildRunnerSource() {
  return `import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { chromium } from "playwright-core";

const captureUrl = process.env.CTX_HN_MOBILE_CAPTURE_URL;
const framesDir = process.env.CTX_HN_MOBILE_FRAMES_DIR;
const captureFps = Number(process.env.CTX_HN_MOBILE_CAPTURE_FPS || "18");
const viewportWidth = Number(process.env.CTX_HN_MOBILE_VIEWPORT_WIDTH || "390");
const viewportHeight = Number(process.env.CTX_HN_MOBILE_VIEWPORT_HEIGHT || "844");
const deviceScaleFactor = Number(process.env.CTX_HN_MOBILE_DEVICE_SCALE_FACTOR || "3");
const chromeExecutable = process.env.CTX_HN_MOBILE_CHROME_EXECUTABLE || undefined;

if (!captureUrl || !framesDir) {
  throw new Error("raw capture env vars are incomplete");
}

const frameIntervalMs = Math.max(16, Math.round(1000 / Math.max(1, captureFps)));
mkdirSync(framesDir, { recursive: true });

const browser = await chromium.launch({
  headless: true,
  executablePath: chromeExecutable,
});

const context = await browser.newContext({
  viewport: { width: viewportWidth, height: viewportHeight },
  screen: { width: viewportWidth, height: viewportHeight },
  deviceScaleFactor,
});

const page = await context.newPage();

let frameIndex = 0;
let capturing = true;
const captureLoop = (async () => {
  while (capturing) {
    const framePath = path.join(framesDir, \`frame-\${String(frameIndex).padStart(5, "0")}.png\`);
    await page.screenshot({ path: framePath });
    frameIndex += 1;
    await new Promise((resolve) => setTimeout(resolve, frameIntervalMs));
  }
})();

try {
  await page.goto(captureUrl, { waitUntil: "load" });
  const hnFrame = page.frameLocator("iframe.hn-page-frame");
  await hnFrame.locator("#hnmain").waitFor({ state: "visible", timeout: 30_000 });
  await page.waitForTimeout(1100);

  await hnFrame.locator('td[style*="text-align:right"] a').first().click();
  await hnFrame.locator('[data-muted-domains-input="true"]').waitFor({ state: "visible", timeout: 30_000 });
  await page.waitForTimeout(650);

  await hnFrame.locator('[data-muted-domains-input="true"]').fill("x.com\\ntwitter.com");
  await page.waitForTimeout(950);
  await hnFrame.locator('input[type="submit"][value="update"]').click();
  await hnFrame.locator('[data-muted-domains-input="true"]').waitFor({ state: "visible", timeout: 30_000 });
  await page.waitForTimeout(950);

  await hnFrame.locator("b.hnname a").click();
  await page.waitForTimeout(2600);
} finally {
  capturing = false;
  await captureLoop;
  writeFileSync(path.join(framesDir, "capture-metadata.json"), JSON.stringify({ frame_count: frameIndex }, null, 2));
  await context.close();
  await browser.close();
}
`;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const tempDir = mkdtempSync(path.join(os.tmpdir(), "ctx-hn-mobile-raw-"));
  const framesDir = path.join(tempDir, "frames");
  const runtimeDir = path.join(os.tmpdir(), "ctx-hn-mobile-playwright-runtime");
  const runnerPath = path.join(runtimeDir, "record-hn-mobile-raw-artifact.runner.mjs");
  const previewLogPath = path.join(tempDir, "preview.log");
  mkdirSync(path.dirname(options.outputPath), { recursive: true });

  let previewProc = null;

  try {
    runChecked("pnpm", ["install", "--frozen-lockfile"], { cwd: options.workspaceRoot });
    runChecked("pnpm", ["build"], { cwd: options.workspaceRoot });

    const previewLog = createWriteStream(previewLogPath, { flags: "a" });
    previewProc = spawn("pnpm", ["preview", "--host", "127.0.0.1", "--port", String(options.port)], {
      cwd: options.workspaceRoot,
      stdio: ["ignore", "pipe", "pipe"],
    });
    previewProc.stdout.pipe(previewLog, { end: false });
    previewProc.stderr.pipe(previewLog, { end: false });

    const previewUrl = `http://127.0.0.1:${options.port}`;
    await waitForHttpOk(previewUrl, { timeoutMs: 60_000, label: "hn-mobile preview" });

    ensurePlaywrightCoreRuntime(runtimeDir);
    writeFileSync(runnerPath, buildRunnerSource(), "utf8");

    runChecked(process.execPath, [runnerPath], {
      cwd: runtimeDir,
      env: {
        ...process.env,
        CTX_HN_MOBILE_CAPTURE_FPS: String(options.captureFps),
        CTX_HN_MOBILE_CAPTURE_URL: buildRawCaptureUrl(previewUrl, options.scenarioId),
        CTX_HN_MOBILE_CHROME_EXECUTABLE: existsSync(GOOGLE_CHROME_EXECUTABLE) ? GOOGLE_CHROME_EXECUTABLE : "",
        CTX_HN_MOBILE_DEVICE_SCALE_FACTOR: String(options.deviceScaleFactor),
        CTX_HN_MOBILE_FRAMES_DIR: framesDir,
        CTX_HN_MOBILE_VIEWPORT_HEIGHT: String(options.viewportHeight),
        CTX_HN_MOBILE_VIEWPORT_WIDTH: String(options.viewportWidth),
      },
    });

    const metadata = JSON.parse(readFileSync(path.join(framesDir, "capture-metadata.json"), "utf8"));
    const effectiveFps = metadata.frame_count > 0 ? options.captureFps : 1;

    runChecked("ffmpeg", buildFrameSequenceFfmpegArgs(framesDir, effectiveFps, options.outputPath));
    process.stdout.write(`${JSON.stringify({ status: "ok", output: options.outputPath }, null, 2)}\n`);
  } finally {
    if (previewProc) {
      previewProc.kill("SIGTERM");
    }
    rmSync(tempDir, { recursive: true, force: true });
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
