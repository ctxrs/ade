const path = require("path");
const { waitForTauri, getConnectionInfo } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");

const DEFAULT_WORKSPACE_PATH = path.resolve(__dirname, "../../../../../");
const WORKSPACE_PATH = process.env.CTX_AUTOMATION_WORKSPACE_PATH
  || process.env.GITHUB_WORKSPACE
  || DEFAULT_WORKSPACE_PATH;

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
  const workspaceId = resp.payload?.id || null;
  if (!workspaceId) {
    throw new Error("No workspace id returned from daemon.");
  }
  return workspaceId;
};

describe("desktop automation", () => {
  it("opens a workspace in the Tauri app", async () => {
    await browser.url("tauri://localhost");
    await waitForTauri();

    const workspaceId = await createWorkspace(WORKSPACE_PATH);
    await browser.execute((id) => {
      window.location.href = `/workspaces/${id}`;
    }, workspaceId);

    await browser.waitUntil(
      async () => {
        const path = await browser.execute(() => window.location.pathname);
        return typeof path === "string" && path.startsWith("/workspaces/");
      },
      { timeout: 60000, timeoutMsg: "Workbench did not load workspace." },
    );

    const info = await getConnectionInfo();
    if (!info || info.kind !== "local") {
      throw new Error(`expected local desktop connection after opening workspace: ${JSON.stringify(info)}`);
    }
  });
});
