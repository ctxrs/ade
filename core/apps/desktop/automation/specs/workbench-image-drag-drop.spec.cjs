const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const { waitForTauri, getConnectionInfo } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");

const runChecked = (cmd, args) => {
  const res = spawnSync(cmd, args, { encoding: "utf8" });
  if (res.status === 0) return;
  throw new Error(
    [
      `command failed: ${cmd} ${args.join(" ")}`,
      String(res.stderr || "").trim(),
      String(res.stdout || "").trim(),
    ].filter(Boolean).join("\n"),
  );
};

const initTempRepo = () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-drop-"));
  runChecked("git", ["init", "--", root]);
  runChecked("git", ["-C", root, "config", "user.email", "ctx-e2e@example.com"]);
  runChecked("git", ["-C", root, "config", "user.name", "ctx-e2e"]);
  fs.writeFileSync(path.join(root, "README.md"), "# desktop drop\n", "utf8");
  runChecked("git", ["-C", root, "add", "README.md"]);
  runChecked("git", ["-C", root, "commit", "-m", "init"]);
  return root;
};

const writeTinyPng = () => {
  const filePath = path.join(os.tmpdir(), `ctx-desktop-drop-${Date.now()}.png`);
  const base64 =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+lmZYAAAAASUVORK5CYII=";
  fs.writeFileSync(filePath, Buffer.from(base64, "base64"));
  return filePath;
};

const createWorkspace = async (rootPath) => {
  const connect = await browser.execute(async () => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const info = await invoke("desktop_connect_local");
      return { info };
    } catch (err) {
      return { error: String(err) };
    }
  });
  if (connect && typeof connect === "object" && connect.error) {
    throw new Error(connect.error);
  }
  const resp = await daemonJson("POST", "/api/workspaces", { root_path: rootPath });
  if (resp.status !== 200 && resp.status !== 201) {
    throw new Error(`workspace creation failed: ${JSON.stringify(resp)}`);
  }
  const workspaceId = String(resp.payload?.id || "").trim();
  if (!workspaceId) throw new Error("workspace creation returned no id");
  return workspaceId;
};

const resolveSessionProvider = async (workspaceId) => {
  const bootstrapResp = await daemonJson("GET", `/api/workspaces/${workspaceId}/providers/bootstrap`);
  if (bootstrapResp.status !== 200) {
    throw new Error(`providers bootstrap read failed: ${JSON.stringify(bootstrapResp)}`);
  }
  const providerOptions =
    bootstrapResp.payload && typeof bootstrapResp.payload.provider_options === "object" && bootstrapResp.payload.provider_options
      ? bootstrapResp.payload.provider_options
      : {};
  const preferredProviderIds = ["codex", "claude-crp"];
  const candidateProviderIds = [
    ...preferredProviderIds,
    ...Object.keys(providerOptions),
  ];
  const seen = new Set();
  for (const providerIdRaw of candidateProviderIds) {
    const providerId = String(providerIdRaw || "").trim();
    if (!providerId || seen.has(providerId)) continue;
    seen.add(providerId);
    const entry = providerOptions[providerId];
    const models = Array.isArray(entry?.models?.models) ? entry.models.models : [];
    const currentModelId = String(entry?.models?.current_model_id || "").trim();
    const firstModelId = models
      .map((model) => String(model?.id || "").trim())
      .find(Boolean) || "";
    const modelId = currentModelId || firstModelId;
    if (!modelId) continue;
    return { providerId, modelId };
  }
  throw new Error(
    `no session-capable provider models available: ${JSON.stringify(bootstrapResp.payload?.provider_options || null)}`,
  );
};

