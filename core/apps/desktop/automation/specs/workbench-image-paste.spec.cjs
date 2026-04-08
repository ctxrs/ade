const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const { waitForTauri, getConnectionInfo } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");

const E2E_IMAGE_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+lmZYAAAAASUVORK5CYII=";
const WEBDRIVER_KEY_CONTROL = "\uE009";

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
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-paste-"));
  runChecked("git", ["init", "--", root]);
  runChecked("git", ["-C", root, "config", "user.email", "ctx-e2e@example.com"]);
  runChecked("git", ["-C", root, "config", "user.name", "ctx-e2e"]);
  fs.writeFileSync(path.join(root, "README.md"), "# desktop paste\n", "utf8");
  runChecked("git", ["-C", root, "add", "README.md"]);
  runChecked("git", ["-C", root, "commit", "-m", "init"]);
  return root;
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

const writeClipboardImage = () => {
  const filePath = path.join(os.tmpdir(), `ctx-desktop-paste-image-${process.pid}-${Date.now()}.png`);
  fs.writeFileSync(filePath, Buffer.from(E2E_IMAGE_BASE64, "base64"));
  return filePath;
};

const setSystemClipboardImage = (filePath) => {
  if (process.platform !== "darwin") {
    throw new Error("desktop image paste automation currently requires macOS clipboard support");
  }
  runChecked("osascript", [
    "-e",
    `set the clipboard to (read (POSIX file ${JSON.stringify(String(filePath))}) as «class PNGf»)`,
  ]);
};

