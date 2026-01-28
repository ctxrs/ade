const path = require("path");

const WORKSPACE_PATH = process.env.CTX_AUTOMATION_WORKSPACE_PATH ||
  "/Users/example-user/code/ctx-monorepo";

const waitForTauri = async () => {
  await browser.waitUntil(
    async () => {
      const hasTauri = await browser.execute(() => Boolean(window.__TAURI__));
      return Boolean(hasTauri);
    },
    { timeout: 30000, timeoutMsg: "Tauri bridge not available in time." },
  );
};

const createWorkspace = async (rootPath) => {
  const result = await browser.executeAsync(async (root, done) => {
    try {
      const invoke = window.__TAURI__?.core?.invoke;
      if (!invoke) {
        done({ error: "Tauri invoke not available" });
        return;
      }
      await invoke("desktop_connect_local");
      const resp = await invoke("desktop_daemon_request", {
        req: {
          method: "POST",
          path: "/api/workspaces",
          body: JSON.stringify({ root_path: root }),
          headers: [["content-type", "application/json"]],
        },
      });
      const payload = JSON.parse(resp.body || "{}");
      done(payload.id || null);
    } catch (err) {
      done({ error: String(err) });
    }
  }, rootPath);

  if (result && typeof result === "object" && result.error) {
    throw new Error(result.error);
  }
  if (!result) {
    throw new Error("No workspace id returned from daemon.");
  }
  return result;
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
  });
});
