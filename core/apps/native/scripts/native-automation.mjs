#!/usr/bin/env node

const DEFAULT_ADDR = "http://127.0.0.1:6160";
const DEFAULT_READY_TIMEOUT_MS = 30000;
const DEFAULT_DELAY_MS = 150;
const DEFAULT_REQUEST_TIMEOUT_MS = 15000;

function printHelp() {
  console.log(`Usage: node core/apps/native/scripts/native-automation.mjs [options]

Options:
  --addr <host:port|url>     Automation server address (default: ${DEFAULT_ADDR})
  --ready-timeout-ms <ms>    Timeout for /ready (default: ${DEFAULT_READY_TIMEOUT_MS})
  --delay-ms <ms>            Delay after focus before screenshot (default: ${DEFAULT_DELAY_MS})
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
    delayMs: DEFAULT_DELAY_MS,
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
    if (arg === "--delay-ms") {
      if (i + 1 >= argv.length) {
        throw new Error("--delay-ms requires a value");
      }
      args.delayMs = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--delay-ms=")) {
      args.delayMs = arg.slice("--delay-ms=".length);
      continue;
    }

    throw new Error(`Unknown argument: ${arg}`);
  }

  args.addr = normalizeAddr(args.addr);
  args.readyTimeoutMs = toPositiveInt(args.readyTimeoutMs, "--ready-timeout-ms");
  args.delayMs = toNonNegativeInt(args.delayMs, "--delay-ms");
  return args;
}

function sleep(ms) {
  if (ms <= 0) {
    return Promise.resolve();
  }
  return new Promise((resolve) => setTimeout(resolve, ms));
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

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.showHelp) {
    printHelp();
    return;
  }

  const targets = [
    { target: "sessions_pane", name: "sessions-pane" },
    { target: "diff_pane", name: "diff-pane" },
    { target: "artifacts_pane", name: "artifacts-pane" },
    { target: "terminal_panel", name: "terminal-panel" },
    { target: "main", name: "web-workbench-task-list" },
    { target: "archived_tasks", name: "web-workbench-archived-tasks" },
    { target: "composer_new_task", name: "web-workbench-new-task" },
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

    for (const { target, name } of targets) {
      await callJson(args.addr, "/focus", {
        method: "POST",
        body: { target },
      });
      if (target === "archived_tasks") {
        try {
          await callJson(args.addr, "/wait", {
            method: "POST",
            body: { target: "archived_loaded", timeout_ms: archivedWaitMs },
            timeoutMs: archivedWaitMs + 10000,
          });
        } catch (err) {
          console.error(`Wait for archived tasks failed: ${err.message}`);
        }
      }
      await sleep(args.delayMs);
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