const createTaskWithSession = async (workspaceId, title) => {
  const resp = await daemonJson("POST", `/api/workspaces/${workspaceId}/tasks`, {
    title,
    description: "desktop drag drop fake task",
    create_default_session: false,
  });
  if (resp.status !== 200 && resp.status !== 201) {
    throw new Error(`fake task creation failed: ${JSON.stringify(resp)}`);
  }
  const taskId = String(resp.payload?.id || "").trim();
  if (!taskId) throw new Error("fake task creation returned no id");

  const executionConfigResp = await daemonJson("GET", `/api/workspaces/${workspaceId}/execution_config`);
  if (executionConfigResp.status !== 200) {
    throw new Error(`execution config read failed: ${JSON.stringify(executionConfigResp)}`);
  }

  const executionEnvironment = String(executionConfigResp.payload?.environment || "").trim();
  if (
    executionEnvironment !== "host"
    && executionEnvironment !== "sandbox"
  ) {
    throw new Error(`unsupported execution environment: ${executionEnvironment || "<missing>"}`);
  }

  const { providerId, modelId } = await resolveSessionProvider(workspaceId);
  const sessionResp = await daemonJson("POST", `/api/tasks/${taskId}/sessions`, {
    provider_id: providerId,
    model_id: modelId,
    execution_environment: executionEnvironment,
  });
  if (sessionResp.status !== 200 && sessionResp.status !== 201) {
    throw new Error(`session creation failed (${providerId}/${modelId}): ${JSON.stringify(sessionResp)}`);
  }
  const sessionId = String(sessionResp.payload?.id || "").trim();
  if (!sessionId) throw new Error("fake session creation returned no id");

  return { taskId, sessionId };
};

const waitForSelector = async (selector, timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => await browser.execute((s) => Boolean(document.querySelector(s)), selector),
    { timeout: timeoutMs, timeoutMsg: `element not found: ${selector}` },
  );
};

const waitForDropScopeReady = async (selector, timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => await browser.execute((s) => {
      const target = document.querySelector(s);
      if (!(target instanceof HTMLElement)) return false;
      const scope = target.closest(".ctx-drop-scope");
      return Boolean(scope && scope.__ctxDropScopeReady === true);
    }, selector),
    { timeout: timeoutMs, timeoutMsg: `drop scope not ready: ${selector}` },
  );
};

const waitForCtxE2EFocusTask = async (timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => await browser.execute(() => typeof window.__ctxE2E?.focusTask === "function"),
    { timeout: timeoutMs, timeoutMsg: "ctxE2E focusTask bridge not available" },
  );
};

