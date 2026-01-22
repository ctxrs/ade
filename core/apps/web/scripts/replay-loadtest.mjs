import { chromium } from "playwright";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(__dirname, "..");

const defaultFixture = path.resolve(appRoot, "e2e/fixtures/workbench-replay-fixture.json");
const defaultOut = path.join(os.tmpdir(), "ctx-web-loadtest-telemetry.json");
const defaultBaseUrl = "http://127.0.0.1:5173";

const args = process.argv.slice(2);
let fixturePath = defaultFixture;
let outPath = defaultOut;
let baseUrl = process.env.CTX_LOADTEST_BASE_URL || defaultBaseUrl;

for (let i = 0; i < args.length; i++) {
  const arg = args[i];
  if (arg === "--fixture") {
    fixturePath = args[i + 1] || fixturePath;
    i += 1;
    continue;
  }
  if (arg === "--out") {
    outPath = args[i + 1] || outPath;
    i += 1;
    continue;
  }
  if (arg === "--base-url") {
    baseUrl = args[i + 1] || baseUrl;
    i += 1;
    continue;
  }
  if (arg === "--help" || arg === "-h") {
    console.log(`Usage: node ./scripts/replay-loadtest.mjs [--fixture path] [--out path] [--base-url url]\n`);
    process.exit(0);
  }
}

const fixture = JSON.parse(await readFile(fixturePath, "utf8"));
const workspaceId = fixture?.workspace?.id || fixture?.active_snapshot?.workspace_id;
if (!workspaceId) {
  throw new Error("Fixture missing workspace id.");
}

const streamEvents = Array.isArray(fixture.stream) ? fixture.stream : [];
const snapshotBySession = fixture.session_snapshots || {};

const respondJson = async (route, body, status = 200) => {
  await route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body ?? null),
  });
};

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext();
const page = await context.newPage();

await page.addInitScript((events) => {
  window.__CTX_LOAD_TEST__ = true;
  window.__CTX_LOAD_TEST_EVENTS__ = Array.isArray(events) ? events : [];

  const OriginalWebSocket = window.WebSocket;
  const matchesReplay = (url) => {
    const u = String(url ?? "");
    return u.includes("/api/workspaces/") && u.includes("/active_snapshot/stream");
  };

  class ReplayWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;

    constructor(url, protocols) {
      if (!matchesReplay(url)) {
        return new OriginalWebSocket(url, protocols);
      }
      this.url = String(url ?? "");
      this.readyState = ReplayWebSocket.CONNECTING;
      this.protocol = "";
      this.extensions = "";
      this.binaryType = "blob";
      this.bufferedAmount = 0;
      this.onopen = null;
      this.onmessage = null;
      this.onerror = null;
      this.onclose = null;
      this._listeners = new Map();
      this._timers = [];
      this._closed = false;

      const openTimer = window.setTimeout(() => {
        if (this._closed) return;
        this.readyState = ReplayWebSocket.OPEN;
        this._emit("open");
        this._startReplay();
      }, 0);
      this._timers.push(openTimer);
    }

    addEventListener(type, listener) {
      const list = this._listeners.get(type) || [];
      list.push(listener);
      this._listeners.set(type, list);
    }

    removeEventListener(type, listener) {
      const list = this._listeners.get(type);
      if (!list) return;
      const next = list.filter((item) => item !== listener);
      if (next.length === 0) {
        this._listeners.delete(type);
      } else {
        this._listeners.set(type, next);
      }
    }

    dispatchEvent(event) {
      const list = this._listeners.get(event.type) || [];
      for (const listener of list) {
        if (typeof listener === "function") {
          listener.call(this, event);
        } else if (listener && typeof listener.handleEvent === "function") {
          listener.handleEvent.call(listener, event);
        }
      }
      return true;
    }

    send() {
      // Ignore client messages; replay is one-way.
    }

    close() {
      if (this._closed) return;
      this.readyState = ReplayWebSocket.CLOSED;
      this._closed = true;
      for (const timer of this._timers) {
        window.clearTimeout(timer);
      }
      this._timers = [];
      this._emit("close");
    }

    _emit(type, data) {
      let evt;
      if (type === "message") {
        if (typeof MessageEvent !== "undefined") {
          evt = new MessageEvent("message", { data });
        } else {
          evt = new Event("message");
          evt.data = data;
        }
      } else if (type === "close") {
        if (typeof CloseEvent !== "undefined") {
          evt = new CloseEvent("close", { code: 1000, reason: "replay complete", wasClean: true });
        } else {
          evt = new Event("close");
        }
      } else {
        evt = new Event(type);
      }

      const handler = this[`on${type}`];
      if (typeof handler === "function") {
        handler.call(this, evt);
      }
      this.dispatchEvent(evt);
    }

    _startReplay() {
      const events = window.__CTX_LOAD_TEST_EVENTS__ || [];
      for (const item of events) {
        const delay = Math.max(0, Number(item?.delay_ms ?? 0));
        const timer = window.setTimeout(() => {
          if (this._closed || this.readyState !== ReplayWebSocket.OPEN) return;
          this._emit("message", JSON.stringify(item.event));
        }, delay);
        this._timers.push(timer);
      }
    }
  }

  window.WebSocket = ReplayWebSocket;
}, streamEvents);

