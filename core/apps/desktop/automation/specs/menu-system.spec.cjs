const path = require("path");
const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");

const DEFAULT_WORKSPACE_PATH = path.resolve(__dirname, "../../../../../");
const WORKSPACE_PATH = process.env.CTX_AUTOMATION_WORKSPACE_PATH
  || process.env.GITHUB_WORKSPACE
  || DEFAULT_WORKSPACE_PATH;

const ALL_MENU_COMMAND_IDS = [
  "file.new-workspace",
  "file.open-workspaces",
  "file.open-recent",
  "file.open-workspace-new-window",
  "file.export-transcript",
  "file.export-session-log",
  "view.find-tasks",
  "view.toggle-sidebar",
  "view.toggle-diff",
  "view.toggle-artifacts",
  "view.toggle-sessions",
  "view.toggle-terminal",
  "task.new",
  "task.rename",
  "task.archive-toggle",
  "task.mark-read-toggle",
  "task.delete",
  "session.copy-transcript",
  "session.copy-session-log",
  "session.copy-worktree-location",
  "session.open-worktree-terminal",
  "session.interrupt",
  "go.launcher",
  "go.workspace-setup",
  "go.workspaces",
  "go.settings",
  "go.diagnostics",
  "go.agent-harnesses",
  "help.crash-course",
  "help.keyboard-shortcuts",
  "help.open-logs-folder",
  "help.report-issue",
  "help.diagnostics",
];

// Keep the app in a stable workbench context while testing most commands.
const MENU_TEST_ORDER = [
  "file.export-transcript",
  "file.export-session-log",
  "file.open-recent",
  "view.find-tasks",
  "view.toggle-sidebar",
  "view.toggle-diff",
  "view.toggle-artifacts",
  "view.toggle-sessions",
  "view.toggle-terminal",
  "task.new",
  "task.rename",
  "task.mark-read-toggle",
  "session.copy-transcript",
  "session.copy-session-log",
  "session.copy-worktree-location",
  "session.open-worktree-terminal",
  "session.interrupt",
  "help.report-issue",
  "go.launcher",
  "go.workspace-setup",
  "file.open-workspaces",
  "go.workspaces",
  "go.settings",
  "go.diagnostics",
  "go.agent-harnesses",
  "help.crash-course",
  "help.keyboard-shortcuts",
  "help.open-logs-folder",
  "help.diagnostics",
  "task.archive-toggle",
  "task.delete",
  "file.open-workspace-new-window",
  "file.new-workspace",
];

const APP_HANDLED_COMMANDS = new Set([
  "file.new-workspace",
  "file.open-workspaces",
  "file.open-workspace-new-window",
  "go.launcher",
  "go.workspace-setup",
  "go.workspaces",
  "go.settings",
  "go.diagnostics",
  "go.agent-harnesses",
  "help.crash-course",
  "help.keyboard-shortcuts",
  "help.open-logs-folder",
  "help.report-issue",
  "help.diagnostics",
]);

const TOGGLE_COMMANDS = new Set([
  "view.toggle-sidebar",
  "view.toggle-diff",
  "view.toggle-artifacts",
  "view.toggle-sessions",
  "view.toggle-terminal",
]);

const NAV_EXPECTATIONS = new Map([
  ["go.launcher", (_workspaceId) => "/"],
  ["go.workspace-setup", (_workspaceId) => "/workspace-setup"],
  ["file.open-workspaces", (_workspaceId) => "/workspaces"],
  ["go.workspaces", (_workspaceId) => "/workspaces"],
  ["go.settings", (workspaceId) => `/settings?ws=${encodeURIComponent(workspaceId)}`],
  ["go.diagnostics", (_workspaceId) => "/diagnostics"],
  ["help.diagnostics", (_workspaceId) => "/diagnostics"],
  ["go.agent-harnesses", (_workspaceId) => "/settings#agent_harnesses"],
  ["help.crash-course", (_workspaceId) => "/crash-course"],
  ["help.keyboard-shortcuts", (_workspaceId) => "/crash-course"],
]);

const assertSetEqual = (a, b, label) => {
  const left = new Set(a);
  const right = new Set(b);
  const missing = [...left].filter((v) => !right.has(v));
  const extra = [...right].filter((v) => !left.has(v));
  if (missing.length || extra.length) {
    throw new Error(`${label} mismatch: missing=${JSON.stringify(missing)} extra=${JSON.stringify(extra)}`);
  }
};

