const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const {
  waitForTauri,
  selectorForTestId,
  waitForTestId,
  clickTestId,
  setInputTestId,
  getConnectionInfo,
} = require("./helpers/tauri.cjs");
const { daemonJson, safeDaemonJson } = require("./helpers/daemon.cjs");

const REMOTE_HOST_RAW = process.env.CTX_AUTOMATION_REMOTE_HOST || "";
const REMOTE_HOST = REMOTE_HOST_RAW.trim();
const REMOTE_USER_DEFAULT = (process.env.CTX_AUTOMATION_REMOTE_USER || "devboxadmin").trim() || "devboxadmin";
const REMOTE_PASSWORD = process.env.CTX_AUTOMATION_REMOTE_PASSWORD || "";
const REMOTE_WIZARD_HOST_INPUT = (() => {
  const raw = REMOTE_HOST_RAW.trim();
  if (!raw) return "";
  if (raw.includes("@")) return raw;
  if (REMOTE_HOST) return `${REMOTE_USER_DEFAULT}@${REMOTE_HOST}`;
  return raw;
})();
const SCENARIO_FILTER = new Set(
  String(process.env.CTX_AUTOMATION_SCENARIOS || "")
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean),
);
const scenarioEnabled = (name, tags = []) => {
  if (SCENARIO_FILTER.size === 0) return true;
  const keys = [name, ...tags].map((s) => String(s).trim().toLowerCase()).filter(Boolean);
  return keys.some((k) => SCENARIO_FILTER.has(k));
};
const parsePort = (raw, fallback) => {
  const n = Number(raw);
  if (!Number.isFinite(n)) return fallback;
  const p = Math.trunc(n);
  if (p < 1 || p > 65535) return fallback;
  return p;
};
const REMOTE_PORT = parsePort(process.env.CTX_AUTOMATION_REMOTE_PORT || "44099", 44099);
const REMOTE_DATA_DIR_RAW = process.env.CTX_AUTOMATION_REMOTE_DATA_DIR || "";
const SSH_NO_START_REMOTE = !["0", "false", "no"].includes(
  String(process.env.CTX_AUTOMATION_SSH_NO_START_REMOTE || "1").trim().toLowerCase(),
);

const run = (cmd, args, opts = {}) => {
  const res = spawnSync(cmd, args, { encoding: "utf8", ...opts });
  if (res.status !== 0) {
    const stderr = String(res.stderr || "").trim();
    const stdout = String(res.stdout || "").trim();
    const msg = [
      `command failed: ${cmd} ${args.join(" ")}`,
      stderr ? `stderr: ${stderr}` : null,
      stdout ? `stdout: ${stdout}` : null,
    ]
      .filter(Boolean)
      .join("\n");
    throw new Error(msg);
  }
  return String(res.stdout || "");
};

const mkTempDir = (prefix) => fs.mkdtempSync(path.join(os.tmpdir(), prefix));

const initGitRepo = (dir, name) => {
  fs.mkdirSync(dir, { recursive: true });
  run("git", ["init", "--", dir]);
  run("git", ["-C", dir, "config", "user.email", "ctx-e2e@example.com"]);
  run("git", ["-C", dir, "config", "user.name", "ctx-e2e"]);
  fs.writeFileSync(path.join(dir, "README.md"), `# ${name}\n`, "utf8");
  run("git", ["-C", dir, "add", "README.md"]);
  run("git", ["-C", dir, "commit", "-m", "init"]);
};

const waitForSelector = async (selector, timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => await browser.execute((s) => Boolean(document.querySelector(s)), selector),
    { timeout: timeoutMs, timeoutMsg: `element not found: ${selector}` },
  );
};

const clickSelector = async (selector) => {
  await waitForSelector(selector);
  const ok = await browser.execute((s) => {
    const el = document.querySelector(s);
    if (!el) return false;
    (el).click();
    return true;
  }, selector);
  if (!ok) throw new Error(`failed to click selector: ${selector}`);
};

const setTextareaSelector = async (selector, value) => {
  await waitForSelector(selector);
  const ok = await browser.execute((s, v) => {
    const el = document.querySelector(s);
    if (!el) return false;
    if (!(el instanceof HTMLTextAreaElement)) return false;
    const desc = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value");
    const setter = desc && desc.set;
    if (!setter) return false;
    setter.call(el, v);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  }, selector, String(value));
  if (!ok) throw new Error(`failed to set textarea for: ${selector}`);
};

const setCheckedTestId = async (id, checked) => {
  await waitForTestId(id);
  const sel = selectorForTestId(id);
  const ok = await browser.execute((s, want) => {
    const el = document.querySelector(s);
    if (!el) return false;
    if (!(el instanceof HTMLInputElement) || el.type !== "checkbox") return false;
    if (Boolean(el.checked) !== Boolean(want)) el.click();
    return true;
  }, sel, Boolean(checked));
  if (!ok) throw new Error(`failed to set checkbox for: ${sel}`);
};

const clickBack = async () => {
  const ok = await browser.execute(() => {
    const btn = document.querySelector('[data-testid="wizard-back"]')
      || document.querySelector('[data-testid="wizard-back-link"]');
    if (!btn) return false;
    (btn).click();
    return true;
  });
  return Boolean(ok);
};

const currentStepKey = async () => {
  return await browser.execute(() => {
    const el = document.querySelector('[data-testid="workspace-setup"]');
    return el ? el.getAttribute("data-step-key") : null;
  });
};

const waitForStep = async (key) => {
  await browser.waitUntil(
    async () => (await currentStepKey()) === key,
    { timeout: 30000, timeoutMsg: `expected step '${key}'` },
  );
};

