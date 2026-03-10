const path = require("path");
const { waitForTauri, waitForTestId } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");

const DEFAULT_WORKSPACE_PATH = path.resolve(__dirname, "../../../../../");
const WORKSPACE_PATH = process.env.CTX_AUTOMATION_WORKSPACE_PATH
  || process.env.GITHUB_WORKSPACE
  || DEFAULT_WORKSPACE_PATH;

const connectLocal = async () => {
  const result = await browser.execute(async () => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const info = await invoke("desktop_connect_local");
      return { info };
    } catch (err) {
      return { error: String(err) };
    }
  });
  if (result && typeof result === "object" && result.error) {
    throw new Error(result.error);
  }
  if (!result || !result.info || result.info.kind !== "local") {
    throw new Error(`expected local connection info, got: ${JSON.stringify(result)}`);
  }
};

const createWorkspace = async (rootPath) => {
  const resp = await daemonJson("POST", "/api/workspaces", { root_path: rootPath });
  if (resp.status !== 200 && resp.status !== 201) {
    throw new Error(`workspace creation failed: ${JSON.stringify(resp)}`);
  }
  const workspaceId = resp.payload?.id || null;
  if (!workspaceId) {
    throw new Error(`workspace id missing from response: ${JSON.stringify(resp)}`);
  }
  return workspaceId;
};

const collectArchivedToggleDiagnostics = async () =>
  await browser.execute(() => {
    const text = (value) => String(value || "").replace(/\s+/g, " ").trim();
    const buttons = Array.from(document.querySelectorAll("button"))
      .map((btn) => text(btn.textContent))
      .filter(Boolean);
    const sectionTitles = Array.from(document.querySelectorAll(".wb-section-title"))
      .map((el) => text(el.textContent))
      .filter(Boolean);
    const mutedTexts = Array.from(document.querySelectorAll(".wb-muted"))
      .map((el) => text(el.textContent))
      .filter(Boolean);
    return {
      pathname: window.location.pathname,
      workbenchTaskSearch: Boolean(document.querySelector('[data-testid="workbench-task-search"]')),
      taskListCount: document.querySelectorAll(".wb-task-list").length,
      taskScrollCount: document.querySelectorAll(".wb-task-scroll").length,
      buttons,
      sectionTitles,
      mutedTexts,
      bodyPreview: text(document.body?.innerText || "").slice(0, 800),
    };
  });

const clickArchivedTasksToggle = async () => {
  try {
    await browser.waitUntil(
      async () =>
        await browser.execute(() => {
          const buttons = Array.from(document.querySelectorAll("button"));
          return buttons.some((btn) => String(btn.textContent || "").includes("Archived Tasks"));
        }),
      { timeout: 30000, timeoutMsg: "Archived Tasks toggle not found." },
    );
  } catch (err) {
    const diagnostics = await collectArchivedToggleDiagnostics();
    throw new Error(`Archived Tasks toggle not found. diagnostics=${JSON.stringify(diagnostics)} cause=${String(err)}`);
  }
  const clicked = await browser.execute(() => {
    const buttons = Array.from(document.querySelectorAll("button"));
    const target = buttons.find((btn) => String(btn.textContent || "").includes("Archived Tasks"));
    if (!target) return false;
    target.click();
    return true;
  });
  if (!clicked) {
    throw new Error("Failed to click Archived Tasks toggle.");
  }
};