const sendPasteShortcut = async () => {
  if (process.platform === "darwin") {
    runChecked("osascript", [
      "-e",
      'tell application "ctx" to activate',
      "-e",
      'tell application "System Events" to keystroke "v" using command down',
    ]);
    return;
  }
  const modifier = WEBDRIVER_KEY_CONTROL;
  await browser.performActions([{
    type: "key",
    id: "ctx-paste-keyboard",
    actions: [
      { type: "keyDown", value: modifier },
      { type: "keyDown", value: "v" },
      { type: "keyUp", value: "v" },
      { type: "keyUp", value: modifier },
    ],
  }]);
  await browser.releaseActions();
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
    description: "desktop paste fake task",
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
  if (executionEnvironment !== "host" && executionEnvironment !== "sandbox") {
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

const waitForCtxE2EFocusTask = async (timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => await browser.execute(() => typeof window.__ctxE2E?.focusTask === "function"),
    { timeout: timeoutMs, timeoutMsg: "ctxE2E focusTask bridge not available" },
  );
};

const getDesktopUploadInvokeCount = async () =>
  await browser.execute(() => {
    if (!Array.isArray(window.__ctxDesktopInvokeLog)) return 0;
    return window.__ctxDesktopInvokeLog.filter((entry) => entry?.command === "desktop_upload_blob").length;
  });

const waitForDesktopUploadInvokeCount = async (expectedCount, timeoutMs = 30000) => {
  try {
    await browser.waitUntil(
      async () => (await getDesktopUploadInvokeCount()) >= expectedCount,
      { timeout: timeoutMs, timeoutMsg: `desktop_upload_blob did not reach count ${expectedCount}` },
    );
  } catch (error) {
    const diagnostics = await browser.execute(() => ({
      composerPasteDebug: window.__ctxComposerPasteDebug || null,
      lastDebug: window.__ctxPasteLastDebug || null,
      pasteProbe: window.__ctxPasteProbe || null,
      desktopInvokes: Array.isArray(window.__ctxDesktopInvokeLog) ? window.__ctxDesktopInvokeLog.slice(-12) : [],
      fetchLog: Array.isArray(window.__ctxPasteFetchLog) ? window.__ctxPasteFetchLog.slice(-12) : [],
      hasTauriInternalsInvoke: typeof window.__TAURI_INTERNALS__?.invoke,
      hasGlobalTauriInvoke: typeof window.__TAURI__?.core?.invoke,
    }));
    throw new Error(`${String(error)}\nDiagnostics: ${JSON.stringify(diagnostics)}`);
  }
};

const installDesktopInvokeLog = async () => {
  await browser.execute(() => {
    if (!Array.isArray(window.__ctxPasteFetchLog)) {
      const originalFetch = window.fetch.bind(window);
      window.__ctxPasteFetchLog = [];
      window.fetch = async (...args) => {
        const [input] = args;
        const url = typeof input === "string" ? input : String(input?.url || input || "");
        try {
          const response = await originalFetch(...args);
          window.__ctxPasteFetchLog.push({ url, ok: response.ok, status: response.status });
          return response;
        } catch (error) {
          window.__ctxPasteFetchLog.push({ url, error: String(error) });
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
  });
};

const pasteImageFromClipboard = async (selector, clipboardImagePath) => {
  await installDesktopInvokeLog();
  await waitForSelector(selector, 30000);
  await browser.execute((targetSelector) => {
    const target = document.querySelector(targetSelector);
    const probe = {
      keydowns: [],
      pasteEvents: [],
      beforeInput: [],
      inputEvents: [],
      activeElementTag: document.activeElement?.tagName || null,
    };
    window.__ctxPasteProbe = probe;
    if (!(target instanceof HTMLElement)) return false;

    const recordPaste = (scope, event) => {
      const clipboardData = event.clipboardData;
      probe.pasteEvents.push({
        scope,
        types: Array.from(clipboardData?.types ?? []),
        files: Array.from(clipboardData?.files ?? []).map((file) => ({
          name: file.name,
          type: file.type,
          size: file.size,
        })),
        items: Array.from(clipboardData?.items ?? []).map((item) => ({
          kind: item.kind,
          type: item.type,
        })),
      });
    };
    const recordBeforeInput = (event) => {
      probe.beforeInput.push({
        inputType: event.inputType || null,
        data: event.data ?? null,
      });
    };
    const recordInput = (event) => {
      probe.inputEvents.push({
        inputType: event.inputType || null,
        data: event.data ?? null,
      });
    };
    const recordKeydown = (event) => {
      probe.keydowns.push({
        key: event.key,
        code: event.code,
        metaKey: event.metaKey,
        ctrlKey: event.ctrlKey,
      });
    };

    document.addEventListener("keydown", recordKeydown, { capture: true, once: false });
    document.addEventListener("paste", (event) => recordPaste("document", event), { capture: true, once: false });
    target.addEventListener("paste", (event) => recordPaste("target", event), { once: false });
    target.addEventListener("beforeinput", recordBeforeInput, { once: false });
    target.addEventListener("input", recordInput, { once: false });
    return true;
  }, selector);
  setSystemClipboardImage(clipboardImagePath);
  const focusApplied = await browser.execute((targetSelector) => {
    const target = document.querySelector(targetSelector);
    if (!(target instanceof HTMLElement)) return false;
    target.focus();
    target.click?.();
    if (target instanceof HTMLTextAreaElement) {
      const position = target.value.length;
      target.setSelectionRange(position, position);
    }
    return document.activeElement === target;
  }, selector);
  if (!focusApplied) {
    throw new Error(`failed to focus paste target: ${selector}`);
  }
  await browser.pause(250);
  await sendPasteShortcut();
  await browser.execute((targetSelector) => {
    window.__ctxPasteLastDebug = {
      selector: targetSelector,
      attachmentCounts: Array.from(document.querySelectorAll(".wb-attach-thumb-img")).length,
      invokeLogSize: Array.isArray(window.__ctxDesktopInvokeLog) ? window.__ctxDesktopInvokeLog.length : 0,
      activeElementTag: document.activeElement?.tagName || null,
      composerPasteDebug: window.__ctxComposerPasteDebug || null,
      pasteProbe: window.__ctxPasteProbe || null,
      fetchLog: Array.isArray(window.__ctxPasteFetchLog) ? window.__ctxPasteFetchLog.slice(-12) : [],
    };
  }, selector);
};

const waitForAttachmentCount = async (selector, expectedCount, timeoutMs = 30000) => {
  try {
    await browser.waitUntil(
      async () => await browser.execute((s, count) => document.querySelectorAll(s).length === count, selector, expectedCount),
      { timeout: timeoutMs, timeoutMsg: `attachment count did not reach ${expectedCount} for ${selector}` },
    );
  } catch (error) {
    const diagnostics = await browser.execute(() => ({
      lastDebug: window.__ctxPasteLastDebug || null,
      desktopInvokes: Array.isArray(window.__ctxDesktopInvokeLog) ? window.__ctxDesktopInvokeLog.slice(-12) : [],
      totalThumbs: document.querySelectorAll(".wb-attach-thumb-img").length,
      composerThumbs: document.querySelectorAll(".wb-composer-attachments .wb-attach-thumb-img").length,
    }));
    throw new Error(`${String(error)}\nDiagnostics: ${JSON.stringify(diagnostics)}`);
  }
};

describe("desktop workbench image paste", () => {
  let repoRoot = "";
  let clipboardImagePath = "";

  before(() => {
    repoRoot = initTempRepo();
    clipboardImagePath = writeClipboardImage();
  });

  after(() => {
    if (repoRoot) fs.rmSync(repoRoot, { recursive: true, force: true });
    if (clipboardImagePath) fs.rmSync(clipboardImagePath, { force: true });
  });

  it("attaches pasted images in new-task and active-session desktop composers", async () => {
    await browser.url("tauri://localhost?ctxE2E=1");
    await waitForTauri();

    const workspaceId = await createWorkspace(repoRoot);
    await browser.execute((id) => {
      window.location.href = `/workspaces/${id}?ctxE2E=1`;
    }, workspaceId);

    await waitForSelector("textarea.wb-new-composer-textarea", 60000);

    const connection = await getConnectionInfo();
    if (!connection || connection.kind !== "local") {
      throw new Error(`expected local desktop connection: ${JSON.stringify(connection)}`);
    }

    const uploadCountBeforeNewTaskPaste = await getDesktopUploadInvokeCount();
    await pasteImageFromClipboard("textarea.wb-new-composer-textarea", clipboardImagePath);
    await waitForDesktopUploadInvokeCount(uploadCountBeforeNewTaskPaste + 1, 30000);
    await waitForAttachmentCount(".wb-new-composer-stack .wb-composer-attachments .wb-attach-thumb-img", 1, 30000);

    const taskTitle = `desktop-paste-fake-${Date.now()}`;
    const { taskId, sessionId } = await createTaskWithSession(workspaceId, taskTitle);
    await waitForCtxE2EFocusTask(30000);
    const focusApplied = await browser.execute((nextTaskId, nextSessionId) => {
      return window.__ctxE2E?.focusTask?.(nextTaskId, nextSessionId) ?? false;
    }, taskId, sessionId);
    if (!focusApplied) {
      throw new Error(`ctxE2E focusTask failed for task ${taskId}`);
    }
    await waitForSelector(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea", 60000);

    const uploadCountBeforeActiveComposerPaste = await getDesktopUploadInvokeCount();
    await pasteImageFromClipboard(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea", clipboardImagePath);
    await waitForDesktopUploadInvokeCount(uploadCountBeforeActiveComposerPaste + 1, 30000);
    await waitForAttachmentCount(
      ".wb-session-slot[aria-hidden=\"false\"] .wb-composer-attachments .wb-attach-thumb-img",
      1,
      30000,
    );
  });
});
