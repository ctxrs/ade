const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const { waitForTauri, getConnectionInfo } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");

const DESKTOP_DRAG_DROP_TEST_EVENT = "ctx:desktop-drag-drop-test";

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

const emitNativeDrop = async (selector, filePath) => {
  const result = await browser.execute(async (targetSelector, droppedPath, testEventName) => {
    const target = document.querySelector(targetSelector);
    if (!(target instanceof HTMLElement)) return { ok: false, error: `missing target: ${targetSelector}` };
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

    const rect = target.getBoundingClientRect();
    const ratio = window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
    const position = {
      x: Math.round((rect.left + rect.width / 2) * ratio),
      y: Math.round((rect.top + rect.height / 2) * ratio),
    };
    const payload = {
      paths: [String(droppedPath)],
      position,
    };
    const emit = async (eventPayload) => {
      const tauriEmit = window.__TAURI__?.event?.emit;
      if (typeof tauriEmit === "function") {
        await tauriEmit(testEventName, eventPayload);
        return;
      }
      window.dispatchEvent(new CustomEvent(testEventName, { detail: eventPayload }));
    };
    await emit({ type: "enter", ...payload });
    await emit({ type: "over", position });
    await emit({ type: "drop", ...payload });
    const cssX = position.x / ratio;
    const cssY = position.y / ratio;
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
      position,
      ratio,
      cssPosition: { x: cssX, y: cssY },
      pointChain,
      attachmentCounts: Array.from(document.querySelectorAll(".wb-attach-thumb-img")).length,
    };
    return { ok: true };
  }, selector, filePath, DESKTOP_DRAG_DROP_TEST_EVENT);

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
      fetchCount: Array.isArray(window.__ctxDropFetchLog) ? window.__ctxDropFetchLog.length : 0,
      totalThumbs: document.querySelectorAll(".wb-attach-thumb-img").length,
      composerThumbs: document.querySelectorAll(".wb-composer-attachments .wb-attach-thumb-img").length,
    }));
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

    const connection = await getConnectionInfo();
    if (!connection || connection.kind !== "local") {
      throw new Error(`expected local desktop connection: ${JSON.stringify(connection)}`);
    }

    await emitNativeDrop("textarea.wb-new-composer-textarea", imagePath);
    await waitForAttachmentCount(".wb-new-composer-stack .wb-composer-attachments .wb-attach-thumb-img", 1, 30000);

    const taskTitle = `desktop-drop-fake-${Date.now()}`;
    const { taskId, sessionId } = await createTaskWithSession(workspaceId, taskTitle);
    await waitForCtxE2EFocusTask(30000);
    const focusApplied = await browser.execute((nextTaskId, nextSessionId) => {
      return window.__ctxE2E?.focusTask?.(nextTaskId, nextSessionId) ?? false;
    }, taskId, sessionId);
    if (!focusApplied) {
      throw new Error(`ctxE2E focusTask failed for task ${taskId}`);
    }
    await waitForSelector(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea", 60000);
    await waitForDropScopeReady(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea", 30000);

    await emitNativeDrop(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea", imagePath);
    await waitForAttachmentCount(
      ".wb-session-slot[aria-hidden=\"false\"] .wb-composer-attachments .wb-attach-thumb-img",
      1,
      30000,
    );
  });
});