const invokeDesktop = async (command, args) => {
  const result = await browser.execute(async (cmd, a) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { ok: false, error: "Tauri invoke not available" };
    try {
      const value = await invoke(cmd, a || {});
      return { ok: true, value };
    } catch (err) {
      return { ok: false, error: String(err) };
    }
  }, command, args || null);
  if (!result || !result.ok) {
    throw new Error(result?.error || `invoke failed: ${command}`);
  }
  return result.value;
};

const installMenuHarness = async () => {
  await browser.execute(() => {
    if (window.__ctxMenuHarnessInstalled) return;
    window.__ctxMenuHarnessInstalled = true;
    window.__ctxMenuHarness = {
      traces: [],
      stateById: {},
      stateVersion: 0,
    };

    window.confirm = () => true;
    window.alert = () => {};

    window.addEventListener("ctx:menu-trace", (event) => {
      const detail = event && event.detail ? event.detail : null;
      if (!detail || typeof detail.commandId !== "string") return;
      const entry = {
        commandId: String(detail.commandId),
        layer: String(detail.layer || ""),
        status: String(detail.status || ""),
        note: typeof detail.note === "string" ? detail.note : "",
        route: `${window.location.pathname || ""}${window.location.search || ""}${window.location.hash || ""}`,
        at: Date.now(),
      };
      window.__ctxMenuHarness.traces.push(entry);
      if (window.__ctxMenuHarness.traces.length > 400) {
        window.__ctxMenuHarness.traces = window.__ctxMenuHarness.traces.slice(-400);
      }
    });

    window.addEventListener("ctx:menu-state", (event) => {
      const detail = event && event.detail ? event.detail : null;
      if (!detail || !Array.isArray(detail.items)) return;
      if (detail.replace !== false) {
        window.__ctxMenuHarness.stateById = {};
      }
      for (const item of detail.items) {
        if (!item || typeof item.id !== "string") continue;
        window.__ctxMenuHarness.stateById[item.id] = {
          id: String(item.id),
          enabled: typeof item.enabled === "boolean" ? item.enabled : null,
          checked: typeof item.checked === "boolean" ? item.checked : null,
        };
      }
      window.__ctxMenuHarness.stateVersion += 1;
    });
  });
};

const getHarnessSnapshot = async () => {
  return browser.execute(() => {
    const h = window.__ctxMenuHarness;
    if (!h) return null;
    return {
      traceCount: Array.isArray(h.traces) ? h.traces.length : 0,
      stateVersion: Number(h.stateVersion || 0),
      stateById: h.stateById || {},
    };
  });
};

const getCommandTracesSince = async (startIndex, commandId) => {
  return browser.execute((start, id) => {
    const h = window.__ctxMenuHarness;
    if (!h || !Array.isArray(h.traces)) return [];
    return h.traces.slice(start).filter((entry) => entry && entry.commandId === id);
  }, startIndex, commandId);
};

const getCommandState = async (commandId) => {
  return browser.execute((id) => {
    const h = window.__ctxMenuHarness;
    if (!h || !h.stateById) return null;
    return h.stateById[id] || null;
  }, commandId);
};

const getCurrentRoute = async () =>
  browser.execute(() => `${window.location.pathname || ""}${window.location.search || ""}${window.location.hash || ""}`);

const ensureWorkspaceRoute = async (workspaceId) => {
  const target = `/workspaces/${workspaceId}`;
  await browser.execute((nextPath) => {
    const current = window.location.pathname || "";
    if (current === nextPath) return;
    window.location.href = nextPath;
  }, target);
  await browser.waitUntil(
    async () => {
      const pathName = await browser.execute(() => window.location.pathname || "");
      return pathName === target;
    },
    { timeout: 60000, timeoutMsg: `failed to route to ${target}` },
  );
};

const waitForStateVersionIncrement = async (startVersion) => {
  await browser.waitUntil(
    async () => {
      const snap = await getHarnessSnapshot();
      return Boolean(snap && snap.stateVersion > startVersion);
    },
    { timeout: 60000, timeoutMsg: "menu state did not refresh in time" },
  );
};

const waitForCommandTrace = async (startTraceCount, commandId) => {
  await browser.waitUntil(
    async () => {
      const traces = await getCommandTracesSince(startTraceCount, commandId);
      return traces.length > 0;
    },
    { timeout: 20000, timeoutMsg: `no menu trace observed for ${commandId}` },
  );
  return getCommandTracesSince(startTraceCount, commandId);
};

