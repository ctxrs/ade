#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";

import { connect, expect } from "./native-driver.mjs";

const DEFAULT_ADDR = "http://127.0.0.1:6160";
const DEFAULT_READY_TIMEOUT_MS = 30000;
const DEFAULT_SETTLE_MS = 150;
const DAEMON_REQUEST_TIMEOUT_MS = 8000;
const AUTH_FILENAME = "daemon_auth.json";

function log(msg) {
  console.log(`[gpui-parity] ${msg}`);
}

function printHelp() {
  console.log(`Usage: node core/apps/native/scripts/gpui-parity-screenshots.mjs [options]

Options:
  --addr <host:port|url>       Automation server address (default: ${DEFAULT_ADDR})
  --ready-timeout-ms <ms>      Timeout for /ready (default: ${DEFAULT_READY_TIMEOUT_MS})
  --settle-ms <ms>             Wait for a stable render before screenshot (default: ${DEFAULT_SETTLE_MS})
  --daemon-url <url>           Daemon base URL (default: $CTX_DAEMON_URL)
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
    daemonUrl: process.env.CTX_DAEMON_URL ?? "",
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
    if (arg === "--daemon-url") {
      if (i + 1 >= argv.length) {
        throw new Error("--daemon-url requires a value");
      }
      args.daemonUrl = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--daemon-url=")) {
      args.daemonUrl = arg.slice("--daemon-url=".length);
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
  args.daemonUrl = args.daemonUrl ? normalizeAddr(args.daemonUrl) : "";
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

function sleep(ms) {
  if (ms <= 0) return Promise.resolve();
  return new Promise((resolve) => setTimeout(resolve, ms));
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

async function readDaemonAuthToken() {
  const dataDir = process.env.CTX_DATA_DIR || process.env.CTX_E2E_DATA_DIR;
  if (!dataDir) {
    return "";
  }
  const authPath = path.join(dataDir, AUTH_FILENAME);
  try {
    const raw = await readFile(authPath, "utf8");
    const parsed = JSON.parse(raw);
    const token = String(parsed?.token ?? "").trim();
    return token;
  } catch {
    return "";
  }
}

function idToString(value) {
  if (!value) return "";
  if (typeof value === "string") return value;
  if (typeof value === "object") {
    if (Object.prototype.hasOwnProperty.call(value, 0)) {
      return String(value[0] ?? "");
    }
    if (Object.prototype.hasOwnProperty.call(value, "0")) {
      return String(value["0"] ?? "");
    }
  }
  return String(value ?? "");
}

async function callDaemonJson(baseUrl, reqPath, options = {}) {
  const { method = "GET", body, timeoutMs = 15000, token = "" } = options;
  const url = new URL(reqPath, baseUrl);
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeoutMs);
  const headers = { accept: "application/json" };
  if (body !== undefined) {
    headers["content-type"] = "application/json";
  }
  if (token) {
    headers.authorization = `Bearer ${token}`;
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
    if (payload === null && text) {
      throw new Error("invalid JSON response");
    }
    return payload;
  } catch (err) {
    throw new Error(`Request ${method} ${url} failed: ${err.message}`);
  } finally {
    clearTimeout(timeoutId);
  }
}

async function resolveTaskSession({
  daemonUrl,
  token,
  workspaceName,
  taskText,
  timeoutMs,
}) {
  const requestTimeoutMs = Math.min(timeoutMs, DAEMON_REQUEST_TIMEOUT_MS);
  const workspaces = await callDaemonJson(daemonUrl, "/api/workspaces", {
    token,
    timeoutMs: requestTimeoutMs,
  });
  const workspace = (workspaces || []).find(
    (item) => String(item?.name ?? "") === workspaceName,
  );
  if (!workspace) {
    return null;
  }
  const workspaceId = idToString(workspace.id);
  if (!workspaceId) {
    return null;
  }
  const tasks = await callDaemonJson(daemonUrl, `/api/workspaces/${workspaceId}/tasks`, {
    token,
    timeoutMs: requestTimeoutMs,
  });
  const targetTitle = String(taskText ?? "").trim();
  const matching = (tasks || []).filter(
    (task) => String(task?.title ?? "").trim() === targetTitle,
  );
  if (matching.length === 0) {
    return null;
  }
  matching.sort((a, b) => String(b?.updated_at ?? "").localeCompare(String(a?.updated_at ?? "")));
  const task = matching[0];
  const taskId = idToString(task?.id);
  let sessionId = idToString(task?.primary_session_id);
  if (!sessionId && taskId) {
    const sessions = await callDaemonJson(daemonUrl, `/api/tasks/${taskId}/sessions`, {
      token,
      timeoutMs: requestTimeoutMs,
    });
    sessionId = idToString(sessions?.[0]?.id);
  }
  if (!sessionId) {
    return null;
  }
  return { workspaceId, taskId, sessionId };
}

async function deleteAllWorkspaceTerminals({ daemonUrl, token, workspaceId, timeoutMs }) {
  if (!daemonUrl || !token || !workspaceId) return;
  const start = Date.now();
  const requestTimeoutMs = Math.min(timeoutMs, DAEMON_REQUEST_TIMEOUT_MS);
  while (Date.now() - start < timeoutMs) {
    const terminals = await callDaemonJson(daemonUrl, `/api/workspaces/${workspaceId}/terminals`, {
      token,
      timeoutMs: requestTimeoutMs,
    }).catch(() => []);
    const list = Array.isArray(terminals) ? terminals : [];
    if (list.length === 0) {
      return;
    }
    await Promise.all(
      list.map((terminal) => {
        const terminalId = idToString(terminal?.id);
        if (!terminalId) return Promise.resolve();
        return callDaemonJson(daemonUrl, `/api/terminals/${terminalId}`, {
          method: "DELETE",
          token,
          timeoutMs: requestTimeoutMs,
        }).catch(() => {});
      }),
    );
    await sleep(200);
  }
  throw new Error("Timed out deleting workspace terminals via daemon");
}

async function listWorkspaceTerminalsViaDaemon({ daemonUrl, token, workspaceId, timeoutMs }) {
  if (!daemonUrl || !token || !workspaceId) return [];
  const requestTimeoutMs = Math.min(timeoutMs, DAEMON_REQUEST_TIMEOUT_MS);
  const terminals = await callDaemonJson(daemonUrl, `/api/workspaces/${workspaceId}/terminals`, {
    token,
    timeoutMs: requestTimeoutMs,
  }).catch(() => []);
  return Array.isArray(terminals) ? terminals : [];
}

async function createWorkspaceTerminalViaDaemon({ daemonUrl, token, workspaceId, cwd, timeoutMs }) {
  if (!daemonUrl || !token || !workspaceId) return "";
  const requestTimeoutMs = Math.min(timeoutMs, DAEMON_REQUEST_TIMEOUT_MS);
  const created = await callDaemonJson(daemonUrl, `/api/workspaces/${workspaceId}/terminals`, {
    method: "POST",
    token,
    timeoutMs: requestTimeoutMs,
    body: {
      cwd: cwd ?? null,
    },
  }).catch(() => null);
  return idToString(created?.id);
}

async function waitForSessionContent({
  daemonUrl,
  token,
  workspaceName,
  taskText,
  timeoutMs,
  sessionId: initialSessionId = "",
}) {
  if (!daemonUrl) {
    log("daemon URL not set; skipping session content wait");
    return "";
  }
  const start = Date.now();
  let sessionId = initialSessionId;
  const requestTimeoutMs = Math.min(timeoutMs, DAEMON_REQUEST_TIMEOUT_MS);
  while (Date.now() - start < timeoutMs) {
    if (!sessionId) {
      const resolved = await resolveTaskSession({
        daemonUrl,
        token,
        workspaceName,
        taskText,
        timeoutMs: requestTimeoutMs,
      }).catch(() => null);
      sessionId = resolved?.sessionId ?? "";
      if (!sessionId) {
        await sleep(250);
        continue;
      }
    }
    const snapshot = await callDaemonJson(
      daemonUrl,
      `/api/sessions/${sessionId}/snapshot?limit=50`,
      { token, timeoutMs: requestTimeoutMs },
    ).catch(() => null);
    const head = snapshot?.head ?? {};
    const messages = Array.isArray(head.messages) ? head.messages : [];
    const turns = Array.isArray(head.turns) ? head.turns : [];
    const hasAssistant = messages.some(
      (msg) =>
        msg?.role === "assistant" && String(msg?.content ?? "").trim().length > 0,
    );
    const hasCompletedTurn = turns.some((turn) => turn?.status === "completed");
    if (hasAssistant && hasCompletedTurn) {
      return sessionId;
    }
    await sleep(250);
  }
  throw new Error(
    `Timed out waiting for session content for task "${taskText}"`,
  );
}

const THREAD_ITEM_PREFIXES = ["assistant-", "tool-", "turn-status-"];

function hasRenderableBounds(node) {
  if (!node || typeof node !== "object") return false;
  const bounds = node.bounds;
  if (!bounds) return true;
  return Number(bounds.width ?? 0) > 1 && Number(bounds.height ?? 0) > 1;
}

function findNodeById(node, targetId) {
  if (!node || typeof node !== "object") return null;
  if (node.id === targetId) return node;
  if (Array.isArray(node.children)) {
    for (const child of node.children) {
      const found = findNodeById(child, targetId);
      if (found) return found;
    }
  }
  return null;
}

async function waitForNodeBounds(app, { timeoutMs, nodeId }) {
  const start = Date.now();
  let last = null;
  while (Date.now() - start < timeoutMs) {
    const tree = await app.rpc("automation.tree.snapshot").catch(() => null);
    if (!tree) {
      await sleep(200);
      continue;
    }
    const node = findNodeById(tree, nodeId);
    if (node && node.bounds) {
      last = node.bounds;
      const width = Number(node.bounds.width ?? 0);
      const height = Number(node.bounds.height ?? 0);
      if (width > 1 && height > 1) {
        return node.bounds;
      }
    }
    await sleep(200);
  }
  throw new Error(`Timed out waiting for node bounds for ${nodeId}: ${JSON.stringify({ last })}`);
}

async function scrollThreadForParity(app, { addr, settleMs, timeoutMs }) {
  const bounds = await waitForNodeBounds(app, { timeoutMs, nodeId: "thread-list" });
  const x = Number(bounds.x ?? 0) + Number(bounds.width ?? 0) / 2;
  const y = Number(bounds.y ?? 0) + Number(bounds.height ?? 0) / 2;
  for (let i = 0; i < 6; i += 1) {
    await app.page.mouse.wheel(0, -320, { x, y, precise: true }).catch(() => {});
    await sleep(50);
  }
  await waitIdle(addr, settleMs);
}

function findThreadItemId(node) {
  if (!node || typeof node !== "object") return "";
  const id = typeof node.id === "string" ? node.id : "";
  const visible = node.visible === true && hasRenderableBounds(node);
  if (visible && id && THREAD_ITEM_PREFIXES.some((prefix) => id.startsWith(prefix))) {
    return id;
  }
  if (Array.isArray(node.children)) {
    for (const child of node.children) {
      const found = findThreadItemId(child);
      if (found) return found;
    }
  }
  return "";
}

function findTaskRowId(node, taskText) {
  let match = "";
  walk(node, (child) => {
    if (match) return;
    const id = typeof child.id === "string" ? child.id : "";
    if (!id.startsWith("task-row-")) return;
    if (child.visible !== true) return;
    const name = typeof child.name === "string" ? child.name : "";
    if (name.includes(taskText)) {
      match = id;
    }
  });
  return match;
}

function findVisibleIdsByPrefix(node, prefix) {
  const matches = [];
  walk(node, (child) => {
    const id = typeof child.id === "string" ? child.id : "";
    if (!id.startsWith(prefix)) return;
    if (child.visible !== true) return;
    matches.push(id);
  });
  return matches;
}

async function closeAllTerminals(app, { addr, timeoutMs }) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const tree = await app.rpc("automation.tree.snapshot").catch(() => null);
    if (!tree) {
      await sleep(200);
      continue;
    }

    const deleteIds = findVisibleIdsByPrefix(tree, "terminal-delete-");
    if (deleteIds.length === 0) {
      return;
    }

    // Click one at a time; IDs should remain stable across snapshots.
    await app.page.locator(`#${deleteIds[0]}`).click();
    await waitIdle(addr, 250);
  }

  throw new Error("Timed out closing terminals");
}

