#!/usr/bin/env node
// README:
//   node core/apps/native/scripts/ghosting-check.mjs --addr http://127.0.0.1:6160
//   node core/apps/native/scripts/ghosting-check.mjs --shots-dir /tmp/ctx-native-shots --use-existing
import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

const DEFAULT_ADDR = "http://127.0.0.1:6160";
const DEFAULT_SHOTS_DIR = process.env.CTX_NATIVE_SHOTS_DIR || "/tmp/ctx-native-shots";
const DEFAULT_THRESHOLD = 0.002;
const DEFAULT_PIXEL_THRESHOLD = 0.1;
const DEFAULT_DELAY_MS = 400;
const DEFAULT_SCROLL_LINES = 8;
const DEFAULT_SCROLL_STEPS = 6;

const USAGE = `
Ghosting regression check for native thread list.

Usage:
  node core/apps/native/scripts/ghosting-check.mjs [options]

Options:
  --addr <host:port|url>     Automation server (default: ${DEFAULT_ADDR})
  --http <url>               HTTP automation URL (overrides --addr)
  --ws <url>                 WebSocket automation URL (overrides --addr)
  --shots-dir <dir>          Directory for existing screenshots (default: ${DEFAULT_SHOTS_DIR})
  --out <dir>                Output directory (default: <shots-dir>/ghosting-check)
  --threshold <ratio>        Max diff ratio to pass (default: ${DEFAULT_THRESHOLD})
  --pixel-threshold <ratio>  Pixelmatch threshold (default: ${DEFAULT_PIXEL_THRESHOLD})
  --delay-ms <ms>            Delay between scrolls (default: ${DEFAULT_DELAY_MS})
  --scroll-lines <n>         Lines per scroll step (default: ${DEFAULT_SCROLL_LINES})
  --scroll-steps <n>         Number of scroll steps (default: ${DEFAULT_SCROLL_STEPS})
  --skip-capture             Skip running /tmp/native-scroll-check.mjs
  --use-existing             Use existing ghost-top images (skip new capture)
  -h, --help                 Show this help text
`;

function parseArgs(argv) {
  const args = {
    addr: DEFAULT_ADDR,
    httpUrl: null,
    wsUrl: null,
    shotsDir: DEFAULT_SHOTS_DIR,
    outDir: null,
    threshold: DEFAULT_THRESHOLD,
    pixelThreshold: DEFAULT_PIXEL_THRESHOLD,
    delayMs: DEFAULT_DELAY_MS,
    scrollLines: DEFAULT_SCROLL_LINES,
    scrollSteps: DEFAULT_SCROLL_STEPS,
    skipCapture: false,
    useExisting: false,
    help: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      args.help = true;
      continue;
    }
    if (arg === "--addr") {
      args.addr = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--addr=")) {
      args.addr = arg.slice("--addr=".length);
      continue;
    }
    if (arg === "--http") {
      args.httpUrl = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--http=")) {
      args.httpUrl = arg.slice("--http=".length);
      continue;
    }
    if (arg === "--ws") {
      args.wsUrl = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--ws=")) {
      args.wsUrl = arg.slice("--ws=".length);
      continue;
    }
    if (arg === "--shots-dir") {
      args.shotsDir = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--shots-dir=")) {
      args.shotsDir = arg.slice("--shots-dir=".length);
      continue;
    }
    if (arg === "--out") {
      args.outDir = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--out=")) {
      args.outDir = arg.slice("--out=".length);
      continue;
    }
    if (arg === "--threshold") {
      args.threshold = Number.parseFloat(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg.startsWith("--threshold=")) {
      args.threshold = Number.parseFloat(arg.slice("--threshold=".length));
      continue;
    }
    if (arg === "--pixel-threshold") {
      args.pixelThreshold = Number.parseFloat(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg.startsWith("--pixel-threshold=")) {
      args.pixelThreshold = Number.parseFloat(arg.slice("--pixel-threshold=".length));
      continue;
    }
    if (arg === "--delay-ms") {
      args.delayMs = Number.parseInt(argv[i + 1], 10);
      i += 1;
      continue;
    }
    if (arg.startsWith("--delay-ms=")) {
      args.delayMs = Number.parseInt(arg.slice("--delay-ms=".length), 10);
      continue;
    }
    if (arg === "--ready-timeout-ms") {
      if (i + 1 < argv.length) {
        i += 1;
      }
      continue;
    }
    if (arg.startsWith("--ready-timeout-ms=")) {
      continue;
    }
    if (arg === "--scroll-lines") {
      args.scrollLines = Number.parseInt(argv[i + 1], 10);
      i += 1;
      continue;
    }
    if (arg.startsWith("--scroll-lines=")) {
      args.scrollLines = Number.parseInt(arg.slice("--scroll-lines=".length), 10);
      continue;
    }
    if (arg === "--scroll-steps") {
      args.scrollSteps = Number.parseInt(argv[i + 1], 10);
      i += 1;
      continue;
    }
    if (arg.startsWith("--scroll-steps=")) {
      args.scrollSteps = Number.parseInt(arg.slice("--scroll-steps=".length), 10);
      continue;
    }
    if (arg === "--skip-capture" || arg === "--no-capture") {
      args.skipCapture = true;
      continue;
    }
    if (arg === "--use-existing") {
      args.useExisting = true;
      continue;
    }
    throw new Error(`Unknown argument: ${arg}`);
  }

  return args;
}

function normalizeAddr(value) {
  const trimmed = String(value || "").trim();
  if (!trimmed) {
    throw new Error("--addr must not be empty");
  }
  if (/^[a-z]+:\/\//i.test(trimmed)) {
    return trimmed;
  }
  return `http://${trimmed}`;
}

function wsUrlFromHttp(addr) {
  const url = new URL(addr);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  if (!url.pathname || url.pathname === "/") {
    url.pathname = "/ws";
  } else if (!url.pathname.endsWith("/ws")) {
    url.pathname = `${url.pathname.replace(/\/$/, "")}/ws`;
  }
  return url.toString();
}

function ensureRatio(value, label) {
  if (!Number.isFinite(value) || value < 0 || value > 1) {
    throw new Error(`${label} must be a number between 0 and 1`);
  }
}

function ensurePositiveInt(value, label) {
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(`${label} must be a positive integer`);
  }
}