const createWorkspaceAndTask = async (rootPath) => {
  await invokeDesktop("desktop_connect_local");
  const workspaceResp = await daemonJson("POST", "/api/workspaces", { root_path: rootPath });
  if (workspaceResp.status !== 200 && workspaceResp.status !== 201) {
    throw new Error(`workspace creation failed: ${JSON.stringify(workspaceResp)}`);
  }
  const workspaceId = workspaceResp.payload?.id || null;
  if (!workspaceId) {
    throw new Error("No workspace id returned from daemon.");
  }

  const taskResp = await daemonJson("POST", `/api/workspaces/${workspaceId}/tasks`, {
    title: "Automation menu task",
    create_default_session: true,
  });
  if (taskResp.status !== 200 && taskResp.status !== 201) {
    throw new Error(`task creation failed: ${JSON.stringify(taskResp)}`);
  }

  return workspaceId;
};

describe("desktop menu automation", () => {
  it("executes every menu command through the Tauri bridge", async () => {
    assertSetEqual(ALL_MENU_COMMAND_IDS, MENU_TEST_ORDER, "menu command coverage");

    await browser.url("tauri://localhost");
    await waitForTauri();
    await installMenuHarness();

    const workspaceId = await createWorkspaceAndTask(WORKSPACE_PATH);
    await ensureWorkspaceRoute(workspaceId);

    const initialSnap = await getHarnessSnapshot();
    const initialVersion = initialSnap ? initialSnap.stateVersion : 0;
    await waitForStateVersionIncrement(initialVersion);

    // Wait for the workspace to hydrate enough that workbench-scoped actions are available.
    await browser.waitUntil(
      async () => {
        const diffState = await getCommandState("view.toggle-diff");
        return Boolean(diffState && diffState.enabled === true);
      },
      { timeout: 90000, timeoutMsg: "workbench menu state did not hydrate (view.toggle-diff disabled)" },
    );

    for (const commandId of MENU_TEST_ORDER) {
      await ensureWorkspaceRoute(workspaceId);

      const before = await getHarnessSnapshot();
      if (!before) {
        throw new Error("menu harness not available");
      }
      const beforeState = before.stateById[commandId] || null;
      const beforeChecked = beforeState && typeof beforeState.checked === "boolean" ? beforeState.checked : null;

      await invokeDesktop("desktop_trigger_menu_command", { command_id: commandId });
      const traces = await waitForCommandTrace(before.traceCount, commandId);
      const appTraces = traces.filter((t) => t.layer === "app");
      const workbenchTraces = traces.filter((t) => t.layer === "workbench");

      if (APP_HANDLED_COMMANDS.has(commandId)) {
        if (!appTraces.some((t) => t.status === "handled")) {
          throw new Error(`expected app handled trace for ${commandId}, got ${JSON.stringify(traces)}`);
        }
      } else if (!appTraces.some((t) => t.status === "forwarded")) {
        throw new Error(`expected app forwarded trace for ${commandId}, got ${JSON.stringify(traces)}`);
      }

      if (!APP_HANDLED_COMMANDS.has(commandId)) {
        if (workbenchTraces.length === 0) {
          throw new Error(`expected workbench trace for ${commandId}, got ${JSON.stringify(traces)}`);
        }
        if (beforeState && beforeState.enabled === true) {
          if (!workbenchTraces.some((t) => t.status === "handled")) {
            throw new Error(`expected handled workbench trace for enabled ${commandId}, got ${JSON.stringify(traces)}`);
          }
        }
      }

      if (TOGGLE_COMMANDS.has(commandId) && beforeState && beforeState.enabled === true && beforeChecked !== null) {
        await browser.waitUntil(
          async () => {
            const state = await getCommandState(commandId);
            if (!state || typeof state.checked !== "boolean") return false;
            return state.checked !== beforeChecked;
          },
          { timeout: 20000, timeoutMsg: `expected checked state change for ${commandId}` },
        );
      }

      const routeExpectation = NAV_EXPECTATIONS.get(commandId);
      if (routeExpectation) {
        const expectedRoute = routeExpectation(workspaceId);
        await browser.waitUntil(
          async () => (await getCurrentRoute()) === expectedRoute,
          { timeout: 30000, timeoutMsg: `expected route ${expectedRoute} after ${commandId}` },
        );
      }
    }
  });
});