const clickOption = async (stepKey, optionId) => {
  const id = `wizard-option-${stepKey}-${optionId}`;
  try {
    await clickTestId(id);
  } catch (error) {
    if (stepKey !== "source") throw error;
    const state = await browser.execute(() => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      const optionTestIds = Array.from(document.querySelectorAll('[data-testid^="wizard-option-"]'))
        .map((el) => String(el.getAttribute("data-testid") || ""))
        .filter(Boolean);
      return {
        step,
        optionTestIds,
        hasSourcePath: Boolean(document.querySelector('[data-testid="wizard-source-path"]')),
        hasRepoUrl: Boolean(document.querySelector('[data-testid="wizard-repo-url"]')),
        hasWorkspaceName: Boolean(document.querySelector('[data-testid="wizard-workspace-name"]')),
      };
    });
    const sourceAlreadySelected =
      state.step === "source"
      && (
        (optionId === "import" && state.hasSourcePath)
        || (optionId === "clone" && state.hasRepoUrl)
        || (optionId === "new" && state.hasWorkspaceName)
      );
    if (sourceAlreadySelected) return;
    const errText = error instanceof Error ? error.message : String(error);
    throw new Error(
      `${errText}; source-step diag=${JSON.stringify(state)}`,
    );
  }
};

const ensureContainerOptionVisible = async (optionId, timeoutMs = 15000) => {
  const optionSelector = `[data-testid="wizard-option-container-${optionId}"]`;
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const state = await browser.execute((selector) => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      if (step !== "container") {
        return { visible: false, toggled: false, step };
      }
      if (document.querySelector(selector)) {
        return { visible: true, toggled: false, step };
      }
      const toggle = document.querySelector('[data-testid="wizard-container-advanced-toggle"]');
      if (toggle) {
        toggle.click();
        return { visible: false, toggled: true, step };
      }
      return { visible: false, toggled: false, step };
    }, optionSelector);
    if (state.step !== "container") return false;
    if (state.visible) return true;
    await browser.pause(state.toggled ? 150 : 100);
  }
  return false;
};

const clickNext = async () => {
  await clickTestId("wizard-next");
};

const clickNextIfEnabled = async () => {
  return await browser.execute(() => {
    const el = document.querySelector('[data-testid="wizard-next"]');
    if (!(el instanceof HTMLButtonElement)) {
      return { present: false, disabled: null, clicked: false };
    }
    if (el.disabled) {
      return { present: true, disabled: true, clicked: false };
    }
    el.click();
    return { present: true, disabled: false, clicked: true };
  });
};

const clickCreate = async () => {
  await waitForTestId("wizard-create");
  const state = await browser.execute(() => {
    const el = document.querySelector('[data-testid="wizard-create"]');
    if (!(el instanceof HTMLButtonElement)) return { present: false, disabled: null, text: "" };
    return {
      present: true,
      disabled: Boolean(el.disabled),
      text: String(el.textContent || "").trim(),
    };
  });
  if (!state?.present) throw new Error("wizard-create button missing at confirm step");
  if (state?.disabled) {
    throw new Error(`wizard-create button disabled at confirm step (label='${state.text || ""}')`);
  }
  await clickTestId("wizard-create");
};

const setInput = async (testId, value) => {
  await setInputTestId(testId, value);
};

const setChecked = async (testId, checked) => {
  await setCheckedTestId(testId, checked);
};


const compactEntity = (value) => {
  if (!value || typeof value !== "object") return value;
  const keys = [
    "id",
    "task_id",
    "session_id",
    "workspace_id",
    "status",
    "state",
    "title",
    "provider_id",
    "provider",
    "event_type",
    "type",
    "kind",
    "name",
    "role",
    "message_type",
    "status_reason",
    "created_at",
    "updated_at",
    "last_error",
    "error",
    "message",
    "content",
    "text",
  ];
  const out = {};
  for (const k of keys) {
    if (Object.prototype.hasOwnProperty.call(value, k)) out[k] = value[k];
  }
  const payloadKeys = ["payload", "event_json", "event", "data", "details", "delta"];
  for (const k of payloadKeys) {
    if (!Object.prototype.hasOwnProperty.call(value, k)) continue;
    try {
      const raw = value[k];
      const str = typeof raw === "string" ? raw : JSON.stringify(raw);
      out[k] = str.length > 800 ? `${str.slice(0, 800)}...` : str;
    } catch {
      out[k] = "[unserializable]";
    }
  }
  return Object.keys(out).length > 0 ? out : value;
};

const idString = (value) => {
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  if (!value || typeof value !== "object") return "";
  if (typeof value.id === "string") return value.id;
  if (typeof value.value === "string") return value.value;
  if (typeof value.id === "number") return String(value.id);
  if (typeof value.value === "number") return String(value.value);
  return "";
};

