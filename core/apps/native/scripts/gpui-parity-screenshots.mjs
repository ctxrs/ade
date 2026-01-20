#!/usr/bin/env node

import { connect, expect } from "./native-driver.mjs";

const DEFAULT_ADDR = "http://127.0.0.1:6160";
const DEFAULT_READY_TIMEOUT_MS = 30000;
const DEFAULT_SETTLE_MS = 150;

function log(msg) {
  console.log(`[gpui-parity] ${msg}`);
}

function printHelp() {
  console.log(`Usage: node core/apps/native/scripts/gpui-parity-screenshots.mjs [options]

Options:
  --addr <host:port|url>       Automation server address (default: ${DEFAULT_ADDR})
  --ready-timeout-ms <ms>      Timeout for /ready (default: ${DEFAULT_READY_TIMEOUT_MS})
  --settle-ms <ms>             Wait for a stable render before screenshot (default: ${DEFAULT_SETTLE_MS})
  --workspace-name <name>      Workspace display name (default: $CTX_PARITY_WORKSPACE_NAME || ws-parity)
  --task-text <text>           Task title text to select (default: $CTX_PARITY_TASK_TEXT || hello)
  -h, --help                   Show this help
`);
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

function toPositiveInt(value, flag) {
  const number = Number(value);
  if (!Number.isFinite(number) || number <= 0) {
    throw new Error(`${flag} must be a positive number`);
  }
  return Math.trunc(number);
}

function toNonNegativeInt(value, flag) {
  const number = Number(value);
  if (!Number.isFinite(number) || number < 0) {
    throw new Error(`${flag} must be a non-negative number`);
  }
  return Math.trunc(number);
}

function parseArgs(argv) {
  const args = {
    addr: DEFAULT_ADDR,
    readyTimeoutMs: DEFAULT_READY_TIMEOUT_MS,
    settleMs: DEFAULT_SETTLE_MS,
    workspaceName: process.env.CTX_PARITY_WORKSPACE_NAME ?? "ws-parity",
    taskText: process.env.CTX_PARITY_TASK_TEXT ?? "hello",
    showHelp: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      args.showHelp = true;
      continue;
    }
    if (arg === "--addr") {
      if (i + 1 >= argv.length) {
        throw new Error("--addr requires a value");
      }
      args.addr = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--addr=")) {
      args.addr = arg.slice("--addr=".length);
      continue;
    }
    if (arg === "--ready-timeout-ms") {
      if (i + 1 >= argv.length) {
        throw new Error("--ready-timeout-ms requires a value");
      }
      args.readyTimeoutMs = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--ready-timeout-ms=")) {
      args.readyTimeoutMs = arg.slice("--ready-timeout-ms=".length);
      continue;
    }
    if (arg === "--settle-ms") {
      if (i + 1 >= argv.length) {
        throw new Error("--settle-ms requires a value");
      }
      args.settleMs = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--settle-ms=")) {
      args.settleMs = arg.slice("--settle-ms=".length);
      continue;
    }
    if (arg === "--workspace-name") {
      if (i + 1 >= argv.length) {
        throw new Error("--workspace-name requires a value");
      }
      args.workspaceName = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--workspace-name=")) {
      args.workspaceName = arg.slice("--workspace-name=".length);
      continue;
    }
    if (arg === "--task-text") {
      if (i + 1 >= argv.length) {
        throw new Error("--task-text requires a value");
      }
      args.taskText = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--task-text=")) {
      args.taskText = arg.slice("--task-text=".length);
      continue;
    }

    throw new Error(`Unknown argument: ${arg}`);
  }

  args.addr = normalizeAddr(args.addr);
  args.readyTimeoutMs = toPositiveInt(args.readyTimeoutMs, "--ready-timeout-ms");
  args.settleMs = toNonNegativeInt(args.settleMs, "--settle-ms");
  return args;
}

async function callJson(baseUrl, reqPath, options = {}) {
  const { method = "POST", body, timeoutMs = 15000 } = options;
  const url = new URL(reqPath, baseUrl);
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeoutMs);
  const headers = { accept: "application/json" };
  if (body !== undefined) {
    headers["content-type"] = "application/json";
  }

  try {
    const response = await fetch(url, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: controller.signal,
    });
    const text = await response.text();
    let payload = null;
    if (text) {
      try {
        payload = JSON.parse(text);
      } catch {
        payload = null;
      }
    }

    if (!response.ok) {
      const detail = payload?.error || text || `HTTP ${response.status} ${response.statusText}`;
      throw new Error(detail);
    }
    if (!payload || payload.ok !== true) {
      throw new Error(payload?.error || "unknown error");
    }
    return payload.result;
  } catch (err) {
    throw new Error(`Request ${method} ${url} failed: ${err.message}`);
  } finally {
    clearTimeout(timeoutId);
  }
}

async function waitIdle(addr, settleMs) {
  await callJson(addr, "/wait", {
    body: { settle_ms: settleMs, timeout_ms: 15000 },
    timeoutMs: 30000,
  }).catch(() => {});
}