await page.route("**/api/**", async (route) => {
  const request = route.request();
  const url = new URL(request.url());
  const method = request.method().toUpperCase();
  const pathname = url.pathname;

  if (method === "OPTIONS") {
    await route.fulfill({ status: 204 });
    return;
  }

  if (pathname === "/api/health") {
    await respondJson(route, fixture.health);
    return;
  }

  if (pathname === "/api/providers") {
    await respondJson(route, fixture.providers ?? []);
    return;
  }

  if (pathname === `/api/workspaces/${workspaceId}`) {
    await respondJson(route, fixture.workspace);
    return;
  }

  if (pathname === `/api/workspaces/${workspaceId}/active_snapshot`) {
    await respondJson(route, fixture.active_snapshot);
    return;
  }

  if (pathname === `/api/workspaces/${workspaceId}/archived_task_summaries`) {
    await respondJson(route, {
      workspace_id: workspaceId,
      archived_rev: fixture.active_snapshot?.archived_rev ?? 0,
      tasks: [],
      next_cursor: null,
      total_archived: 0,
    });
    return;
  }

  const sessionMatch = pathname.match(/^\/api\/sessions\/([^/]+)(?:\/(.*))?$/);
  if (sessionMatch) {
    const sessionId = sessionMatch[1];
    const tail = sessionMatch[2] || "";

    if (tail.startsWith("snapshot")) {
      const snapshot = snapshotBySession[sessionId];
      await respondJson(route, snapshot ?? { summary: {}, head: { session: {}, turns: [], messages: [], last_event_seq: 0 } });
      return;
    }

    if (tail.startsWith("state")) {
      const snapshot = snapshotBySession[sessionId];
      await respondJson(route, snapshot?.state ?? { artifacts: [], git_status: null });
      return;
    }

    if (tail.startsWith("artifacts")) {
      await respondJson(route, []);
      return;
    }

    if (tail.startsWith("subagent_invocations")) {
      await respondJson(route, []);
      return;
    }

    if (tail.startsWith("turns/") && tail.endsWith("/tools")) {
      await respondJson(route, []);
      return;
    }

    if (tail.startsWith("diff_summary")) {
      await respondJson(route, { summary: "", added: 0, removed: 0 });
      return;
    }

    if (tail.startsWith("diff")) {
      await respondJson(route, { diff: "" });
      return;
    }

    if (tail.startsWith("git_status")) {
      await respondJson(route, { summary_line: "", branch: null, ahead: 0, behind: 0, detached: false, staged: 0, unstaged: 0, untracked: 0, entries: [] });
      return;
    }
  }

  if (pathname === "/api/telemetry/client") {
    await route.fulfill({ status: 204 });
    return;
  }

  await route.fulfill({ status: 404, contentType: "application/json", body: JSON.stringify({ error: "fixture miss" }) });
});

try {
  await page.goto(`${baseUrl}/workspaces/${workspaceId}?loadtest=1`, { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => document.querySelectorAll(".wb-task-row").length >= 2, null, { timeout: 15000 });
  await page.evaluate(() => window.__ctxLoadTestTelemetry?.reset?.());

  const rows = page.locator(".wb-task-row");
  await rows.nth(0).click();
  await page.waitForTimeout(300);
  await rows.nth(1).click();
  await page.waitForTimeout(400);
  await rows.nth(0).click();

  await page.waitForFunction(
    () => (window.__ctxLoadTestTelemetry?.getSnapshot?.().session_switches?.length ?? 0) >= 2,
    null,
    { timeout: 15000 },
  );

  const telemetry = await page.evaluate(() => window.__ctxLoadTestTelemetry?.getSnapshot?.());
  const output = {
    fixture: fixturePath,
    base_url: baseUrl,
    workspace_id: workspaceId,
    captured_at: new Date().toISOString(),
    telemetry,
  };
  await mkdir(path.dirname(outPath), { recursive: true });
  await writeFile(outPath, JSON.stringify(output, null, 2));
  console.log(`wrote telemetry to ${outPath}`);
} finally {
  await browser.close();
}