async function waitForTaskRowId(app, { timeoutMs, taskText }) {
  const target = String(taskText ?? "").trim();
  if (!target) {
    throw new Error("Task text must not be empty");
  }
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const tree = await app.rpc("automation.tree.snapshot").catch(() => null);
    if (tree) {
      const match = findTaskRowId(tree, target);
      if (match) {
        return match;
      }
    }
    await sleep(250);
  }
  throw new Error(`Timed out waiting for task row containing "${target}"`);
}

async function waitForThreadRender(app, { timeoutMs }) {
  const start = Date.now();
  let lastStatus = null;
  let lastThreadItemId = "";
  while (Date.now() - start < timeoutMs) {
    const tree = await app.rpc("automation.tree.snapshot").catch(() => null);
    const threadRoot = tree ? findNodeById(tree, "thread-list") ?? tree : null;
    const threadItemId = threadRoot ? findThreadItemId(threadRoot) : "";
    if (threadItemId) {
      lastThreadItemId = threadItemId;
    }

    const status = await app.rpc("automation.thread.status").catch(() => null);
    if (status && typeof status === "object") {
      lastStatus = status;
      const total = Number(status.total ?? 0);
      const historyLoading = status.history_loading === true;
      const resyncing = status.resyncing === true;
      if (total > 0 && !historyLoading && !resyncing && threadItemId) {
        return status;
      }
    }
    if (threadItemId) {
      if (!lastStatus) {
        return { assistant: threadItemId.startsWith("assistant-") ? 1 : 0, total: 1 };
      }
    }
    await sleep(250);
  }
  throw new Error(
    `Timed out waiting for thread render: ${JSON.stringify({
      last_thread_item_id: lastThreadItemId || null,
      last_status: lastStatus,
    })}`,
  );
}

