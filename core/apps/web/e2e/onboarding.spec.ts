import { execSync } from "child_process";
import { mkdtempSync, realpathSync, writeFileSync } from "fs";
import os from "os";
import path from "path";
import { test, expect, type APIRequestContext, type Page } from "./utils/fixtures";

const initRepo = (baseDir: string): string => {
  const repo = mkdtempSync(path.join(baseDir, "ctx-e2e-onboarding-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "fixture\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });
  return repo;
};

const installDesktopStub = async (page: Page) => {
  await page.addInitScript(() => {
    const bridge: any = {
      isDesktopApp: true,
      connection: { kind: "none", base_url: null, token: null },
      daemonSettings: {
        auto_start: true,
        launch_mode: null,
        docker_passthrough: null,
        last_connection: null,
        last_workspace_id: null,
      },
      lastConnectLocal: null,
      invoke: async (cmd: string, args?: any) => {
        const baseUrl = window.location.origin;
        switch (cmd) {
          case "desktop_get_connection":
            return bridge.connection;
          case "desktop_get_daemon_settings":
            return bridge.daemonSettings;
          case "desktop_connect_local":
            bridge.lastConnectLocal = args?.req ?? null;
            bridge.connection = {
              kind: "local",
              base_url: baseUrl,
              token: sessionStorage.getItem("ctxAuthToken"),
            };
            return bridge.connection;
          case "desktop_set_last_workspace":
            bridge.daemonSettings.last_workspace_id = args?.workspace_id ?? null;
            return null;
          case "desktop_pick_folder": {
            const pick = bridge.nextPickFolder ?? null;
            bridge.nextPickFolder = null;
            return pick;
          }
          case "desktop_daemon_request": {
            const req = args?.req ?? {};
            const headers = new Headers(req.headers ?? []);
            const init: RequestInit = { method: req.method, headers };
            if (req.body !== null && req.body !== undefined) {
              init.body = req.body;
            }
            const resp = await fetch(new URL(req.path, baseUrl).toString(), init);
            return {
              status: resp.status,
              body: await resp.text(),
              content_type: resp.headers.get("content-type"),
            };
          }
          default:
            throw new Error(`Unhandled desktop command: ${cmd}`);
        }
      },
    };
    (window as any).__CTX_DESKTOP_BRIDGE__ = bridge;
  });
};

const runWizard = async (
  page: Page,
  {
    executionMode,
    workspacePath,
  }: {
    executionMode: "host" | "container";
    workspacePath: string;
  },
) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("launcher-host-local").check();
  await page.getByTestId("launcher-wizard-next").click();

  if (executionMode === "container") {
    await page.getByTestId("launcher-mode-container").check();
  } else {
    await page.getByTestId("launcher-mode-host").check();
  }
  await page.getByTestId("launcher-wizard-next").click();

  await page.getByTestId("launcher-workspace-path").fill(workspacePath);
  await page.getByTestId("launcher-wizard-next").click();
  await page.getByTestId("launcher-wizard-next").click();

  await page.getByTestId("launcher-open-workbench").waitFor({ timeout: 20000 });
  await page.getByTestId("launcher-open-workbench").click();
  await expect(page).toHaveURL(/\/workspaces\/[^/]+/);
};

const waitForWorkspaceRoot = async (request: APIRequestContext, rootPath: string) => {
  await expect.poll(async () => {
    const resp = await request.get("/api/workspaces");
    if (!resp.ok()) return null;
    const workspaces = (await resp.json()) as Array<{ root_path?: string }>;
    return workspaces.find((workspace) => workspace.root_path === rootPath) ?? null;
  }).not.toBeNull();
};

test("launcher onboarding: host mode creates workspace", async ({ page, request }) => {
  const repo = initRepo(os.tmpdir());
  const canonical = realpathSync(repo);

  await installDesktopStub(page);
  await runWizard(page, { executionMode: "host", workspacePath: canonical });
  const lastConnect = await page.evaluate(() => (window as any).__CTX_DESKTOP_BRIDGE__?.lastConnectLocal ?? null);
  expect(lastConnect?.launch_mode).toBe("host");
  await waitForWorkspaceRoot(request, canonical);
});

test("launcher onboarding: container mode creates workspace", async ({ page, request }) => {
  test.skip(process.platform !== "linux", "Container mode is only supported on Linux.");
  const repo = initRepo(os.tmpdir());
  const canonical = realpathSync(repo);

  await installDesktopStub(page);
  await runWizard(page, { executionMode: "container", workspacePath: canonical });
  const lastConnect = await page.evaluate(() => (window as any).__CTX_DESKTOP_BRIDGE__?.lastConnectLocal ?? null);
  expect(lastConnect?.launch_mode).toBe("container");
  await waitForWorkspaceRoot(request, canonical);
});

test("launcher onboarding: tilde path expands to canonical root", async ({ page, request }) => {
  const repo = initRepo(os.homedir());
  const canonical = realpathSync(repo);
  const relative = path.relative(os.homedir(), canonical).split(path.sep).join("/");
  const tildePath = `~/${relative}`;

  await installDesktopStub(page);
  await runWizard(page, { executionMode: "host", workspacePath: tildePath });
  const lastConnect = await page.evaluate(() => (window as any).__CTX_DESKTOP_BRIDGE__?.lastConnectLocal ?? null);
  expect(lastConnect?.launch_mode).toBe("host");
  await waitForWorkspaceRoot(request, canonical);
});