async function screenshot(addr, relPath) {
  log(`screenshot ${relPath}`);
  await callJson(addr, "/screenshot", {
    body: { path: relPath },
    timeoutMs: 30000,
  });
}

async function focus(addr, target) {
  log(`focus ${target}`);
  await callJson(addr, "/focus", {
    body: { target },
  });
}

function walk(node, f) {
  if (!node || typeof node !== "object") return;
  f(node);
  if (Array.isArray(node.children)) {
    for (const child of node.children) {
      walk(child, f);
    }
  }
}

async function dumpTree(app, label) {
  try {
    const tree = await app.rpc("automation.tree.snapshot");
    const ids = new Set();
    const roles = new Set();
    const interesting = [];
    walk(tree, (node) => {
      if (node.id) ids.add(String(node.id));
      if (node.role) roles.add(String(node.role));
      const id = node.id ? String(node.id) : "";
      const name = node.name ? String(node.name) : "";
      if (/(workspace|workspaces|settings|composer|sidebar|session|app-shell)/i.test(id) || /(Workspaces|Settings|New Task)/i.test(name)) {
        interesting.push({ id, role: node.role, name });
      }
    });
    log(`${label}: ids=${Array.from(ids).slice(0, 50).join(", ")}`);
    log(`${label}: roles=${Array.from(roles).slice(0, 50).join(", ")}`);
    log(`${label}: interesting=${JSON.stringify(interesting.slice(0, 50))}`);
  } catch (err) {
    log(`${label}: failed to dump tree (${err.message})`);
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.showHelp) {
    printHelp();
    return;
  }

  let runError = null;
  try {
    log(`addr=${args.addr}`);
    await callJson(args.addr, `/ready?timeout_ms=${args.readyTimeoutMs}`, {
      method: "GET",
      timeoutMs: Math.max(args.readyTimeoutMs + 5000, 20000),
    });

    const app = await connect({ httpUrl: args.addr });
    try {
      log("waiting for workspaces list");
      await expect(app.page.getByRole("list", { name: "Workspaces" })).toBeVisible({
        timeoutMs: args.readyTimeoutMs,
      });
      await waitIdle(args.addr, args.settleMs);
      await screenshot(args.addr, "launcher/native.png");

      log(`waiting for workspace button '${args.workspaceName}'`);
      const workspace = app.page.getByRole("button", { name: args.workspaceName });
      await expect(workspace).toBeVisible({ timeoutMs: args.readyTimeoutMs });
      log("select workspace 0");
      await callJson(args.addr, "/select_workspace", { body: { index: 0 } });
      await waitIdle(args.addr, args.settleMs);

      // New task
      await focus(args.addr, "new_task");
      await waitIdle(args.addr, args.settleMs);
      await screenshot(args.addr, "new-task/native.png");

      // Harness menu (open)
      await focus(args.addr, "composer_provider_menu");
      await waitIdle(args.addr, args.settleMs);
      await screenshot(args.addr, "new-task/menu/harness/native.png");
      // Dismiss the menu so subsequent focus/scroll states aren't occluded.
      await focus(args.addr, "dismiss_menus");
      await waitIdle(args.addr, args.settleMs);

      // New task focus state
      log("focus composer input");
      await app.page.locator("#composer-input").click();
      await waitIdle(args.addr, args.settleMs);
      await screenshot(args.addr, "new-task/focus/composer/native.png");

      // Sidebar search focus
      await focus(args.addr, "task_search");
      await waitIdle(args.addr, args.settleMs);
      await screenshot(args.addr, "new-task/focus/search/native.png");

      // Archived empty
      await focus(args.addr, "archived_tasks");
      await waitIdle(args.addr, args.settleMs);
      await screenshot(args.addr, "archived/empty/native.png");
      // Restore collapsed state for subsequent screenshots.
      await focus(args.addr, "archived_tasks_close");
      await waitIdle(args.addr, args.settleMs);

      // Active session (assumes exactly one task was created in web)
      log("select session 0");
      await app.page.clickSession(0);
      await waitIdle(args.addr, args.settleMs);
      await screenshot(args.addr, "active-session/native.png");

      // Settings
      log("open settings");
      await callJson(args.addr, "/route", { body: { route: "settings" } });
      await expect(app.page.locator("#settings-pane")).toBeVisible({
        timeoutMs: args.readyTimeoutMs,
      });
      await waitIdle(args.addr, args.settleMs);

      await screenshot(args.addr, "settings/native.png");

      log("focus settings search");
      await app.page.locator("#settings-search-input").click();
      await waitIdle(args.addr, args.settleMs);
      await screenshot(args.addr, "settings/focus/search/native.png");
    } catch (err) {
      await dumpTree(app, "on-error");
      throw err;
    } finally {
      await app.close();
    }
  } catch (err) {
    runError = err;
  }

  try {
    await callJson(args.addr, "/exit", { body: {} });
  } catch (err) {
    if (!runError) {
      runError = err;
    }
  }

  if (runError) {
    console.error(runError.message);
    process.exitCode = 1;
  }
}

main().catch((err) => {
  console.error(err.message);
  process.exitCode = 1;
});