const collectCodexSmokeDiagnostics = async (workspaceId) => {
  const diag = { workspaceId };
  const tasksResp = await safeDaemonJson("GET", `/api/workspaces/${workspaceId}/tasks`);
  diag.workspaceTasksStatus = tasksResp.status ?? null;
  if (tasksResp.error) diag.workspaceTasksError = tasksResp.error;
  const tasks = Array.isArray(tasksResp.payload) ? tasksResp.payload : [];
  diag.workspaceTasks = tasks.slice(-3).map((t) => compactEntity(t));

  const latestTask = tasks[tasks.length - 1];
  const taskId = idString(latestTask?.id || latestTask?.task_id || latestTask?.taskId);
  if (!taskId) return diag;
  diag.taskId = taskId;

  const sessionsResp = await safeDaemonJson("GET", `/api/tasks/${taskId}/sessions`);
  diag.taskSessionsStatus = sessionsResp.status ?? null;
  if (sessionsResp.error) diag.taskSessionsError = sessionsResp.error;
  const sessions = Array.isArray(sessionsResp.payload) ? sessionsResp.payload : [];
  diag.taskSessions = sessions.slice(-3).map((s) => compactEntity(s));

  const latestSession = sessions[sessions.length - 1];
  const sessionId = idString(latestSession?.id || latestSession?.session_id || latestSession?.sessionId);
  if (!sessionId) return diag;
  diag.sessionId = sessionId;

  const sessionStateResp = await safeDaemonJson("GET", `/api/sessions/${sessionId}/state`);
  diag.sessionStateStatus = sessionStateResp.status ?? null;
  if (sessionStateResp.error) diag.sessionStateError = sessionStateResp.error;
  diag.sessionState = compactEntity(sessionStateResp.payload);

  const sessionEventsResp = await safeDaemonJson("GET", `/api/sessions/${sessionId}/events?limit=5`);
  diag.sessionEventsStatus = sessionEventsResp.status ?? null;
  if (sessionEventsResp.error) diag.sessionEventsError = sessionEventsResp.error;
  const eventsPayload = sessionEventsResp.payload;
  const events = Array.isArray(eventsPayload?.events)
    ? eventsPayload.events
    : Array.isArray(eventsPayload)
      ? eventsPayload
      : [];
  diag.sessionEvents = events.slice(-5).map((evt) => compactEntity(evt));

  const sessionHistoryResp = await safeDaemonJson("GET", `/api/sessions/${sessionId}/history?limit=8`);
  diag.sessionHistoryStatus = sessionHistoryResp.status ?? null;
  if (sessionHistoryResp.error) diag.sessionHistoryError = sessionHistoryResp.error;
  const historyPayload = sessionHistoryResp.payload;
  const history = Array.isArray(historyPayload?.items)
    ? historyPayload.items
    : Array.isArray(historyPayload?.history)
      ? historyPayload.history
      : Array.isArray(historyPayload)
        ? historyPayload
        : [];
  diag.sessionHistory = history.slice(-8).map((item) => compactEntity(item));
  return diag;
};

const getWorkspace = async (id) => {
  const resp = await daemonJson("GET", `/api/workspaces/${id}`);
  if (resp.status !== 200) throw new Error(`GET /api/workspaces/${id} failed (${resp.status})`);
  return resp.payload;
};

const getWorkspaceHarnessContainer = async (workspaceId) => {
  const resp = await daemonJson("GET", `/api/workspaces/${workspaceId}/harness_container`);
  if (resp.status !== 200) {
    throw new Error(`GET /api/workspaces/${workspaceId}/harness_container failed (${resp.status})`);
  }
  return resp.payload;
};

const createWorkspaceTerminal = async (workspaceId, body = {}) => {
  const resp = await daemonJson("POST", `/api/workspaces/${workspaceId}/terminals`, body);
  if (resp.status !== 200) {
    throw new Error(`POST /api/workspaces/${workspaceId}/terminals failed (${resp.status})`);
  }
  return resp.payload;
};

const deleteTerminal = async (terminalId) => {
  const resp = await daemonJson("DELETE", `/api/terminals/${terminalId}`);
  if (resp.status !== 204 && resp.status !== 404) {
    throw new Error(`DELETE /api/terminals/${terminalId} failed (${resp.status})`);
  }
};

const getWorkspaceTerminalCwd = async (workspaceId) => {
  const terminal = await createWorkspaceTerminal(workspaceId, {});
  const terminalId = typeof terminal.id === "string"
    ? terminal.id
    : String(terminal.id?.id || terminal.id?.value || terminal.id?.["0"] || "");
  if (!terminalId) {
    throw new Error(`terminal id missing: ${JSON.stringify(terminal)}`);
  }
  try {
    return String(terminal.cwd || "");
  } finally {
    await deleteTerminal(terminalId);
  }
};

const assertWorkspaceTerminalCwdPrefix = async (workspaceId, expectedPrefix) => {
  const cwd = await getWorkspaceTerminalCwd(workspaceId);
  if (!cwd.startsWith(expectedPrefix)) {
    throw new Error(`expected terminal cwd to start with '${expectedPrefix}', got '${cwd}'`);
  }
};

const daemonOverlayText = async () => {
  return await browser.execute(() => {
    const el = document.querySelector(".daemon-overlay");
    if (!el) return "";
    const text = String(el.textContent || "").trim();
    return text || "daemon overlay visible";
  });
};

const assertNoDaemonOverlayFor = async (durationMs = 2000) => {
  const started = Date.now();
  while (Date.now() - started < durationMs) {
    const text = await daemonOverlayText();
    if (text) throw new Error(`daemon unavailable overlay rendered: ${text}`);
    await browser.pause(100);
  }
};

const waitForRemoteStepAfterLocation = async (timeoutMs = 60000) => {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const state = await browser.execute(() => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      const errEl = document.querySelector(".wizard-error");
      const err = errEl ? String(errEl.textContent || "").trim() : "";
      return { step, err };
    });
    const step = String(state?.step || "");
    const err = String(state?.err || "");
    if (err) throw new Error(`remote verification failed: ${err}`);
    if (step && step !== "location") return step;
    await browser.pause(100);
  }
  throw new Error("remote verification did not advance from location step within timeout");
};

const clickAuthImportSkip = async () => {
  const ok = await browser.execute(() => {
    const step = document.querySelector('[data-testid="wizard-step"][data-step-key="auth-import"]');
    if (!step) return false;
    const buttons = Array.from(step.querySelectorAll("button"));
    const skip = buttons.find((btn) => String(btn.textContent || "").trim() === "Skip for now");
    if (!skip) return false;
    skip.click();
    return true;
  });
  if (!ok) throw new Error("failed to skip auth-import step");
};