async function waitForSessionsLoaded(app, { timeoutMs }) {
  const start = Date.now();
  let lastStatus = null;
  while (Date.now() - start < timeoutMs) {
    const status = await app.rpc("automation.thread.status").catch(() => null);
    if (status && typeof status === "object") {
      lastStatus = status;
      const sessionsTotal = Number(status.sessions_total ?? 0);
      if (sessionsTotal > 0) {
        return status;
      }
    }
    await sleep(250);
  }
  throw new Error(
    `Timed out waiting for sessions to load: ${JSON.stringify({ last_status: lastStatus })}`,
  );
}

async function waitForSelectedTask(app, { timeoutMs, taskId }) {
  const start = Date.now();
  let lastStatus = null;
  const target = String(taskId ?? "").trim();
  if (!target) {
    throw new Error("taskId must not be empty");
  }
  while (Date.now() - start < timeoutMs) {
    const status = await app.rpc("automation.thread.status").catch(() => null);
    if (status && typeof status === "object") {
      lastStatus = status;
      if (String(status.selected_task_id ?? "").trim() === target) {
        return status;
      }
    }
    await sleep(250);
  }
  throw new Error(
    `Timed out waiting for selected task ${target}: ${JSON.stringify({ last_status: lastStatus })}`,
  );
}