function sleep(ms) {
  if (!Number.isFinite(ms) || ms <= 0) {
    return Promise.resolve();
  }
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function safeClose(app) {
  await Promise.race([
    app.close().catch(() => {}),
    sleep(1000),
  ]);
}

async function runNativeScrollCheck(args, repoRoot) {
  const scriptPath = "/tmp/native-scroll-check.mjs";
  try {
    await fs.stat(scriptPath);
  } catch (_) {
    console.warn(
      `[ghosting-check] ${scriptPath} not found; skipping scroll capture.`,
    );
    return;
  }
  const command = "node";
  const nodeArgs = ["--experimental-websocket", scriptPath];
  if (args.httpUrl) {
    nodeArgs.push("--http", args.httpUrl);
  }
  if (args.wsUrl) {
    nodeArgs.push("--ws", args.wsUrl);
  }
  if (Number.isFinite(args.delayMs)) {
    nodeArgs.push("--delay-ms", String(args.delayMs));
  }
  const child = spawn(command, nodeArgs, {
    cwd: repoRoot,
    stdio: "inherit",
  });
  const code = await new Promise((resolve) => {
    child.on("close", resolve);
  });
  if (code !== 0) {
    throw new Error(`/tmp/native-scroll-check.mjs exited with ${code}`);
  }
}

async function ensureWebSocketGlobal() {
  if (typeof WebSocket !== "undefined") {
    return;
  }
  try {
    const undici = await import("undici");
    if (undici.WebSocket) {
      globalThis.WebSocket = undici.WebSocket;
      return;
    }
  } catch (_) {
    // ignore
  }
  try {
    const ws = await import("ws");
    if (ws.WebSocket) {
      globalThis.WebSocket = ws.WebSocket;
      return;
    }
    if (typeof ws.default === "function") {
      globalThis.WebSocket = ws.default;
      return;
    }
  } catch (_) {
    // ignore
  }
  throw new Error(
    "WebSocket is not available. Install ws or use Node 20+.",
  );
}

function findNodeById(node, id) {
  if (!node) {
    return null;
  }
  if (node.id === id) {
    return node;
  }
  if (!node.children) {
    return null;
  }
  for (const child of node.children) {
    const found = findNodeById(child, id);
    if (found) {
      return found;
    }
  }
  return null;
}

function findNodeByPrefix(node, prefix) {
  if (!node) {
    return null;
  }
  if (typeof node.id === "string" && node.id.startsWith(prefix)) {
    return node;
  }
  if (!node.children) {
    return null;
  }
  for (const child of node.children) {
    const found = findNodeByPrefix(child, prefix);
    if (found) {
      return found;
    }
  }
  return null;
}

async function getThreadListBounds(page) {
  const tree = await page.rpc("automation.tree.snapshot", {});
  const node = findNodeById(tree, "thread-list");
  if (!node || !node.bounds) {
    throw new Error("thread-list bounds not found in automation tree");
  }
  const { x, y, width, height } = node.bounds;
  return { x, y, width, height };
}

async function maybeSelectWorkspace(page) {
  const tree = await page.rpc("automation.tree.snapshot", {});
  const node = findNodeByPrefix(tree, "workspace-item-");
  if (!node || !node.bounds) {
    return false;
  }
  const { x, y, width, height } = node.bounds;
  await page
    .rpc("ctx.input.click", {
      x: x + width * 0.5,
      y: y + height * 0.5,
      button: "left",
    })
    .catch(() => {});
  await sleep(500);
  return true;
}

async function waitForThreadList(page, timeoutMs) {
  const threadList = page.locator("#thread-list");
  const start = Date.now();
  let attemptedWorkspace = false;
  while (Date.now() - start < timeoutMs) {
    try {
      if (await threadList.isVisible()) {
        return;
      }
    } catch (_) {
      // ignore
    }
    if (!attemptedWorkspace) {
      attemptedWorkspace = await maybeSelectWorkspace(page);
    }
    await page.rpc("ctx.sessions.select", { index: 0 }).catch(() => {});
    await sleep(500);
  }
  throw new Error("Thread list never became visible");
}

async function scrollBy(page, point, deltaY, delayMs) {
  await page.mouse.wheel(0, deltaY, { x: point.x, y: point.y });
  await page.rpc("ctx.wait.idle", { frames: 2, timeout_ms: 1500 }).catch(() => {});
  await sleep(delayMs);
}

async function scrollToTop(page, point, steps, delta, delayMs) {
  for (let i = 0; i < steps; i += 1) {
    await scrollBy(page, point, -delta, delayMs);
  }
}

async function captureGhostTopImages({
  page,
  bounds,
  delayMs,
  scrollLines,
  scrollSteps,
  topPath,
  returnPath,
}) {
  const center = {
    x: bounds.x + bounds.width * 0.5,
    y: bounds.y + bounds.height * 0.5,
  };
  await page.rpc("ctx.input.click", { x: center.x, y: center.y, button: "left" }).catch(() => {});
  await sleep(delayMs);

  const delta = scrollLines * 120;
  await scrollToTop(page, center, scrollSteps, delta, delayMs);
  await page.screenshot({ path: topPath });

  for (let i = 0; i < scrollSteps; i += 1) {
    await scrollBy(page, center, delta, delayMs);
  }
  await scrollToTop(page, center, scrollSteps + 2, delta, delayMs);
  await page.screenshot({ path: returnPath });
}

function clampBounds(bounds, png) {
  const x = Math.max(0, Math.floor(bounds.x));
  const y = Math.max(0, Math.floor(bounds.y));
  const maxWidth = Math.max(0, png.width - x);
  const maxHeight = Math.max(0, png.height - y);
  const width = Math.min(maxWidth, Math.ceil(bounds.width));
  const height = Math.min(maxHeight, Math.ceil(bounds.height));
  if (width <= 0 || height <= 0) {
    throw new Error("Thread list bounds are outside screenshot bounds");
  }
  return { x, y, width, height };
}

function cropPng(png, bounds) {
  const cropped = new PNG({ width: bounds.width, height: bounds.height });
  PNG.bitblt(png, cropped, bounds.x, bounds.y, bounds.width, bounds.height, 0, 0);
  return cropped;
}

async function diffGhosting({
  topPath,
  returnPath,
  bounds,
  outDir,
  threshold,
  pixelThreshold,
}) {
  const [topRaw, returnRaw] = await Promise.all([
    fs.readFile(topPath),
    fs.readFile(returnPath),
  ]);
  const topPng = PNG.sync.read(topRaw);
  const returnPng = PNG.sync.read(returnRaw);

  if (topPng.width !== returnPng.width || topPng.height !== returnPng.height) {
    throw new Error("Screenshot sizes do not match");
  }

  const clamped = clampBounds(bounds, topPng);
  const topCrop = cropPng(topPng, clamped);
  const returnCrop = cropPng(returnPng, clamped);
  const diff = new PNG({ width: clamped.width, height: clamped.height });

  const diffPixels = pixelmatch(
    topCrop.data,
    returnCrop.data,
    diff.data,
    clamped.width,
    clamped.height,
    { threshold: pixelThreshold },
  );
  const totalPixels = clamped.width * clamped.height;
  const diffRatio = totalPixels === 0 ? 0 : diffPixels / totalPixels;
  const pass = diffRatio <= threshold;

  await fs.mkdir(outDir, { recursive: true });
  const cropTopPath = path.join(outDir, "ghost-top-crop.png");
  const cropReturnPath = path.join(outDir, "ghost-top-return-crop.png");
  const diffPath = path.join(outDir, "ghost-diff.png");
  await Promise.all([
    fs.writeFile(cropTopPath, PNG.sync.write(topCrop)),
    fs.writeFile(cropReturnPath, PNG.sync.write(returnCrop)),
    fs.writeFile(diffPath, PNG.sync.write(diff)),
  ]);

  const summary = {
    topPath,
    returnPath,
    cropTopPath,
    cropReturnPath,
    diffPath,
    bounds: clamped,
    diffPixels,
    diffRatio,
    threshold,
    pixelThreshold,
    pass,
  };
  const summaryPath = path.join(outDir, "ghosting-summary.json");
  await fs.writeFile(summaryPath, `${JSON.stringify(summary, null, 2)}\n`);

  console.log(
    `Ghosting diff ratio ${diffRatio.toFixed(6)} (threshold ${threshold}).`,
  );
  console.log(`Diff: ${diffPath}`);
  console.log(`Summary: ${summaryPath}`);
  if (!pass) {
    process.exitCode = 1;
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log(USAGE.trim());
    return;
  }

  if (!args.httpUrl) {
    args.httpUrl = normalizeAddr(args.addr);
  }
  if (!args.wsUrl) {
    args.wsUrl = wsUrlFromHttp(args.httpUrl);
  }

  ensureRatio(args.threshold, "threshold");
  ensureRatio(args.pixelThreshold, "pixel-threshold");
  ensurePositiveInt(args.scrollLines, "scroll-lines");
  ensurePositiveInt(args.scrollSteps, "scroll-steps");

  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const repoRoot = path.resolve(scriptDir, "../../../..");
  const shotsDir = path.resolve(args.shotsDir);
  const outDir = path.resolve(args.outDir ?? path.join(shotsDir, "ghosting-check"));

  await fs.mkdir(outDir, { recursive: true });

  await ensureWebSocketGlobal();
  const driverUrl = pathToFileURL(
    path.join(repoRoot, "core", "apps", "native", "scripts", "native-driver.mjs"),
  );
  const { connect } = await import(driverUrl.href);
  const app = await connect({ httpUrl: args.httpUrl, wsUrl: args.wsUrl });
  const page = app.page;

  try {
    await page.rpc("ctx.ready", { timeoutMs: 30000 }).catch(() => {});
    await waitForThreadList(page, 30000);

    if (!args.skipCapture && !args.useExisting) {
      await runNativeScrollCheck(args, repoRoot);
      await waitForThreadList(page, 30000);
    }

    const bounds = await getThreadListBounds(page);

    const topPath = path.join(outDir, "ghost-top.png");
    const returnPath = path.join(outDir, "ghost-top-return.png");

    if (args.useExisting) {
      await Promise.all([fs.stat(topPath), fs.stat(returnPath)]);
      await diffGhosting({
        topPath,
        returnPath,
        bounds,
        outDir,
        threshold: args.threshold,
        pixelThreshold: args.pixelThreshold,
      });
    } else {
      await captureGhostTopImages({
        page,
        bounds,
        delayMs: args.delayMs,
        scrollLines: args.scrollLines,
        scrollSteps: args.scrollSteps,
        topPath,
        returnPath,
      });
      await diffGhosting({
        topPath,
        returnPath,
        bounds,
        outDir,
        threshold: args.threshold,
        pixelThreshold: args.pixelThreshold,
      });
    }
  } finally {
    await safeClose(app);
  }
}

main()
  .then(() => {
    const code = Number.isInteger(process.exitCode) ? process.exitCode : 0;
    process.exit(code);
  })
  .catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