const ensureReadyForSourceSelection = async ({ location, container }, timeoutMs = 60000) => {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const key = await currentStepKey();
    if (key === "source") return;
    if (key === "auth-import") {
      await clickAuthImportSkip();
      await browser.pause(100);
      continue;
    }
    if (key === "container") {
      if (!container) throw new Error("container step reached but scenario.container is missing");
      if (container === "host-mounted") {
        const hostMountedVisible = await ensureContainerOptionVisible("host-mounted");
        if (!hostMountedVisible) {
          await browser.pause(100);
          continue;
        }
      }
      await clickOption("container", container);
      await browser.pause(100);
      const afterSelect = await currentStepKey();
      if (afterSelect === "container") {
        const next = await clickNextIfEnabled();
        if (next.clicked) {
          await browser.pause(100);
        }
      }
      continue;
    }
    if (key === "location") {
      if (!location) throw new Error("location step reached but scenario.location is missing");
      await clickOption("location", location);
      await browser.pause(100);
      continue;
    }
    if (key === "network" || key === "setup" || key === "merge-queue" || key === "confirm") {
      throw new Error(`source step skipped unexpectedly; current step is '${key}'`);
    }
    await browser.pause(100);
  }
  const finalStep = await currentStepKey();
  throw new Error(`expected step 'source', got '${finalStep || "unknown"}'`);
};

const selectSourceOptionWithRetry = async (
  { location, container, sourceKind },
  attempts = 6,
) => {
  const sourceSelectionReady = async () => {
    const state = await browser.execute(() => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      return {
        step,
        hasSourcePath: Boolean(document.querySelector('[data-testid="wizard-source-path"]')),
        hasRepoUrl: Boolean(document.querySelector('[data-testid="wizard-repo-url"]')),
        hasWorkspaceName: Boolean(document.querySelector('[data-testid="wizard-workspace-name"]')),
      };
    });
    if (state.step !== "source") return false;
    if (sourceKind === "import") return state.hasSourcePath;
    if (sourceKind === "clone") return state.hasRepoUrl;
    if (sourceKind === "new") return state.hasWorkspaceName;
    return false;
  };

  let lastError = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    await ensureReadyForSourceSelection({ location, container });
    try {
      await clickOption("source", sourceKind);
      if (await sourceSelectionReady()) return;
      lastError = new Error(`source option '${sourceKind}' click did not settle UI`);
    } catch (error) {
      lastError = error;
    }
    if (attempt === attempts - 1) break;
    await browser.pause(150);
  }
  if (lastError instanceof Error) throw lastError;
  throw new Error("failed to select source option");
};

const collectWorkspaceRouteDiagnostics = async () => {
  const ui = await browser.execute(() => {
    const pathname = window.location.pathname;
    const root = document.querySelector('[data-testid="workspace-setup"]');
    const step = root ? root.getAttribute("data-step-key") : null;
    const errEl = document.querySelector(".wizard-error");
    const err = errEl ? String(errEl.textContent || "").trim() : "";
    const overlayEl = document.querySelector(".daemon-overlay");
    const overlay = overlayEl ? String(overlayEl.textContent || "").trim() : "";
    const creatingEl = document.querySelector('[data-testid="wizard-creating-status"]');
    const creating = creatingEl ? String(creatingEl.textContent || "").trim() : "";
    const createBtn = document.querySelector('[data-testid="wizard-create"]');
    const createDisabled = createBtn instanceof HTMLButtonElement ? Boolean(createBtn.disabled) : null;
    const createLabel = createBtn ? String(createBtn.textContent || "").trim() : "";
    const summaryRows = Array.from(document.querySelectorAll(".wizard-summary-row"))
      .map((row) => {
        const k = row.querySelector(".wizard-summary-k");
        const v = row.querySelector(".wizard-summary-v");
        return {
          k: k ? String(k.textContent || "").trim() : "",
          v: v ? String(v.textContent || "").trim() : "",
        };
      })
      .filter((r) => r.k || r.v);
    return { pathname, step, err, overlay, creating, createDisabled, createLabel, summaryRows };
  });
  let conn;
  try {
    conn = await getConnectionInfo();
  } catch (error) {
    conn = { error: String(error) };
  }
  const wsResp = await safeDaemonJson("GET", "/api/workspaces");
  const workspaces = Array.isArray(wsResp.payload) ? wsResp.payload : [];
  return {
    ui,
    connection: conn,
    workspaceStatus: wsResp.status ?? null,
    workspaceError: wsResp.error || null,
    workspaces: workspaces.slice(-5).map((w) => compactEntity(w)),
  };
};

const waitForWorkspaceRoute = async (timeoutMs = 120000) => {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const state = await browser.execute(() => {
      const pathname = window.location.pathname;
      const errEl = document.querySelector(".wizard-error");
      const err = errEl ? String(errEl.textContent || "").trim() : "";
      const overlayEl = document.querySelector(".daemon-overlay");
      const overlay = overlayEl ? String(overlayEl.textContent || "").trim() : "";
      return { pathname, err, overlay };
    });
    if (state?.err) {
      throw new Error(`wizard create error: ${state.err}`);
    }
    if (state?.overlay) {
      throw new Error(`daemon unavailable overlay rendered: ${state.overlay}`);
    }
    const p = String(state?.pathname || "");
    if (p.startsWith("/workspaces/")) {
      const id = p.split("/")[2] || "";
      if (!id) throw new Error("workspace id missing from route");
      return id;
    }
    await browser.pause(200);
  }
  const diag = await collectWorkspaceRouteDiagnostics();
  throw new Error(`did not navigate to /workspaces/:id; diag=${JSON.stringify(diag)}`);
};

const assertLocalWorkspaceConfig = async (workspaceId, expectations) => {
  const resp = await daemonJson("GET", `/api/workspaces/${workspaceId}/execution_config`);
  if (resp.status !== 200) {
    throw new Error(`GET /api/workspaces/${workspaceId}/execution_config failed (${resp.status})`);
  }
  const cfg = resp.payload || {};

  if (expectations.environment && cfg.environment !== expectations.environment) {
    throw new Error(`expected execution.environment=${expectations.environment}, got ${cfg.environment || "<missing>"}`);
  }

  if (expectations.networkMode) {
    const got = cfg.network_mode || "";
    if (got !== expectations.networkMode) {
      throw new Error(`expected execution.network_mode=${expectations.networkMode}, got ${got || "<missing>"}`);
    }
  }

  if (expectations.allowlist) {
    const got = Array.isArray(cfg.allowlist) ? cfg.allowlist : [];
    for (const entry of expectations.allowlist) {
      if (!got.includes(entry)) {
        throw new Error(`expected execution.allowlist to include '${entry}', got ${JSON.stringify(got)}`);
      }
    }
  }
};