const readTaskSessionDiagnostics = async (taskId, sessionId, taskTitle) =>
  await browser.execute((expectedTaskId, expectedSessionId, expectedTaskTitle) => {
    const text = (value) => String(value || "").replace(/\s+/g, " ").trim();
    const snapshot = window.__ctxE2E?.getWorkspaceSnapshot?.() ?? null;
    const item = snapshot?.tasksById?.[expectedTaskId] ?? null;
    const sessionIds = Array.isArray(item?.sessions)
      ? item.sessions
        .map((entry) => String(entry?.session?.id || entry?.id || ""))
        .filter(Boolean)
      : [];
    const taskRows = Array.from(document.querySelectorAll(".wb-task-row"))
      .map((row) => ({
        title: text(row.querySelector(".wb-task-title")?.textContent),
        active: row.classList.contains("wb-task-row-active"),
      }))
      .filter((row) => row.title);
    const diagnostics = window.__ctxE2E?.getDiagnostics?.() ?? [];
    return {
      location: window.location.href,
      snapshot: snapshot
        ? {
          initialized: Boolean(snapshot.initialized),
          connection: snapshot.connection ?? null,
          fetchState: snapshot.fetchState ?? null,
          activeIds: Array.isArray(snapshot.activeIds) ? snapshot.activeIds.slice(0, 20) : [],
          totalActive: Number(snapshot.totalActive || 0),
          taskPresent: Boolean(item),
          taskTitle: text(item?.task?.title),
          primarySessionId: String(item?.primarySessionId || ""),
          sessionIds,
          expectedSessionPresent:
            sessionIds.includes(String(expectedSessionId))
            || String(item?.primarySessionId || "") === String(expectedSessionId),
        }
        : null,
      ui: {
        expectedTaskTitle: text(expectedTaskTitle),
        hasVisibleSessionSlot: Boolean(document.querySelector(".wb-session-slot[aria-hidden=\"false\"]")),
        hasHydratingSlot: Boolean(document.querySelector(".wb-session-slot--hydrating")),
        hasActiveTextarea: Boolean(document.querySelector(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")),
        loadIssues: Array.from(document.querySelectorAll(".wb-session-load-issues"))
          .map((node) => text(node.textContent))
          .filter(Boolean),
      },
      taskRows: taskRows.slice(0, 20),
      matchingTaskRows: text(expectedTaskTitle)
        ? taskRows.filter((row) => row.title === text(expectedTaskTitle))
        : [],
      diagnostics: Array.isArray(diagnostics)
        ? diagnostics.slice(-12).map((entry) => ({
          code: String(entry?.code || ""),
          source: String(entry?.source || ""),
          message: text(entry?.message),
          context: entry?.context ?? null,
        }))
        : [],
    };
  }, taskId, sessionId, taskTitle);

const waitForWorkspaceTaskSession = async (taskId, sessionId, taskTitle, timeoutMs = 60000) => {
  try {
    await browser.waitUntil(
      async () =>
        await browser.execute((expectedTaskId, expectedSessionId) => {
          const snapshot = window.__ctxE2E?.getWorkspaceSnapshot?.() ?? null;
          if (!snapshot?.initialized) return false;
          const item = snapshot.tasksById?.[expectedTaskId];
          if (!item) return false;
          const sessionIds = Array.isArray(item.sessions)
            ? item.sessions
              .map((entry) => String(entry?.session?.id || entry?.id || ""))
              .filter(Boolean)
            : [];
          return (
            sessionIds.includes(String(expectedSessionId))
            || String(item.primarySessionId || "") === String(expectedSessionId)
          );
        }, taskId, sessionId),
      { timeout: timeoutMs, timeoutMsg: `task ${taskId} / session ${sessionId} missing from workspace snapshot` },
    );
  } catch (error) {
    const diagnostics = await readTaskSessionDiagnostics(taskId, sessionId, taskTitle);
    throw new Error(`${String(error)}\nDiagnostics: ${JSON.stringify(diagnostics)}`);
  }
};

const waitForActiveSessionTextarea = async (taskId, sessionId, taskTitle, timeoutMs = 60000) => {
  try {
    await browser.waitUntil(
      async () =>
        await browser.execute(
          () => Boolean(document.querySelector(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")),
        ),
      {
        timeout: timeoutMs,
        timeoutMsg: `active-session textarea did not render for task ${taskId} / session ${sessionId}`,
      },
    );
  } catch (error) {
    const diagnostics = await readTaskSessionDiagnostics(taskId, sessionId, taskTitle);
    throw new Error(`${String(error)}\nDiagnostics: ${JSON.stringify(diagnostics)}`);
  }
};

const waitForCtxE2EDropBridge = async (timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => await browser.execute(() => typeof window.__ctxE2E?.emitDesktopDrop === "function"),
    { timeout: timeoutMs, timeoutMsg: "ctxE2E desktop drop bridge not available" },
  );
};

const emitNativeDrop = async (selector, filePath) => {
  const result = await browser.execute(async (targetSelector, droppedPath) => {
    if (!Array.isArray(window.__ctxDropFetchLog)) {
      const originalFetch = window.fetch.bind(window);
      window.__ctxDropFetchLog = [];
      window.fetch = async (...args) => {
        const [input] = args;
        const url = typeof input === "string" ? input : String(input?.url || input || "");
        try {
          const response = await originalFetch(...args);
          window.__ctxDropFetchLog.push({ url, ok: response.ok, status: response.status });
          return response;
        } catch (error) {
          window.__ctxDropFetchLog.push({ url, error: String(error) });
          throw error;
        }
      };
    }
    if (!Array.isArray(window.__ctxDesktopInvokeLog)) {
      const tauriInternals = window.__TAURI_INTERNALS__;
      const originalInvoke = tauriInternals?.invoke?.bind(tauriInternals);
      window.__ctxDesktopInvokeLog = [];
      if (typeof originalInvoke === "function") {
        const wrappedInvoke = async (...args) => {
          const [command, payload] = args;
          window.__ctxDesktopInvokeLog.push({ command: String(command || ""), payload: payload ?? null });
          return await originalInvoke(...args);
        };
        tauriInternals.invoke = wrappedInvoke;
        if (window.__TAURI__?.core) {
          window.__TAURI__.core.invoke = wrappedInvoke;
        }
      }
    }
    const dispatched = await window.__ctxE2E?.emitDesktopDrop?.(targetSelector, String(droppedPath));
    if (!dispatched?.ok) {
      return { ok: false, error: dispatched?.error || "ctxE2E desktop drop dispatch failed" };
    }
    const target = document.querySelector(targetSelector);
    if (!(target instanceof HTMLElement)) return { ok: false, error: `missing target after drop: ${targetSelector}` };
    const rect = target.getBoundingClientRect();
    const ratio = window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
    const cssX = rect.left + rect.width / 2;
    const cssY = rect.top + rect.height / 2;
    const pointEl = document.elementFromPoint(cssX, cssY);
    const pointChain = [];
    let current = pointEl;
    while (current instanceof HTMLElement && pointChain.length < 8) {
      pointChain.push({
        tag: current.tagName,
        className: current.className,
        testId: current.getAttribute("data-testid"),
      });
      current = current.parentElement;
    }
    window.__ctxDropLastDebug = {
      selector: targetSelector,
      droppedPath: String(droppedPath),
      convertFileSrcType: typeof window.__TAURI__?.core?.convertFileSrc,
      convertedPath:
        typeof window.__TAURI__?.core?.convertFileSrc === "function"
          ? window.__TAURI__.core.convertFileSrc(String(droppedPath))
          : null,
      tauriEventEmitType: typeof window.__TAURI__?.event?.emit,
      ratio,
      cssPosition: { x: cssX, y: cssY },
      pointChain,
      attachmentCounts: Array.from(document.querySelectorAll(".wb-attach-thumb-img")).length,
    };
    return { ok: true };
  }, selector, filePath);

  if (!result?.ok) {
    throw new Error(result?.error || "failed to emit native drop");
  }
};

const waitForAttachmentCount = async (selector, expectedCount, timeoutMs = 30000) => {
  try {
    await browser.waitUntil(
      async () => await browser.execute((s, count) => document.querySelectorAll(s).length === count, selector, expectedCount),
      { timeout: timeoutMs, timeoutMsg: `attachment count did not reach ${expectedCount} for ${selector}` },
    );
  } catch (error) {
    const diagnostics = await browser.execute(() => ({
      lastDebug: window.__ctxDropLastDebug || null,
      fetchLog: Array.isArray(window.__ctxDropFetchLog) ? window.__ctxDropFetchLog.slice(-12) : [],
      relevantFetches: Array.isArray(window.__ctxDropFetchLog)
        ? window.__ctxDropFetchLog.filter((entry) => !String(entry?.url || "").includes("plugin%3Aautomation%7Cresolve")).slice(-12)
        : [],
      nativeDropEvents: Array.isArray(window.__ctxNativeDropEvents) ? window.__ctxNativeDropEvents.slice(-12) : [],
      nativeDropDelivered: window.__ctxNativeDropDelivered || null,
      droppedImagePathCalls: Array.isArray(window.__ctxDroppedImagePathsCalls)
        ? window.__ctxDroppedImagePathsCalls.slice(-12)
        : [],
      desktopInvokes: Array.isArray(window.__ctxDesktopInvokeLog) ? window.__ctxDesktopInvokeLog.slice(-12) : [],
      fetchCount: Array.isArray(window.__ctxDropFetchLog) ? window.__ctxDropFetchLog.length : 0,
      totalThumbs: document.querySelectorAll(".wb-attach-thumb-img").length,
      composerThumbs: document.querySelectorAll(".wb-composer-attachments .wb-attach-thumb-img").length,
    }));
    throw new Error(`${String(error)}\nDiagnostics: ${JSON.stringify(diagnostics)}`);
  }
};

const waitForBlobBackedAttachments = async (selector, expectedCount, timeoutMs = 30000) => {
  await waitForAttachmentCount(selector, expectedCount, timeoutMs);
  try {
    await browser.waitUntil(
      async () =>
        await browser.execute((s, count) => {
          const images = Array.from(document.querySelectorAll(s));
          if (images.length !== count) return false;
          return images.every((image) => {
            const src = image.getAttribute("src") || image.src || "";
            return src.includes("/api/blobs/") && !src.startsWith("data:");
          });
        }, selector, expectedCount),
      { timeout: timeoutMs, timeoutMsg: `attachments were not blob-backed for ${selector}` },
    );
  } catch (error) {
    const diagnostics = await browser.execute((s) => ({
      lastDebug: window.__ctxDropLastDebug || null,
      fetchLog: Array.isArray(window.__ctxDropFetchLog) ? window.__ctxDropFetchLog.slice(-12) : [],
      relevantFetches: Array.isArray(window.__ctxDropFetchLog)
        ? window.__ctxDropFetchLog.filter((entry) => !String(entry?.url || "").includes("plugin%3Aautomation%7Cresolve")).slice(-12)
        : [],
      nativeDropEvents: Array.isArray(window.__ctxNativeDropEvents) ? window.__ctxNativeDropEvents.slice(-12) : [],
      nativeDropDelivered: window.__ctxNativeDropDelivered || null,
      droppedImagePathCalls: Array.isArray(window.__ctxDroppedImagePathsCalls)
        ? window.__ctxDroppedImagePathsCalls.slice(-12)
        : [],
      desktopInvokes: Array.isArray(window.__ctxDesktopInvokeLog) ? window.__ctxDesktopInvokeLog.slice(-12) : [],
      attachmentSrcs: Array.from(document.querySelectorAll(s)).map((image) => image.getAttribute("src") || image.src || ""),
    }), selector);
    throw new Error(`${String(error)}\nDiagnostics: ${JSON.stringify(diagnostics)}`);
  }
};

describe("desktop workbench image drag drop", () => {
  let repoRoot = "";
  let imagePath = "";

  before(() => {
    repoRoot = initTempRepo();
    imagePath = writeTinyPng();
  });

  after(() => {
    if (repoRoot) fs.rmSync(repoRoot, { recursive: true, force: true });
    if (imagePath) fs.rmSync(imagePath, { force: true });
  });

  it("attaches dropped images in new-task and active-session desktop composers", async () => {
    await browser.url("tauri://localhost?ctxE2E=1");
    await waitForTauri();

    const workspaceId = await createWorkspace(repoRoot);
    await browser.execute((id) => {
      window.location.href = `/workspaces/${id}?ctxE2E=1`;
    }, workspaceId);

    await waitForSelector("textarea.wb-new-composer-textarea", 60000);
    await waitForDropScopeReady("textarea.wb-new-composer-textarea", 30000);
    await waitForCtxE2EDropBridge(30000);

    const connection = await getConnectionInfo();
    if (!connection || connection.kind !== "local") {
      throw new Error(`expected local desktop connection: ${JSON.stringify(connection)}`);
    }

    await emitNativeDrop("textarea.wb-new-composer-textarea", imagePath);
    await waitForBlobBackedAttachments(".wb-new-composer-stack .wb-composer-attachments .wb-attach-thumb-img", 1, 30000);

    const taskTitle = `desktop-drop-fake-${Date.now()}`;
    const { taskId, sessionId } = await createTaskWithSession(workspaceId, taskTitle);
    await waitForCtxE2EFocusTask(30000);
    await waitForWorkspaceTaskSession(taskId, sessionId, taskTitle, 60000);
    const focusApplied = await browser.execute((nextTaskId, nextSessionId) => {
      return window.__ctxE2E?.focusTask?.(nextTaskId, nextSessionId) ?? false;
    }, taskId, sessionId);
    if (!focusApplied) {
      throw new Error(`ctxE2E focusTask failed for task ${taskId}`);
    }
    await waitForActiveSessionTextarea(taskId, sessionId, taskTitle, 60000);
    await waitForDropScopeReady(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea", 30000);

    await emitNativeDrop(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea", imagePath);
    await waitForBlobBackedAttachments(
      ".wb-session-slot[aria-hidden=\"false\"] .wb-composer-attachments .wb-attach-thumb-img",
      1,
      30000,
    );
  });
});