async function waitForSelectedSession(app, { timeoutMs, index }) {
  const start = Date.now();
  let lastStatus = null;
  const targetIndex = Number(index);
  while (Date.now() - start < timeoutMs) {
    const status = await app.rpc("automation.thread.status").catch(() => null);
    if (status && typeof status === "object") {
      lastStatus = status;
      if (Number(status.selected_session_index ?? -1) === targetIndex) {
        return status;
      }
    }
    await sleep(250);
  }
  throw new Error(
    `Timed out waiting for selected session ${targetIndex}: ${JSON.stringify({ last_status: lastStatus })}`,
  );
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
      await waitForTaskRowId(app, {
        timeoutMs: args.readyTimeoutMs,
        taskText: args.taskText,
      });

      // New task
      await focus(args.addr, "new_task");
      await waitIdle(args.addr, args.settleMs);
      await expect(app.page.locator("#composer-input")).toBeVisible({
        timeoutMs: args.readyTimeoutMs,
      });
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
      const daemonToken = await readDaemonAuthToken();
      const resolved =
        args.daemonUrl && daemonToken
          ? await resolveTaskSession({
              daemonUrl: args.daemonUrl,
              token: daemonToken,
              workspaceName: args.workspaceName,
              taskText: args.taskText,
              timeoutMs: args.readyTimeoutMs,
            }).catch(() => null)
          : null;
      if (args.daemonUrl && daemonToken) {
        await waitForSessionContent({
          daemonUrl: args.daemonUrl,
          token: daemonToken,
          workspaceName: args.workspaceName,
          taskText: args.taskText,
          timeoutMs: args.readyTimeoutMs,
          sessionId: resolved?.sessionId ?? "",
        });
      }

      if (resolved?.taskId) {
        log(`select task via daemon ${resolved.taskId}`);
        await app.rpc("ctx.tasks.select", { task_id: resolved.taskId });
        await waitIdle(args.addr, args.settleMs);
        await waitForSelectedTask(app, {
          timeoutMs: args.readyTimeoutMs,
          taskId: resolved.taskId,
        });
      } else {
        log(`select task containing '${args.taskText}'`);
        const taskRowId = await waitForTaskRowId(app, {
          timeoutMs: args.readyTimeoutMs,
          taskText: args.taskText,
        });
        const taskRow = app.page.locator(`#${taskRowId}`);
        await expect(taskRow).toBeVisible({ timeoutMs: args.readyTimeoutMs });
        await taskRow.click();
        await waitIdle(args.addr, args.settleMs);
      }

      log("select session 0");
      await waitForSessionsLoaded(app, { timeoutMs: args.readyTimeoutMs });
      await app.page.clickSession(0);
      await waitIdle(args.addr, args.settleMs);
      await waitForSelectedSession(app, {
        timeoutMs: args.readyTimeoutMs,
        index: 0,
      });

      await expect(app.page.locator("#thread-list")).toBeVisible({
        timeoutMs: args.readyTimeoutMs,
      });
      await waitForThreadRender(app, {
        timeoutMs: args.readyTimeoutMs,
      }).catch(async (err) => {
        log(`warn: thread did not render in time (${err.message})`);
        const debug = await app.rpc("automation.thread.debug").catch(() => null);
        if (debug) {
          log(`thread.debug: ${JSON.stringify(debug)}`);
        }
      });
      await scrollThreadForParity(app, {
        addr: args.addr,
        settleMs: args.settleMs,
        timeoutMs: args.readyTimeoutMs,
      });
      await waitIdle(args.addr, args.settleMs);
      await screenshot(args.addr, "active-session/native.png");

      // Integrated terminal (open panel).
      log("open terminal panel");
      let parityTerminalId = "";
      if (args.daemonUrl && daemonToken && resolved?.workspaceId) {
        const workspaceId = resolved.workspaceId;
        const terminals = await listWorkspaceTerminalsViaDaemon({
          daemonUrl: args.daemonUrl,
          token: daemonToken,
          workspaceId,
          timeoutMs: args.readyTimeoutMs,
        });
        if (terminals.length === 1) {
          parityTerminalId = idToString(terminals[0]?.id);
        } else if (terminals.length > 1) {
          await deleteAllWorkspaceTerminals({
            daemonUrl: args.daemonUrl,
            token: daemonToken,
            workspaceId,
            timeoutMs: args.readyTimeoutMs,
          });
          parityTerminalId = await createWorkspaceTerminalViaDaemon({
            daemonUrl: args.daemonUrl,
            token: daemonToken,
            workspaceId,
            cwd: process.env.CTX_PARITY_REPO_DIR ?? null,
            timeoutMs: args.readyTimeoutMs,
          });
        } else {
          parityTerminalId = await createWorkspaceTerminalViaDaemon({
            daemonUrl: args.daemonUrl,
            token: daemonToken,
            workspaceId,
            cwd: process.env.CTX_PARITY_REPO_DIR ?? null,
            timeoutMs: args.readyTimeoutMs,
          });
        }
      }
      await focus(args.addr, "terminal_panel");
      await waitIdle(args.addr, args.settleMs);
      await expect(app.page.locator("#terminal-panel")).toBeVisible({
        timeoutMs: args.readyTimeoutMs,
      });
      await app.page.locator("#terminal-scope-workspace").click().catch(() => {});
      await app.page.locator("#terminal-refresh").click().catch(() => {});
      await waitIdle(args.addr, args.settleMs);
      if (parityTerminalId) {
        const parityTerminalLocator = app.page.locator(
          `#terminal-select-${parityTerminalId}`,
        );
        const parityTerminalVisible = await expect(parityTerminalLocator)
          .toBeVisible({
            timeoutMs: args.readyTimeoutMs,
          })
          .then(() => true)
          .catch(() => false);
        if (parityTerminalVisible) {
          await parityTerminalLocator.click().catch(() => {});
          await waitIdle(args.addr, args.settleMs);
        } else {
          log(
            `warn: terminal-select-${parityTerminalId} never became visible; capturing terminal anyway`,
          );
        }
      } else {
        await closeAllTerminals(app, { addr: args.addr, timeoutMs: args.readyTimeoutMs });
      }
      await screenshot(args.addr, "terminal/native.png");

      // Right pane (artifacts).
      log("open artifacts pane");
      await focus(args.addr, "artifacts_pane");
      await waitIdle(args.addr, args.settleMs);
      await expect(app.page.locator("#artifacts-pane")).toBeVisible({
        timeoutMs: args.readyTimeoutMs,
      });
      await screenshot(args.addr, "right-pane/artifacts/native.png");
      await screenshot(args.addr, "active-session/right-pane/artifacts/native.png");

      // Right pane (diff).
      log("open diff pane");
      await focus(args.addr, "diff_pane");
      await waitIdle(args.addr, args.settleMs);
      await expect(app.page.locator("#diff-pane")).toBeVisible({
        timeoutMs: args.readyTimeoutMs,
      });
      await screenshot(args.addr, "right-pane/diff/native.png");

      // Settings
      log("open settings");
      await callJson(args.addr, "/route", { body: { route: "settings" } });
      await expect(app.page.locator("#settings-pane")).toBeVisible({
        timeoutMs: args.readyTimeoutMs,
      });
      await waitIdle(args.addr, args.settleMs);

      await screenshot(args.addr, "settings/native.png");

      log("focus settings search");
      await focus(args.addr, "settings_search");
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