// Quote a string for safe embedding in a single-quoted POSIX shell string.
const shSingleQuote = (s) => `'${String(s).replace(/'/g, `'\"'\"'`)}'`;

const sshArgs = (target, cmd) => {
  const args = [
    "-o", "StrictHostKeyChecking=accept-new",
    "-o", "ConnectTimeout=15",
  ];
  if (!REMOTE_PASSWORD) {
    args.push("-o", "BatchMode=yes");
  }
  args.push(target, `bash -lc ${shSingleQuote(cmd)}`);
  return args;
};

const ssh = (target, cmd) => {
  const args = sshArgs(target, cmd);
  if (REMOTE_PASSWORD) {
    return run("sshpass", ["-e", "ssh", ...args], {
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, SSHPASS: REMOTE_PASSWORD },
    });
  }
  return run("ssh", args, { stdio: ["ignore", "pipe", "pipe"] });
};

const ensureRemoteTarget = () => {
  if (!REMOTE_HOST) return null;
  if (REMOTE_HOST.includes("@")) return REMOTE_HOST;
  return `${REMOTE_USER_DEFAULT}@${REMOTE_HOST}`;
};

const assertConnectedLocalAndListening = async () => {
  const info = await getConnectionInfo();
  if (!info || info.kind !== "local") {
    throw new Error(`expected local daemon connection, got: ${JSON.stringify(info)}`);
  }
  if (!info.base_url || !info.token) {
    throw new Error(`expected base_url+token in connection info, got: ${JSON.stringify(info)}`);
  }
  // Sanity: confirm something is actually listening on the connected port.
  const u = new URL(info.base_url);
  const port = Number(u.port || "");
  if (!port) {
    throw new Error(`expected base_url to include a port, got: ${info.base_url}`);
  }
  const ls = spawnSync("lsof", ["-nP", `-iTCP:${port}`, "-sTCP:LISTEN"], { encoding: "utf8" });
  if (ls.status !== 0) {
    throw new Error(`expected daemon to be listening on port ${port}`);
  }
};

const runWizardScenario = async (scenario) => {
  // Use a unique query param to force a real navigation (avoid SPA state re-use).
  await browser.url(`tauri://localhost/workspace-setup?e2e=${Date.now()}`);
  await waitForTauri();
  // Hard-reset wizard UI state between scenarios.
  // The wizard persists progress in webview storage; without clearing, the app can reopen mid-step.
  await browser.execute(() => {
    try { localStorage.clear(); } catch { /* ignore */ }
    try { sessionStorage.clear(); } catch { /* ignore */ }
  });
  await waitForTestId("workspace-setup", 60000);

  // If we land mid-wizard (e.g. due to single-instance or a prior run), walk back to the start.
  for (let i = 0; i < 12; i += 1) {
    const key = await currentStepKey();
    if (key === "location") break;
    const clicked = await clickBack();
    if (!clicked) break;
    await browser.pause(50);
  }

  await waitForStep("location");
  await clickOption("location", scenario.location);

  if (scenario.location === "remote") {
    await setInput("wizard-remote-host", scenario.remoteHost);
    if (
      typeof scenario.remotePort === "number"
      || typeof scenario.remoteDataDir === "string"
    ) {
      const hasAdvanced = await browser.execute(
        () => Boolean(document.querySelector('[data-testid="wizard-remote-port"]')),
      );
      if (!hasAdvanced) {
        await clickTestId("wizard-remote-advanced-toggle");
      }
      if (typeof scenario.remotePort === "number") {
        await setInput("wizard-remote-port", String(scenario.remotePort));
      }
      if (typeof scenario.remoteDataDir === "string" && scenario.remoteDataDir.trim()) {
        await setInput("wizard-remote-data-dir", scenario.remoteDataDir.trim());
      }
    }
    await clickNext(); // verifies SSH and advances
  }

  if (scenario.location === "remote") {
    const afterLocation = await waitForRemoteStepAfterLocation();
    if (afterLocation === "source" && scenario.container && scenario.container !== "no-container") {
      throw new Error("remote wizard did not expose container step (container modes unavailable)");
    }
    if (afterLocation !== "source" && afterLocation !== "container") {
      const wizardErr = await browser.execute(() => {
        const el = document.querySelector(".wizard-error");
        return el ? String(el.textContent || "").trim() : "";
      });
      throw new Error(
        `expected step 'container' or 'source', got '${afterLocation}' (wizard-error: ${wizardErr || "none"})`,
      );
    }
  }

  await selectSourceOptionWithRetry({
    location: scenario.location,
    container: scenario.container,
    sourceKind: scenario.source.kind,
  });

  const sourcePathVisible = await browser.execute(
    () => Boolean(document.querySelector('[data-testid="wizard-source-path"]')),
  );
  const expectsManagedStagingSource = (
    scenario.container === "disk-isolated"
    && (scenario.source.kind === "clone" || scenario.source.kind === "new")
  );

  if (scenario.source.kind === "import") {
    if (!sourcePathVisible) {
      throw new Error("wizard-source-path missing for import source");
    }
    await setInput("wizard-source-path", scenario.source.path);
  } else if (scenario.source.kind === "clone") {
    if (sourcePathVisible) {
      await setInput("wizard-source-path", scenario.source.destPath);
    } else if (!expectsManagedStagingSource) {
      throw new Error("wizard-source-path missing for clone source");
    }
    await setInput("wizard-repo-url", scenario.source.repoUrl);
    if (scenario.source.branch) {
      await setInput("wizard-repo-branch", scenario.source.branch);
    }
  } else if (scenario.source.kind === "new") {
    if (scenario.source.destPath && sourcePathVisible) {
      await setInput("wizard-source-path", scenario.source.destPath);
    } else if (scenario.source.destPath && !expectsManagedStagingSource) {
      throw new Error("wizard-source-path missing for new source");
    }
    if (scenario.source.workspaceName) {
      await setInput("wizard-workspace-name", scenario.source.workspaceName);
    }
  }
  await clickNext();

  const afterSource = await currentStepKey();
  if (afterSource === "network") {
    await clickOption("network", scenario.network);
    if (scenario.network === "allowlist") {
      await setInput("wizard-network-allowlist", scenario.networkAllowlist || "github.com");
      await clickNext();
    }
  }

  await waitForStep("setup");
  if (scenario.setupHook) {
    await setInput("wizard-setup-hook", scenario.setupHook);
  }
  await clickNext();

  await waitForStep("merge-queue");
  if (scenario.mergeQueue.kind === "skip") {
    await clickTestId("wizard-merge-skip");
  } else {
    await setInput("wizard-merge-target-branch", scenario.mergeQueue.targetBranch || "main");
    if (scenario.mergeQueue.verifyCommand) {
      await setInput("wizard-merge-verify-command", scenario.mergeQueue.verifyCommand);
    }
    if (scenario.mergeQueue.pushOnSuccess) {
      await clickTestId("wizard-merge-advanced-toggle");
      await setChecked("wizard-merge-push-on-success", true);
      if (scenario.mergeQueue.pushRemote) {
        await setInput("wizard-merge-push-remote", scenario.mergeQueue.pushRemote);
      }
      if (scenario.mergeQueue.pushBranch) {
        await setInput("wizard-merge-push-branch", scenario.mergeQueue.pushBranch);
      }
    }
    await clickNext();
  }

  await waitForStep("confirm");
  await clickCreate();

  const id = await waitForWorkspaceRoute(scenario.location === "remote" ? 180000 : 120000);
  // The workbench must never render the "daemon unavailable" overlay on first navigation.
  // If connect_local returns before the daemon is reachable, this can flash briefly.
  await assertNoDaemonOverlayFor(2000);
  return id;
};

