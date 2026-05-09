const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const { navigateToTauriUrl, getConnectionInfo } = require("./helpers/tauri.cjs");
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
  const executionConfigResp = await daemonJson("GET", `/api/workspaces/${workspaceId}/execution_config`);
  if (executionConfigResp.status !== 200) {
    throw new Error(`execution config read failed: ${JSON.stringify(executionConfigResp)}`);
  }

  const executionEnvironment = String(executionConfigResp.payload?.environment || "").trim();
  if (executionEnvironment !== "host" && executionEnvironment !== "sandbox") {
    throw new Error(`unsupported execution environment: ${executionEnvironment || "<missing>"}`);
  }

  const { providerId, modelId } = await resolveSessionProvider(workspaceId);
  const resp = await daemonJson("POST", `/api/workspaces/${workspaceId}/tasks`, {
    title,
    description: "desktop paste fake task",
    default_session: {
      provider_id: providerId,
      model_id: modelId,
      execution_environment: executionEnvironment,
    },
  });
  if (resp.status !== 200 && resp.status !== 201) {
    throw new Error(`fake task creation failed (${providerId}/${modelId}): ${JSON.stringify(resp)}`);
  }
  const taskId = String(resp.payload?.id || "").trim();
  if (!taskId) throw new Error("fake task creation returned no id");
  const sessionId = String(resp.payload?.primary_session_id || "").trim();
  if (!sessionId) throw new Error("fake task creation returned no primary_session_id");

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

const waitForCtxE2EPasteBridge = async (timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => await browser.execute(() => typeof window.__ctxE2E?.pasteImageIntoComposer === "function"),
    { timeout: timeoutMs, timeoutMsg: "ctxE2E paste bridge not available" },
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

const pasteImageFromClipboard = async (selector) => {
  await installDesktopInvokeLog();
  await waitForCtxE2EPasteBridge(30000);
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
  const dispatched = await browser.execute(async (targetSelector) => {
    const result = await window.__ctxE2E?.pasteImageIntoComposer?.(targetSelector);
    return result ?? { ok: false, error: "ctxE2E paste bridge unavailable" };
  }, selector);
  if (!dispatched?.ok) {
    throw new Error(dispatched?.error || `failed to dispatch paste for ${selector}`);
  }
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
      lastDebug: window.__ctxPasteLastDebug || null,
      composerPasteDebug: window.__ctxComposerPasteDebug || null,
      desktopInvokes: Array.isArray(window.__ctxDesktopInvokeLog) ? window.__ctxDesktopInvokeLog.slice(-12) : [],
      fetchLog: Array.isArray(window.__ctxPasteFetchLog) ? window.__ctxPasteFetchLog.slice(-12) : [],
      attachmentSrcs: Array.from(document.querySelectorAll(s)).map((image) => image.getAttribute("src") || image.src || ""),
    }), selector);
    throw new Error(`${String(error)}\nDiagnostics: ${JSON.stringify(diagnostics)}`);
  }
};

describe("desktop workbench image paste", () => {
  let repoRoot = "";

  before(() => {
    repoRoot = initTempRepo();
  });

  after(() => {
    if (repoRoot) fs.rmSync(repoRoot, { recursive: true, force: true });
  });

  it("attaches pasted images in new-task and active-session desktop composers", async () => {
    await navigateToTauriUrl("tauri://localhost?ctxE2E=1");

    const workspaceId = await createWorkspace(repoRoot);
    await browser.execute((id) => {
      window.location.href = `/workspaces/${id}?ctxE2E=1`;
    }, workspaceId);

    await waitForSelector("textarea.wb-new-composer-textarea", 60000);

    const connection = await getConnectionInfo();
    if (!connection || connection.kind !== "local") {
      throw new Error(`expected local desktop connection: ${JSON.stringify(connection)}`);
    }

    await pasteImageFromClipboard("textarea.wb-new-composer-textarea");
    await waitForBlobBackedAttachments(".wb-new-composer-stack .wb-composer-attachments .wb-attach-thumb-img", 1, 30000);

    const taskTitle = `desktop-paste-fake-${Date.now()}`;
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

    await pasteImageFromClipboard(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea");
    await waitForBlobBackedAttachments(
      ".wb-session-slot[aria-hidden=\"false\"] .wb-composer-attachments .wb-attach-thumb-img",
      1,
      30000,
    );
  });
});
