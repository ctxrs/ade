#!/usr/bin/env node

import { connect } from "./native-driver.mjs";

const DEFAULT_ADDR = "http://127.0.0.1:6160";
const DEFAULT_READY_TIMEOUT_MS = 30000;
const DEFAULT_SETTLE_MS = 150;
const DEFAULT_REQUEST_TIMEOUT_MS = 15000;

function printHelp() {
  console.log(`Usage: node core/apps/native/scripts/native-automation.mjs [options]

Options:
  --addr <host:port|url>     Automation server address (default: ${DEFAULT_ADDR})
  --ready-timeout-ms <ms>    Timeout for /ready (default: ${DEFAULT_READY_TIMEOUT_MS})
  --settle-ms <ms>           Wait for a stable render before screenshot (default: ${DEFAULT_SETTLE_MS})
  --delay-ms <ms>            Deprecated alias for --settle-ms
  --new-task-text <text>     Optional text to type in the new task composer
  -h, --help                 Show this help
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
    newTaskText: null,
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
    if (arg === "--delay-ms") {
      if (i + 1 >= argv.length) {
        throw new Error("--delay-ms requires a value");
      }
      args.settleMs = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--delay-ms=")) {
      args.settleMs = arg.slice("--delay-ms=".length);
      continue;
    }
    if (arg === "--new-task-text") {
      if (i + 1 >= argv.length) {
        throw new Error("--new-task-text requires a value");
      }
      args.newTaskText = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--new-task-text=")) {
      args.newTaskText = arg.slice("--new-task-text=".length);
      continue;
    }

    throw new Error(`Unknown argument: ${arg}`);
  }

  args.addr = normalizeAddr(args.addr);
  args.readyTimeoutMs = toPositiveInt(args.readyTimeoutMs, "--ready-timeout-ms");
  args.settleMs = toNonNegativeInt(args.settleMs, "--settle-ms");
  return args;
}

async function callJson(baseUrl, path, options = {}) {
  const { method = "GET", body, timeoutMs = DEFAULT_REQUEST_TIMEOUT_MS } = options;
  const url = new URL(path, baseUrl);
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
    let payload;
    if (text) {
      try {
        payload = JSON.parse(text);
      } catch (err) {
        throw new Error(`invalid JSON response: ${err.message}`);
      }
    }

    if (!response.ok) {
      throw new Error(`HTTP ${response.status} ${response.statusText}`);
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

async function typeNewTaskText(httpUrl, text) {
  if (!text) {
    return;
  }
  if (typeof WebSocket === "undefined") {
    throw new Error(
      "--new-task-text requires WebSocket support (run node with --experimental-websocket)",
    );
  }
  const app = await connect({ httpUrl });
  try {
    await app.page.keyboard.press("escape");
    await app.page.locator("#composer-input").click();
    await app.page.locator("#composer-input").type(String(text));
    const status = await app.rpc("automation.composer.status");
    console.log("Composer status after type:", status);
    const tree = await app.rpc("automation.tree.snapshot");
    const composerNode = findNodeById(tree, "composer-input");
    console.log("Composer node after type:", composerNode);
  } finally {
    await app.close();
  }
}

function findNodeById(node, id) {
  if (!node || typeof node !== "object") {
    return null;
  }
  if (node.id === id) {
    return node;
  }
  if (Array.isArray(node.children)) {
    for (const child of node.children) {
      const found = findNodeById(child, id);
      if (found) {
        return found;
      }
    }
  }
  return null;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.showHelp) {
    printHelp();
    return;
  }

  const targets = [
    { target: "new_task", name: "web-workbench-new-task" },
    { target: "sessions_pane", name: "sessions-pane" },
    { target: "diff_pane", name: "diff-pane" },
    { target: "artifacts_pane", name: "artifacts-pane" },
    { target: "terminal_panel", name: "terminal-panel" },
    { target: "main", name: "web-workbench-task-list" },
    { target: "archived_tasks", name: "web-workbench-archived-tasks" },
    { target: "composer_provider_menu", name: "web-workbench-harness-menu" },
    { target: "composer_model_menu", name: "web-workbench-model-menu" },
  ];
  const archivedWaitMs = 60000;

  let runError = null;
  try {
    const readyTimeout = Math.max(args.readyTimeoutMs + 5000, DEFAULT_REQUEST_TIMEOUT_MS);
    await callJson(args.addr, `/ready?timeout_ms=${args.readyTimeoutMs}`, {
      timeoutMs: readyTimeout,
    });

    if (args.newTaskText) {
      const waitTimeout = DEFAULT_REQUEST_TIMEOUT_MS;
      await callJson(args.addr, "/focus", {
        method: "POST",
        body: { target: "new_task" },
      });
      try {
        await callJson(args.addr, "/wait", {
          method: "POST",
          body: { settle_ms: args.settleMs, timeout_ms: waitTimeout },
          timeoutMs: waitTimeout + 10000,
        });
      } catch (err) {
        console.error(`Wait for render idle failed: ${err.message}`);
      }
      await typeNewTaskText(args.addr, args.newTaskText);
      try {
        await callJson(args.addr, "/wait", {
          method: "POST",
          body: { settle_ms: args.settleMs, timeout_ms: waitTimeout },
          timeoutMs: waitTimeout + 10000,
        });
      } catch (err) {
        console.error(`Wait for render idle failed: ${err.message}`);
      }
      await callJson(args.addr, "/screenshot", {
        method: "POST",
        body: { name: "web-workbench-new-task-typed" },
      });
    }
    for (const { target, name } of targets) {
      await callJson(args.addr, "/focus", {
        method: "POST",
        body: { target },
      });
      const waitTimeout =
        target === "archived_tasks" ? archivedWaitMs : DEFAULT_REQUEST_TIMEOUT_MS;
      try {
        await callJson(args.addr, "/wait", {
          method: "POST",
          body: { settle_ms: args.settleMs, timeout_ms: waitTimeout },
          timeoutMs: waitTimeout + 10000,
        });
      } catch (err) {
        console.error(`Wait for render idle failed: ${err.message}`);
      }
      await callJson(args.addr, "/screenshot", {
        method: "POST",
        body: { name },
      });
    }
  } catch (err) {
    runError = err;
  }

  try {
    await callJson(args.addr, "/exit", { method: "POST", body: {} });
  } catch (err) {
    if (!runError) {
      runError = err;
    } else {
      console.error(`Exit request failed: ${err.message}`);
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