const readArchivedTasksUiState = async () =>
  await browser.execute(() => {
    const text = (value) => String(value || "").replace(/\s+/g, " ").trim();
    const mutedTexts = Array.from(document.querySelectorAll(".wb-muted"))
      .map((el) => text(el.textContent))
      .filter(Boolean);
    const archivedToggle = Array.from(document.querySelectorAll("button"))
      .find((btn) => text(btn.textContent).includes("Archived Tasks"));
    const taskTitles = Array.from(document.querySelectorAll(".wb-task-title"))
      .map((el) => text(el.textContent))
      .filter(Boolean);
    const snapshot = window.__ctxE2E?.getWorkspaceSnapshot?.() ?? null;
    return {
      mutedTexts,
      hasEmpty: mutedTexts.includes("No archived tasks."),
      hasError: mutedTexts.some((text) => text.includes("Failed to load archived tasks")),
      hasLoading: Boolean(document.querySelector(".wb-archived-loading")),
      archivedExpanded: archivedToggle ? archivedToggle.getAttribute("aria-expanded") : null,
      taskTitles,
      bodyPreview: text(document.body?.innerText || "").slice(0, 800),
      snapshot: snapshot
        ? {
          archivedIds: Array.isArray(snapshot.archivedIds) ? snapshot.archivedIds : [],
          archivedLoaded: Boolean(snapshot.archivedLoaded),
          fetchState: snapshot.fetchState || null,
          hasMoreArchived: Boolean(snapshot.hasMoreArchived),
          initialized: Boolean(snapshot.initialized),
        }
        : null,
    };
  });

const readArchivedFailureDiagnostics = async () =>
  await browser.execute(() => {
    const events = window.__ctxE2E?.getDiagnostics?.();
    if (!Array.isArray(events)) return [];
    return events.filter((event) => {
      const code = String(event?.code || "");
      if (code === "workspace.archived_load_failed") return true;
      if (code === "api.transport_error" || code === "api.http_error") {
        return String(event?.context?.path || "").includes("/archived_task_summaries");
      }
      return false;
    });
  });

const assertNoArchivedLoadFailedDiagnostics = async () => {
  await browser.waitUntil(
    async () =>
      await browser.execute(() => {
        return typeof window.__ctxE2E?.getDiagnostics === "function";
      }),
    { timeout: 30000, timeoutMsg: "ctxE2E diagnostics bridge not available." },
  );
  const failures = await browser.execute(() => {
    const events = window.__ctxE2E?.getDiagnostics?.();
    if (!Array.isArray(events)) return [];
    return events.filter((event) => event && event.code === "workspace.archived_load_failed");
  });
  if (Array.isArray(failures) && failures.length > 0) {
    throw new Error(`workspace.archived_load_failed diagnostics detected: ${JSON.stringify(failures)}`);
  }
};

describe("desktop archived tasks loading", () => {
  it("loads archived tasks without failure state for a new workspace", async () => {
    await browser.url("tauri://localhost");
    await waitForTauri();
    await connectLocal();

    const workspaceId = await createWorkspace(WORKSPACE_PATH);
    const archivedResp = await daemonJson(
      "GET",
      `/api/workspaces/${workspaceId}/archived_task_summaries`,
    );
    if (archivedResp.status !== 200) {
      throw new Error(`archived task summaries request failed: ${JSON.stringify(archivedResp)}`);
    }

    await browser.execute((id) => {
      window.location.href = `/workspaces/${id}?ctxE2E=1`;
    }, workspaceId);
    await waitForTestId("workbench-task-search", 30000);

    await clickArchivedTasksToggle();
    try {
      await browser.waitUntil(
        async () => {
          const state = await readArchivedTasksUiState();
          return state.hasEmpty || state.hasError;
        },
        { timeout: 30000, timeoutMsg: "Archived tasks did not reach a terminal state." },
      );
    } catch (err) {
      const state = await readArchivedTasksUiState();
      const diagnostics = await readArchivedFailureDiagnostics();
      throw new Error(
        `Archived tasks terminal state wait failed: ${String(err)} state=${JSON.stringify(state)} diagnostics=${JSON.stringify(diagnostics)}`,
      );
    }
    const finalState = await readArchivedTasksUiState();
    if (finalState.hasError) {
      const diagnostics = await readArchivedFailureDiagnostics();
      throw new Error(
        `Archived tasks rendered load failure state: state=${JSON.stringify(finalState)} diagnostics=${JSON.stringify(diagnostics)}`,
      );
    }
    if (!finalState.hasEmpty) {
      throw new Error(`Archived tasks did not render empty state: ${JSON.stringify(finalState)}`);
    }
    await assertNoArchivedLoadFailedDiagnostics();
  });
});
