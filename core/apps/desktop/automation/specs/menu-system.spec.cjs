const path = require("path");
const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");

const DEFAULT_WORKSPACE_PATH = path.resolve(__dirname, "../../../../../");
const WORKSPACE_PATH = process.env.CTX_AUTOMATION_WORKSPACE_PATH
  || process.env.GITHUB_WORKSPACE
  || DEFAULT_WORKSPACE_PATH;

const ALL_MENU_COMMAND_IDS = [
  "file.new-workspace",
  "file.new-window",
  "file.open-recent",
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
  "file.new-window",
  "go.settings",
  "go.diagnostics",
  "go.agent-harnesses",
  "help.crash-course",
  "help.keyboard-shortcuts",
  "help.open-logs-folder",
  "help.diagnostics",
  "task.archive-toggle",
  "task.delete",
  "file.new-workspace",
];

const APP_HANDLED_COMMANDS = new Set([
  "file.new-workspace",
  "file.new-window",
  "go.launcher",
  "go.workspace-setup",
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
  const exec = async (cmd, a) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { ok: false, error: "Tauri invoke not available" };
    try {
      const value = await invoke(cmd, a || {});
      return { ok: true, value };
    } catch (err) {
      return { ok: false, error: String(err) };
    }
  };
  const result = typeof args === "undefined"
    ? await browser.execute(exec, command)
    : await browser.execute(exec, command, args);
  if (!result || !result.ok) {
    throw new Error(result?.error || `invoke failed: ${command}`);
  }
  return result.value;
};

const installDialogShims = async () => {
  await browser.execute(() => {
    window.confirm = () => true;
    window.alert = () => {};
  });
};

const getCommandState = async (commandId) => {
  return await invokeDesktop("desktop_get_menu_item_state", { commandId });
};

const triggerCommandAndCollectTraces = async (commandId, timeoutMs = 4000) => {
  const result = await browser.executeAsync((id, timeout, done) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      done({ ok: false, error: "Tauri invoke not available", traces: [] });
      return;
    }
    const traces = [];
    const onTrace = (event) => {
      const detail = event && event.detail ? event.detail : null;
      if (!detail || detail.commandId !== id) return;
      traces.push({
        commandId: String(detail.commandId),
        layer: String(detail.layer || ""),
        status: String(detail.status || ""),
        note: typeof detail.note === "string" ? detail.note : "",
        route: `${window.location.pathname || ""}${window.location.search || ""}${window.location.hash || ""}`,
      });
    };

    let finished = false;
    const finish = (payload) => {
      if (finished) return;
      finished = true;
      window.removeEventListener("ctx:menu-trace", onTrace);
      done(payload);
    };
    window.addEventListener("ctx:menu-trace", onTrace);
    window.confirm = () => true;
    window.alert = () => {};

    const start = Date.now();
    const poll = () => {
      if (traces.length > 0) {
        finish({ ok: true, traces });
        return;
      }
      if (Date.now() - start >= timeout) {
        finish({ ok: true, traces });
        return;
      }
      setTimeout(poll, 25);
    };

    invoke("desktop_trigger_menu_command", { commandId: id })
      .then(() => poll())
      .catch((err) => finish({ ok: false, error: String(err), traces }));
  }, commandId, timeoutMs);
  if (!result || !result.ok) {
    throw new Error(result?.error || `desktop_trigger_menu_command failed: ${commandId}`);
  }
  return Array.isArray(result.traces) ? result.traces : [];
};

const waitForTraceBridgeReady = async () => {
  await browser.waitUntil(
    async () => {
      const traces = await triggerCommandAndCollectTraces("file.open-recent", 1200);
      return traces.some((t) => t.layer === "app");
    },
    { timeout: 30000, timeoutMsg: "menu trace bridge not ready" },
  );
};

const getToggleUiState = async (commandId) => {
  return await browser.execute((id) => {
    switch (id) {
      case "view.toggle-sidebar": {
        const root = document.querySelector(".wb-root");
        return Boolean(root && root.classList.contains("wb-root-collapsed"));
      }
      case "view.toggle-diff":
        return Boolean(document.querySelector(".wb-diff"));
      case "view.toggle-artifacts":
        return Boolean(document.querySelector(".wb-artifacts"));
      case "view.toggle-sessions":
        return Boolean(document.querySelector(".wb-sessions"));
      case "view.toggle-terminal": {
        const shell = document.querySelector(".wb-terminal-shell");
        if (!(shell instanceof HTMLElement)) return null;
        return shell.getAttribute("aria-hidden") !== "true";
      }
      default:
        return null;
    }
  }, commandId);
};

const getCurrentRoute = async () =>
  browser.execute(() => `${window.location.pathname || ""}${window.location.search || ""}${window.location.hash || ""}`);

const ensureWorkspaceRoute = async (workspaceId) => {
  const target = `/workspaces/${workspaceId}`;
  await browser.url(`tauri://localhost${target}?menu_e2e=${Date.now()}`);
  await waitForTauri();
  await installDialogShims();
  await browser.waitUntil(
    async () => {
      const pathName = await browser.execute(() => window.location.pathname || "");
      return pathName === target;
    },
    { timeout: 60000, timeoutMsg: `failed to route to ${target}` },
  );
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

    await browser.url(`tauri://localhost/workspace-setup?menu_e2e=${Date.now()}`);
    await waitForTauri();
    await installDialogShims();

    const workspaceId = await createWorkspaceAndTask(WORKSPACE_PATH);
    await ensureWorkspaceRoute(workspaceId);
    await waitForTraceBridgeReady();

    for (const commandId of MENU_TEST_ORDER) {
      await ensureWorkspaceRoute(workspaceId);
      await waitForTraceBridgeReady();

      const beforeState = await getCommandState(commandId);
      const beforeToggleUiState = TOGGLE_COMMANDS.has(commandId) ? await getToggleUiState(commandId) : null;

      const traces = await triggerCommandAndCollectTraces(commandId);
      if (traces.length === 0) {
        throw new Error(`no menu trace observed for ${commandId}`);
      }
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
        if (beforeState.enabled === true) {
          if (!workbenchTraces.some((t) => t.status === "handled")) {
            throw new Error(`expected handled workbench trace for enabled ${commandId}, got ${JSON.stringify(traces)}`);
          }
        }
      }

      if (TOGGLE_COMMANDS.has(commandId) && beforeState.enabled === true && typeof beforeToggleUiState === "boolean") {
        await browser.waitUntil(
          async () => {
            const nextState = await getToggleUiState(commandId);
            return typeof nextState === "boolean" && nextState !== beforeToggleUiState;
          },
          { timeout: 20000, timeoutMsg: `expected UI state change for ${commandId}` },
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