describe("launcher workspace wizard (e2e)", () => {
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-e2e-${runId}-`);
  const localImportRepo = path.join(localBase, "import-repo");
  const localCloneSrc = path.join(localBase, "clone-src");

  const remoteTarget = ensureRemoteTarget();
  const remoteHostForWizard = REMOTE_WIZARD_HOST_INPUT || REMOTE_HOST || (REMOTE_HOST_RAW.includes("@") ? REMOTE_HOST_RAW.split("@").pop() : REMOTE_HOST_RAW);
  const remoteBase = `/tmp/ctx-e2e-${runId}`;
  const remoteDataDir = REMOTE_DATA_DIR_RAW.trim() || `${remoteBase}/daemon`;
  const preserveRemoteDaemonDir = SSH_NO_START_REMOTE;
  let remoteBaseResolved = remoteBase;
  let remoteHasPodman = false;
  let remoteSupportsContainerStep = false;

  before(async () => {
    initGitRepo(localImportRepo, "local-import");
    initGitRepo(localCloneSrc, "local-clone-src");

    if (remoteTarget) {
      const podmanProbe = ssh(
        remoteTarget,
        "if command -v podman >/dev/null 2>&1; then echo yes; else echo no; fi",
      ).trim();
      remoteHasPodman = podmanProbe === "yes";
      // Pre-create remote repos for import/clone without touching the daemon.
      const script = [
        "set -euo pipefail",
        `base=${JSON.stringify(remoteBase)}`,
        `daemon_dir=${JSON.stringify(remoteDataDir)}`,
        ...(preserveRemoteDaemonDir
          ? []
          : ["if [[ -n \"$daemon_dir\" && \"$daemon_dir\" == /tmp/* ]]; then rm -rf \"$daemon_dir\"; fi"]),
        "rm -rf \"$base\"",
        "mkdir -p \"$base\"",
        "mkdir -p \"$base/import-repo\"",
        "git init \"$base/import-repo\"",
        "git -C \"$base/import-repo\" config user.email ctx-e2e@example.com",
        "git -C \"$base/import-repo\" config user.name ctx-e2e",
        "echo hi >\"$base/import-repo/README.md\"",
        "git -C \"$base/import-repo\" add README.md",
        "git -C \"$base/import-repo\" commit -m init",
        "mkdir -p \"$base/clone-src\"",
        "git init \"$base/clone-src\"",
        "git -C \"$base/clone-src\" config user.email ctx-e2e@example.com",
        "git -C \"$base/clone-src\" config user.name ctx-e2e",
        "echo hi >\"$base/clone-src/README.md\"",
        "git -C \"$base/clone-src\" add README.md",
        "git -C \"$base/clone-src\" commit -m init",
      ].join("\n");
      ssh(remoteTarget, script);
      try {
        const resolved = ssh(
          remoteTarget,
          `base=${JSON.stringify(remoteBase)}; mkdir -p "$base"; realpath "$base"`,
        ).trim();
        if (resolved) remoteBaseResolved = resolved;
      } catch {
        // Keep the nominal /tmp path if realpath is unavailable.
      }

      // Probe whether the current wizard flow exposes a container step for remote targets.
      await browser.url(`tauri://localhost/workspace-setup?remoteProbe=${Date.now()}`);
      await waitForTauri();
      await waitForTestId("workspace-setup", 60000);
      await waitForStep("location");
      await clickOption("location", "remote");
      await setInput("wizard-remote-host", remoteHostForWizard);
      await clickNext();
      let step = await currentStepKey();
      if (step === "location") {
        try {
          step = await waitForRemoteStepAfterLocation(20000);
        } catch {
          step = await currentStepKey();
        }
      }
      remoteSupportsContainerStep = step === "container";
    }
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore
    }
    if (remoteTarget) {
      try {
        const cleanup = [
          "set -euo pipefail",
          `base=${JSON.stringify(remoteBase)}`,
          `daemon_dir=${JSON.stringify(remoteDataDir)}`,
          "rm -rf \"$base\"",
          ...(preserveRemoteDaemonDir
            ? []
            : ["if [[ -n \"$daemon_dir\" && \"$daemon_dir\" == /tmp/* ]]; then rm -rf \"$daemon_dir\"; fi"]),
        ].join("\n");
        ssh(remoteTarget, cleanup);
      } catch {
        // ignore
      }
    }
  });

  it("local import works end-to-end", async function () {
    if (!scenarioEnabled("local-import", ["local", "host"])) this.skip();
    const id = await runWizardScenario({
      location: "local",
      container: "no-container",
      source: { kind: "import", path: localImportRepo },
      setupHook: "pnpm install",
      mergeQueue: { kind: "skip" },
    });

    await assertConnectedLocalAndListening();
    const ws = await getWorkspace(id);
    await assertLocalWorkspaceConfig(id, { environment: "host", mergeQueueEnabled: false, setupHook: "pnpm install" });
    await assertWorkspaceTerminalCwdPrefix(id, ws.root_path);
  });

  it("local clone works end-to-end (merge queue enabled)", async function () {
    if (!scenarioEnabled("local-clone-disk-isolated", ["local", "container", "disk-isolated"])) this.skip();
    const destParent = path.join(localBase, "clone-dest");
    fs.mkdirSync(destParent, { recursive: true });
    const destPath = `${destParent}/`; // trailing slash -> derive repo name

    const id = await runWizardScenario({
      location: "local",
      container: "disk-isolated",
      network: "providers",
      source: { kind: "clone", repoUrl: localCloneSrc, branch: "", destPath },
      setupHook: "pnpm install",
      mergeQueue: { kind: "enabled", targetBranch: "main", verifyCommand: "pnpm -v" },
    });

    await assertConnectedLocalAndListening();
    const ws = await getWorkspace(id);
    await assertLocalWorkspaceConfig(id, {
      environment: "container_disk_isolated",
      networkMode: "llm_only",
      mergeQueueEnabled: true,
      targetBranch: "main",
      setupHook: "pnpm install",
    });
    const container = await getWorkspaceHarnessContainer(id);
    if (!container || !container.running || container.mount_mode !== "disk_isolated") {
      throw new Error(`expected running disk-isolated container, got: ${JSON.stringify(container)}`);
    }
    const cloneCwd = await getWorkspaceTerminalCwd(id);
    if (!cloneCwd.startsWith("/ctx/ws")) {
      throw new Error(`expected disk-isolated terminal cwd under /ctx/ws, got: ${cloneCwd}`);
    }
  });

  it("local new empty works end-to-end", async function () {
    if (!scenarioEnabled("local-new-host-mounted", ["local", "container", "host-mounted"])) this.skip();
    const dest = path.join(localBase, "new-host-mounted");
    const id = await runWizardScenario({
      location: "local",
      container: "host-mounted",
      network: "allowlist",
      networkAllowlist: "github.com\nregistry.npmjs.org",
      source: { kind: "new", destPath: dest, workspaceName: "host-mounted-ws" },
      setupHook: "",
      mergeQueue: { kind: "enabled", targetBranch: "main", verifyCommand: "" },
    });

    await assertConnectedLocalAndListening();
    const ws = await getWorkspace(id);
    await assertLocalWorkspaceConfig(id, {
      environment: "container_host_mounted",
      networkMode: "allowlist",
      allowlist: ["github.com", "registry.npmjs.org"],
      mergeQueueEnabled: true,
      targetBranch: "main",
    });
    const container = await getWorkspaceHarnessContainer(id);
    if (!container || !container.running || container.mount_mode !== "host_mounted") {
      throw new Error(`expected running host-mounted container, got: ${JSON.stringify(container)}`);
    }
    await assertWorkspaceTerminalCwdPrefix(id, ws.root_path);
  });

  it("local disk-isolated container works end-to-end", async function () {
    if (!scenarioEnabled("local-new-disk-isolated", ["local", "container", "disk-isolated"])) this.skip();
    const dest = path.join(localBase, "new-disk-isolated");
    const id = await runWizardScenario({
      location: "local",
      container: "disk-isolated",
      network: "full",
      source: { kind: "new", destPath: dest, workspaceName: "disk-isolated-ws" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });

    await assertConnectedLocalAndListening();
    const ws = await getWorkspace(id);
    await assertLocalWorkspaceConfig(id, {
      environment: "container_disk_isolated",
      networkMode: "all",
      mergeQueueEnabled: false,
    });
    const container = await getWorkspaceHarnessContainer(id);
    if (!container || !container.running || container.mount_mode !== "disk_isolated") {
      throw new Error(`expected running disk-isolated container, got: ${JSON.stringify(container)}`);
    }
    await assertWorkspaceTerminalCwdPrefix(id, "/ctx/ws");
  });

  it("local container can start Codex and respond", async function () {
    if (!scenarioEnabled("local-codex-smoke", ["local", "container", "provider"])) this.skip();
    // Container start + provider spin-up can take a while on a fresh machine (Podman VM, image load, etc).
    this.timeout(420000);

    const dest = path.join(localBase, "codex-host-mounted");
    const id = await runWizardScenario({
      location: "local",
      container: "host-mounted",
      network: "providers",
      source: { kind: "new", destPath: dest, workspaceName: "codex-smoke" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });

    await assertConnectedLocalAndListening();
    // Wait for the workbench to render the new-task composer, then send a simple prompt.
    await waitForSelector("textarea.wb-new-composer-textarea", 60000);
    await setTextareaSelector("textarea.wb-new-composer-textarea", "hello");
    await clickSelector("button.wb-send");

    // Wait for any assistant response, but fail fast if the UI reports session start failure.
    let lastState = "{}";
    try {
      await browser.waitUntil(
        async () => {
          const state = await browser.execute(() => {
            const assistantEls = Array.from(document.querySelectorAll(".wb-assistant-entry, .msg.assistant"));
            const assistantText = assistantEls
              .map((el) => String(el.textContent || "").trim())
              .filter(Boolean)
              .join("\n");
            const wbBanner = Array.from(document.querySelectorAll(".wb-banner"))
              .map((el) => String(el.textContent || "").trim())
              .filter(Boolean)
              .join(" | ");
            const failHeader = Array.from(document.querySelectorAll(".wb-session-slot .banner strong"))
              .map((el) => String(el.textContent || "").trim())
              .find((t) => /failed to start/i.test(t)) || "";
            const failDetail = Array.from(document.querySelectorAll(".wb-session-slot .banner .error"))
              .map((el) => String(el.textContent || "").trim())
              .find(Boolean) || "";
            return {
              assistantText,
              wbBanner,
              failHeader,
              failDetail,
            };
          });
          const diag = {
            assistantText: String(state?.assistantText || ""),
            wbBanner: String(state?.wbBanner || ""),
            failHeader: String(state?.failHeader || ""),
            failDetail: String(state?.failDetail || ""),
          };
          lastState = JSON.stringify(diag);
          if (diag.failHeader || diag.failDetail || /failed to start/i.test(diag.wbBanner)) {
            throw new Error(`Codex session failed to start: ${lastState}`);
          }
          return diag.assistantText.length > 0;
        },
        {
          timeout: 240000,
          interval: 250,
          timeoutMsg: `no assistant response received in time; last_state=${lastState}`,
        },
      );
    } catch (err) {
      const apiDiag = await collectCodexSmokeDiagnostics(id);
      throw new Error(
        `codex smoke did not complete: ${String(err)}; ui=${lastState}; api=${JSON.stringify(apiDiag)}`,
      );
    }

    // Sanity: ensure we stayed in the same workspace route.
    const ws = await getWorkspace(id);
    await assertLocalWorkspaceConfig(id, { environment: "container_host_mounted" });
  });

  it("remote import works end-to-end", async function () {
    if (!scenarioEnabled("remote-import-host", ["remote", "remote-host"])) this.skip();
    if (!remoteTarget) this.skip();
    const importPath = `${remoteBase}/import-repo`;

    const id = await runWizardScenario({
      location: "remote",
      remoteHost: remoteHostForWizard,
      remotePort: REMOTE_PORT,
      remoteDataDir,
      container: "no-container",
      source: { kind: "import", path: importPath },
      setupHook: "pnpm install",
      mergeQueue: { kind: "skip" },
    });

    const ws = await getWorkspace(id);
    const rootPath = String(ws.root_path || "");
    const expectedPrefixes = Array.from(new Set([remoteBase, remoteBaseResolved]));
    if (!expectedPrefixes.some((prefix) => rootPath.startsWith(prefix))) {
      throw new Error(`expected remote workspace root under one of [${expectedPrefixes.join(", ")}], got ${rootPath}`);
    }
  });

  it("remote clone works end-to-end", async function () {
    if (!scenarioEnabled("remote-clone-host-mounted", ["remote", "remote-container", "host-mounted"])) this.skip();
    if (!remoteTarget) this.skip();
    if (!remoteHasPodman) this.skip();
    if (!remoteSupportsContainerStep) this.skip();
    const destParent = `${remoteBase}/clone-dest`;
    const destPath = `${destParent}/`;
    const src = `${remoteBase}/clone-src`;
    ssh(remoteTarget, `mkdir -p ${JSON.stringify(destParent)}`);

    const id = await runWizardScenario({
      location: "remote",
      remoteHost: remoteHostForWizard,
      remotePort: REMOTE_PORT,
      remoteDataDir,
      container: "host-mounted",
      network: "allowlist",
      networkAllowlist: "github.com",
      source: { kind: "clone", repoUrl: src, branch: "", destPath },
      setupHook: "pnpm install",
      mergeQueue: { kind: "enabled", targetBranch: "main", verifyCommand: "echo ok" },
    });

    const ws = await getWorkspace(id);
    const rootPath = String(ws.root_path || "");
    const expectedPrefixes = Array.from(new Set([remoteBase, remoteBaseResolved]));
    if (!expectedPrefixes.some((prefix) => rootPath.startsWith(prefix))) {
      throw new Error(`expected remote workspace root under one of [${expectedPrefixes.join(", ")}], got ${rootPath}`);
    }
  });

  it("remote new empty works end-to-end", async function () {
    if (!scenarioEnabled("remote-new-disk-isolated", ["remote", "remote-container", "disk-isolated"])) this.skip();
    if (!remoteTarget) this.skip();
    if (!remoteHasPodman) this.skip();
    if (!remoteSupportsContainerStep) this.skip();
    const dest = `${remoteBase}/new-disk-isolated`;
    ssh(remoteTarget, `rm -rf ${JSON.stringify(dest)}`);

    const id = await runWizardScenario({
      location: "remote",
      remoteHost: remoteHostForWizard,
      remotePort: REMOTE_PORT,
      remoteDataDir,
      container: "disk-isolated",
      network: "full",
      source: { kind: "new", destPath: dest, workspaceName: "disk-isolated-remote" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });

    const ws = await getWorkspace(id);
    const rootPath = String(ws.root_path || "");
    const expectedPrefixes = Array.from(new Set([remoteBase, remoteBaseResolved]));
    if (!expectedPrefixes.some((prefix) => rootPath.startsWith(prefix))) {
      throw new Error(`expected remote workspace root under one of [${expectedPrefixes.join(", ")}], got ${rootPath}`);
    }
  });
});
