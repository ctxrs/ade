#!/usr/bin/env node
import { createWriteStream, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import os from "node:os";
import path from "node:path";

import { waitForHttpOk } from "./demo_lib.mjs";

const GOOGLE_CHROME_EXECUTABLE = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const SCRIPT_DIR = path.dirname(new URL(import.meta.url).pathname);
const REPO_ROOT = path.resolve(SCRIPT_DIR, "../../../..");
const PLAYWRIGHT_CORE_VERSION = "1.57.0";

function pickUnusedPortSync(fallback) {
  const script = [
    "const net = require('node:net');",
    "const server = net.createServer();",
    "server.on('error', () => process.exit(2));",
    "server.listen(0, '127.0.0.1', () => {",
    "  const addr = server.address();",
    "  const port = addr && typeof addr === 'object' ? addr.port : 0;",
    "  server.close(() => {",
    "    if (!port) process.exit(3);",
    "    process.stdout.write(String(port));",
    "  });",
    "});",
  ].join("\n");
  const result = spawnSync(process.execPath, ["-e", script], { encoding: "utf8" });
  if (result.status !== 0) {
    return fallback;
  }
  const parsed = Number.parseInt(String(result.stdout || "").trim(), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with status ${result.status}`);
  }
}

function parseArgs(argv) {
  const options = {
    workspaceRoot: "",
    outputPath: "",
    port: pickUnusedPortSync(4173),
    videoWidth: 720,
    videoHeight: 1280,
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
    } else if (arg === "--width") {
      options.videoWidth = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--height") {
      options.videoHeight = Number.parseInt(next, 10);
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

export function buildWrapperHtml(previewUrl) {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>HN Mobile Demo</title>
    <style>
      :root {
        color-scheme: dark;
      }

      * {
        box-sizing: border-box;
      }

      body {
        margin: 0;
        min-height: 100vh;
        display: grid;
        place-items: center;
        background:
          radial-gradient(circle at top, rgba(255, 149, 0, 0.3), transparent 42%),
          linear-gradient(180deg, #1a1410 0%, #0f1014 100%);
        font-family: "SF Pro Display", "Helvetica Neue", sans-serif;
      }

      .stage {
        width: 720px;
        height: 1280px;
        display: grid;
        place-items: center;
      }

      .phone {
        width: 520px;
        height: 1088px;
        position: relative;
        border-radius: 76px;
        padding: 20px;
        background:
          linear-gradient(145deg, rgba(255, 255, 255, 0.12), rgba(8, 8, 8, 0.98)),
          #111315;
        box-shadow:
          0 36px 80px rgba(0, 0, 0, 0.42),
          inset 0 1px 0 rgba(255, 255, 255, 0.2);
      }

      .phone::before {
        content: "";
        position: absolute;
        inset: 10px;
        border-radius: 64px;
        border: 1px solid rgba(255, 255, 255, 0.12);
        pointer-events: none;
      }

      .phone::after {
        content: "";
        position: absolute;
        top: 38px;
        left: 50%;
        width: 182px;
        height: 34px;
        transform: translateX(-50%);
        border-radius: 999px;
        background: rgba(10, 10, 10, 0.95);
        box-shadow: inset 0 -2px 6px rgba(255, 255, 255, 0.05);
        z-index: 2;
      }

      .screen {
        width: 100%;
        height: 100%;
        display: flex;
        flex-direction: column;
        padding: 58px 0 24px;
        border-radius: 58px;
        overflow: hidden;
        background: #f6f6ef;
        box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.06);
      }

      .screen-safe {
        flex: 1;
        min-height: 0;
        overflow: hidden;
      }

      iframe {
        width: 100%;
        height: 100%;
        border: 0;
        display: block;
        background: #f6f6ef;
      }
    </style>
  </head>
  <body>
    <div class="stage">
      <div class="phone">
        <div class="screen">
          <div class="screen-safe">
            <iframe class="demo-phone-screen" src="${previewUrl}" title="HN Mobile demo"></iframe>
          </div>
        </div>
      </div>
    </div>
  </body>
</html>
`;
}

function ensurePlaywrightCoreRuntime(runtimeDir) {
  const packageMarker = path.join(runtimeDir, "node_modules", "playwright-core", "package.json");
  if (existsSync(packageMarker)) {
    return;
  }
  mkdirSync(runtimeDir, { recursive: true });
  runChecked("npm", [
    "install",
    "--no-save",
    "--prefix", runtimeDir,
    `playwright-core@${PLAYWRIGHT_CORE_VERSION}`,
  ], { cwd: REPO_ROOT });
}

export function buildPlaywrightRunner() {
  return `import { cpSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { chromium } from "playwright-core";

const width = Number(process.env.CTX_HN_MOBILE_VIDEO_WIDTH || "720");
const height = Number(process.env.CTX_HN_MOBILE_VIDEO_HEIGHT || "1280");
const wrapperPath = process.env.CTX_HN_MOBILE_WRAPPER_PATH;
const rawVideoPath = process.env.CTX_HN_MOBILE_RAW_VIDEO_PATH;
const chromeExecutable = process.env.CTX_HN_MOBILE_CHROME_EXECUTABLE || undefined;

if (!wrapperPath || !rawVideoPath) {
  throw new Error("artifact recording env vars are incomplete");
}

const browser = await chromium.launch({
  headless: true,
  executablePath: chromeExecutable,
});

const context = await browser.newContext({
  viewport: { width, height },
  recordVideo: {
    dir: process.cwd(),
    size: { width, height },
  },
});

const page = await context.newPage();
const pageVideo = page.video();

await page.goto(pathToFileURL(wrapperPath).href, { waitUntil: "load" });
const appFrame = page.frameLocator("iframe.demo-phone-screen");
const hnFrame = appFrame.frameLocator("iframe.hn-page-frame");
await appFrame.locator("iframe.hn-page-frame").waitFor({ state: "visible", timeout: 30_000 });
await hnFrame.locator('[data-muted-domains-input="true"]').waitFor({ state: "visible", timeout: 30_000 });
await page.waitForTimeout(900);
await hnFrame.locator('[data-muted-domains-input="true"]').fill("x.com");
await page.waitForTimeout(750);
await hnFrame.locator('input[type="submit"][value="update"]').click();
await hnFrame.locator('[data-muted-domains-input="true"]').waitFor({ state: "visible", timeout: 30_000 });
await page.waitForTimeout(600);
await hnFrame.locator("body").evaluate((body) => {
  body.ownerDocument.defaultView.location.href = "/proxy/hn/news?ctxDemoMockFrontPage=1";
});
await hnFrame.locator("tr.athing").first().waitFor({ state: "visible", timeout: 30_000 });
await hnFrame.locator("#ctx-muted-domains-toast").waitFor({ state: "visible", timeout: 30_000 });
await page.waitForTimeout(2100);

await context.close();
await browser.close();

const recordedVideoPath = await pageVideo.path();
cpSync(recordedVideoPath, rawVideoPath);
`;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const tempDir = mkdtempSync(path.join(os.tmpdir(), "ctx-hn-mobile-recording-"));
  const previewLogPath = path.join(tempDir, "preview.log");
  const wrapperPath = path.join(tempDir, "wrapper.html");
  const runtimeDir = path.join(os.tmpdir(), "ctx-hn-mobile-playwright-runtime");
  const runnerPath = path.join(runtimeDir, "record-hn-mobile-artifact.runner.mjs");
  const rawVideoPath = path.join(tempDir, "hn-mobile-artifact.webm");
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

    const previewUrl = `http://127.0.0.1:${options.port}?demoPath=${encodeURIComponent("/proxy/hn/user?id=ADE_TEST_ACCOUNT&ctxDemoMockFrontPage=1")}`;
    await waitForHttpOk(previewUrl, { timeoutMs: 60_000, label: "hn-mobile preview" });

    ensurePlaywrightCoreRuntime(runtimeDir);
    writeFileSync(wrapperPath, buildWrapperHtml(previewUrl), "utf8");
    writeFileSync(runnerPath, buildPlaywrightRunner(), "utf8");

    runChecked(process.execPath, [
      runnerPath,
    ], {
      cwd: runtimeDir,
      env: {
        ...process.env,
        CTX_HN_MOBILE_CHROME_EXECUTABLE: existsSync(GOOGLE_CHROME_EXECUTABLE) ? GOOGLE_CHROME_EXECUTABLE : "",
        CTX_HN_MOBILE_RAW_VIDEO_PATH: rawVideoPath,
        CTX_HN_MOBILE_VIDEO_HEIGHT: String(options.videoHeight),
        CTX_HN_MOBILE_VIDEO_WIDTH: String(options.videoWidth),
        CTX_HN_MOBILE_WRAPPER_PATH: wrapperPath,
      },
    });

    runChecked("ffmpeg", [
      "-y",
      "-i", rawVideoPath,
      "-an",
      "-c:v", "libx264",
      "-pix_fmt", "yuv420p",
      "-movflags", "+faststart",
      options.outputPath,
    ]);

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
