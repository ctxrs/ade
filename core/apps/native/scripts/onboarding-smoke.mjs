#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

import { connect, expect } from "./native-driver.mjs";

const DEFAULT_HTTP_URL = process.env.CTX_NATIVE_HTTP ?? "http://127.0.0.1:6160";
const DEFAULT_WS_URL = process.env.CTX_NATIVE_WS ?? "ws://127.0.0.1:6160/ws";
const DEFAULT_WORKSPACE = process.env.CTX_NATIVE_WORKSPACE ?? "";
const DEFAULT_READY_TIMEOUT_MS = 30_000;
const DEFAULT_SETTLE_MS = 150;
const DEFAULT_PROGRESS_TIMEOUT_MS = 90_000;

function printHelp() {
  console.log(`Native onboarding smoke test (GPUI automation).

Usage:
  node core/apps/native/scripts/onboarding-smoke.mjs [options]

Options:
  --http <url>               Automation HTTP URL (default: ${DEFAULT_HTTP_URL})
  --ws <url>                 Automation WS URL (default: ${DEFAULT_WS_URL})
  --workspace <path>         Git repo root to open (default: CTX_NATIVE_WORKSPACE)
  --ready-timeout-ms <ms>    Timeout for ctx.ready (default: ${DEFAULT_READY_TIMEOUT_MS})
  --settle-ms <ms>           Render settle wait (default: ${DEFAULT_SETTLE_MS})
  --progress-timeout-ms <ms> Wait for launcher progress (default: ${DEFAULT_PROGRESS_TIMEOUT_MS})
  -h, --help                 Show this help text
`);
}

function parseArgs(argv) {
  const args = {
    httpUrl: DEFAULT_HTTP_URL,
    wsUrl: DEFAULT_WS_URL,
    workspace: DEFAULT_WORKSPACE,
    readyTimeoutMs: DEFAULT_READY_TIMEOUT_MS,
    settleMs: DEFAULT_SETTLE_MS,
    progressTimeoutMs: DEFAULT_PROGRESS_TIMEOUT_MS,
    help: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      args.help = true;
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
    if (arg === "--workspace") {
      args.workspace = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--workspace=")) {
      args.workspace = arg.slice("--workspace=".length);
      continue;
    }
    if (arg === "--ready-timeout-ms") {
      args.readyTimeoutMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg.startsWith("--ready-timeout-ms=")) {
      args.readyTimeoutMs = Number(arg.slice("--ready-timeout-ms=".length));
      continue;
    }
    if (arg === "--settle-ms") {
      args.settleMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg.startsWith("--settle-ms=")) {
      args.settleMs = Number(arg.slice("--settle-ms=".length));
      continue;
    }
    if (arg === "--progress-timeout-ms") {
      args.progressTimeoutMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg.startsWith("--progress-timeout-ms=")) {
      args.progressTimeoutMs = Number(arg.slice("--progress-timeout-ms=".length));
      continue;
    }
    throw new Error(`Unknown argument: ${arg}`);
  }

  return args;
}

function skip(message) {
  console.log(`skipped: ${message}`);
  process.exit(0);
}

function ensureWorkspace(pathStr) {
  const resolved = path.resolve(String(pathStr));
  if (!fs.existsSync(resolved)) {
    throw new Error(`workspace path does not exist: ${resolved}`);
  }
  if (!fs.existsSync(path.join(resolved, ".git"))) {
    throw new Error(`workspace path is not a git repo root: ${resolved}`);
  }
  return resolved;
}

async function waitIdle(app, settleMs, timeoutMs) {
  try {
    await app.rpc("ctx.wait.idle", {
      settle_ms: settleMs,
      timeout_ms: timeoutMs,
    });
  } catch (err) {
    console.warn(`wait idle failed: ${err.message}`);
  }
}

async function clearInput(page) {
  const modifier = process.platform === "darwin" ? "cmd" : "ctrl";
  await page.keyboard.press(`${modifier}-a`);
  await page.keyboard.press("backspace");
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    printHelp();
    return;
  }

  if (!args.workspace) {
    skip("set --workspace or CTX_NATIVE_WORKSPACE to a git repo root");
  }

  const workspacePath = ensureWorkspace(args.workspace);

  let app;
  try {
    app = await connect({ httpUrl: args.httpUrl, wsUrl: args.wsUrl });
  } catch (err) {
    skip(`native automation harness unavailable (${err.message})`);
  }

  try {
    await app.rpc("ctx.ready", { timeout_ms: args.readyTimeoutMs });

    const page = app.page;
    await waitIdle(app, args.settleMs, 10_000);

    const launcherCard = page.locator("#launcher-card");
    try {
      await expect(launcherCard).toBeVisible({ timeoutMs: 8_000 });
    } catch (err) {
      const composer = page.locator("#composer-input");
      try {
        await expect(composer).toBeVisible({ timeoutMs: 2_000 });
        skip("launcher not visible; set CTX_DATA_DIR to a fresh dir for onboarding");
      } catch (_) {
        throw err;
      }
    }

    const nextButton = page.locator("#launcher-next");
    await nextButton.click();
    await waitIdle(app, args.settleMs, 10_000);

    const hostMode = page.locator("#launcher-mode-host");
    await expect(hostMode).toBeVisible({ timeoutMs: 10_000 });
    await hostMode.click();
    await waitIdle(app, args.settleMs, 10_000);

    await nextButton.click();
    await waitIdle(app, args.settleMs, 10_000);

    const workspaceInput = page.locator("#launcher-workspace-input");
    await expect(workspaceInput).toBeVisible({ timeoutMs: 10_000 });
    await workspaceInput.click();
    await clearInput(page);
    await app.rpc("automation.keyboard.type", { text: workspacePath });
    await waitIdle(app, args.settleMs, 10_000);

    await nextButton.click();
    await waitIdle(app, args.settleMs, 10_000);

    const openWorkbench = page.locator("#launcher-open-workbench");
    await expect(openWorkbench).toBeVisible({
      timeoutMs: args.progressTimeoutMs,
    });
    await openWorkbench.click();
    await waitIdle(app, args.settleMs, 10_000);

    const composer = page.locator("#composer-input");
    await expect(composer).toBeVisible({ timeoutMs: 20_000 });
  } finally {
    if (app) {
      await app.close();
    }
  }
}

main().catch((err) => {
  console.error(err.message);
  process.exitCode = 1;
});
