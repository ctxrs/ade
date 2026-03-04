import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import type { Page } from "playwright/test";

const AUTO_APPLY_ON_LAUNCH_STORAGE_KEY = "ctx_update_auto_apply_on_launch_v1";
const PROMPT_SNOOZE_STORAGE_KEY = "ctx_update_prompt_next_allowed_at_v1";
const IDLE_UPDATE_VERSION_STORAGE_KEY = "ctx_update_prompt_idle_versions_v1";
const RESTART_REQUIRED_VERSION_STORAGE_KEY = "ctx_update_restart_required_version_v1";

type DesktopUpdateState = {
  configured: boolean;
  available: boolean;
  restart_required: boolean;
  current_version: string;
  latest_version: string | null;
  target: string;
  endpoint: string;
  message: string | null;
};

type DesktopApplyResponse = {
  applied: boolean;
  needs_restart: boolean;
  up_to_date: boolean;
  latest_version: string | null;
  message: string;
};

type HarnessConfig = {
  updateState: DesktopUpdateState;
  applyResponse: DesktopApplyResponse;
};

type HarnessState = {
  updateState: DesktopUpdateState;
  applyResponse: DesktopApplyResponse;
  invokeCalls: string[];
};

const createWorkspaceAndOpenWorkbench = async (page: Page, workspaceName: string) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-updater-desktop-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "desktop updater e2e fixture\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceResp = await page.request.post("/api/workspaces", {
    data: {
      root_path: repo,
      name: workspaceName,
    },
  });
  expect(workspaceResp.ok()).toBeTruthy();
  const workspaceJson = (await workspaceResp.json()) as { id: string };
  expect(typeof workspaceJson.id).toBe("string");
  await page.goto(`/workspaces/${workspaceJson.id}`, { waitUntil: "domcontentloaded" });
};

const installDesktopHarness = async (page: Page, config: HarnessConfig) => {
  await page.addInitScript((initial: HarnessConfig) => {
    type TauriInvoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
    type TauriInternals = {
      invoke?: TauriInvoke;
      transformCallback?: (cb: unknown, once?: boolean) => number;
      unregisterCallback?: (id: number) => void;
      metadata?: Record<string, unknown>;
    } & Record<string, unknown>;

    type TauriWindow = Window & {
      __TAURI__?: { core?: { invoke?: TauriInvoke } };
      __TAURI_INTERNALS__?: TauriInternals;
      __ctxDesktopUpdaterE2E?: HarnessState;
    };

    const w = window as TauriWindow;
    const state: HarnessState = {
      updateState: { ...initial.updateState },
      applyResponse: { ...initial.applyResponse },
      invokeCalls: [],
    };

    const invoke: TauriInvoke = async (cmd, rawArgs) => {
      const name = String(cmd || "");
      state.invokeCalls.push(name);
      if (name === "plugin:app|version") {
        return state.updateState.current_version;
      }
      if (name === "desktop_get_connection") {
        return {
          kind: "local",
          base_url: "http://127.0.0.1:4399",
          token: "local-token",
        };
      }
      if (name === "desktop_get_app_update_state") {
        return {
          ...state.updateState,
        };
      }
      if (name === "desktop_apply_app_update") {
        const response = { ...state.applyResponse };
        if (response.needs_restart) {
          state.updateState = {
            ...state.updateState,
            available: false,
            restart_required: true,
            latest_version: response.latest_version ?? state.updateState.latest_version,
          };
        } else if (response.applied || response.up_to_date) {
          const nextVersion = response.latest_version ?? state.updateState.latest_version ?? state.updateState.current_version;
          state.updateState = {
            ...state.updateState,
            available: false,
            restart_required: false,
            current_version: nextVersion,
            latest_version: null,
          };
        }
        return response;
      }
      if (name === "desktop_restart_app") {
        return { requested: true, message: "Restart requested." };
      }
      if (name === "desktop_daemon_request") {
        const args =
          rawArgs && typeof rawArgs === "object"
            ? (rawArgs as Record<string, unknown>)
            : {};
        const req =
          args.req && typeof args.req === "object"
            ? (args.req as Record<string, unknown>)
            : {};
        const path = String(req.path ?? "/");
        const method = String(req.method ?? "GET").toUpperCase();
        const body = typeof req.body === "string" ? req.body : "";
        const headers = req.headers && typeof req.headers === "object" ? (req.headers as Record<string, unknown>) : {};
        const fetchHeaders: Record<string, string> = {};
        for (const [key, value] of Object.entries(headers)) {
          if (typeof value === "string") {
            fetchHeaders[key] = value;
          }
        }
        if (!fetchHeaders.authorization) {
          fetchHeaders.authorization = "Bearer ctx-e2e-auth-token";
        }
        if (!fetchHeaders["content-type"] && body) {
          fetchHeaders["content-type"] = "application/json";
        }
        const response = await fetch(path, {
          method,
          headers: fetchHeaders,
          body: method === "GET" || method === "HEAD" ? undefined : body,
        });
        const text = await response.text();
        return {
          status: response.status,
          content_type: response.headers.get("content-type") ?? "application/json",
          body: text,
        };
      }
      return null;
    };

    const existingInternals = w.__TAURI_INTERNALS__ ?? {};
    const metadata =
      existingInternals.metadata && typeof existingInternals.metadata === "object"
        ? existingInternals.metadata
        : {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        };
    w.__TAURI_INTERNALS__ = {
      ...existingInternals,
      metadata,
      transformCallback:
        typeof existingInternals.transformCallback === "function"
          ? existingInternals.transformCallback
          : () => 1,
      unregisterCallback:
        typeof existingInternals.unregisterCallback === "function"
          ? existingInternals.unregisterCallback
          : () => {},
      invoke,
    };

    const existingTauri = w.__TAURI__ ?? {};
    const existingCore = existingTauri.core ?? {};
    w.__TAURI__ = {
      ...existingTauri,
      core: {
        ...existingCore,
        invoke,
      },
    };
    w.__ctxDesktopUpdaterE2E = state;
  }, config);
};

const desktopCommandCallCount = async (page: Page, command: string): Promise<number> => {
  return await page.evaluate((name: string) => {
    const w = window as Window & { __ctxDesktopUpdaterE2E?: HarnessState };
    const calls = w.__ctxDesktopUpdaterE2E?.invokeCalls ?? [];
    return calls.filter((cmd) => cmd === name).length;
  }, command);
};

const installUpdatePolicyRoute = async (page: Page, updateAvailable = true) => {
  await page.route("**/api/updates/check**", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        channel: "stable",
        base_url: "https://example.com",
        platform: "macos-arm64",
        current_version: "1.0.0",
        latest_version: "9.9.9",
        update_available: updateAvailable,
        platform_supported: true,
        in_place_update_supported: true,
        in_place_update_reason: null,
      }),
    });
  });
};

test("desktop updater remains silent while only available/staging", async ({ page }) => {
  await page.addInitScript((autoApplyKey: string, snoozeKey: string, idleKey: string, restartKey: string) => {
    localStorage.removeItem("ctx_update_check_v1");
    localStorage.removeItem(snoozeKey);
    localStorage.removeItem(idleKey);
    localStorage.setItem(autoApplyKey, "0");
    sessionStorage.removeItem(restartKey);
  }, AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, PROMPT_SNOOZE_STORAGE_KEY, IDLE_UPDATE_VERSION_STORAGE_KEY, RESTART_REQUIRED_VERSION_STORAGE_KEY);
  await installDesktopHarness(page, {
    updateState: {
      configured: true,
      available: true,
      restart_required: false,
      current_version: "0.4.7",
      latest_version: "0.4.8",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    },
    applyResponse: {
      applied: true,
      needs_restart: true,
      up_to_date: false,
      latest_version: "0.4.8",
      message: "Update takes ~1 second and preserves data. Active agents will be paused.",
    },
  });
  await installUpdatePolicyRoute(page, true);

  await createWorkspaceAndOpenWorkbench(page, `ws-desktop-banner-${Date.now()}`);
  await expect.poll(async () => desktopCommandCallCount(page, "desktop_get_app_update_state")).toBeGreaterThan(0);
  await expect(page.getByTestId("update-available-snackbar")).toHaveCount(0);
});

test("desktop Update Now requests restart in restart-required state", async ({ page }) => {
  await page.addInitScript((autoApplyKey: string, snoozeKey: string, idleKey: string, restartKey: string) => {
    localStorage.removeItem("ctx_update_check_v1");
    localStorage.removeItem(snoozeKey);
    localStorage.removeItem(idleKey);
    localStorage.setItem(autoApplyKey, "0");
    sessionStorage.removeItem(restartKey);
  }, AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, PROMPT_SNOOZE_STORAGE_KEY, IDLE_UPDATE_VERSION_STORAGE_KEY, RESTART_REQUIRED_VERSION_STORAGE_KEY);
  await installDesktopHarness(page, {
    updateState: {
      configured: true,
      available: false,
      restart_required: true,
      current_version: "0.4.7",
      latest_version: "0.4.8",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    },
    applyResponse: {
      applied: true,
      needs_restart: true,
      up_to_date: false,
      latest_version: "0.4.8",
      message: "Update takes ~1 second and preserves data. Active agents will be paused.",
    },
  });
  await installUpdatePolicyRoute(page, true);

  await createWorkspaceAndOpenWorkbench(page, `ws-desktop-apply-now-${Date.now()}`);
  await expect(page.getByTestId("update-available-snackbar")).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Update Now" }).dispatchEvent("click");
  await expect.poll(async () => desktopCommandCallCount(page, "desktop_restart_app")).toBe(1);
  await expect.poll(async () => desktopCommandCallCount(page, "desktop_apply_app_update")).toBe(0);
  await expect(page.getByRole("button", { name: "Update Now" })).toBeVisible({ timeout: 20_000 });
});

test("desktop Update on Next Idle schedules restart when restart is ready", async ({ page }) => {
  await page.addInitScript((autoApplyKey: string, snoozeKey: string, idleKey: string, restartKey: string) => {
    localStorage.removeItem("ctx_update_check_v1");
    localStorage.removeItem(snoozeKey);
    localStorage.removeItem(idleKey);
    localStorage.setItem(autoApplyKey, "0");
    sessionStorage.removeItem(restartKey);
  }, AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, PROMPT_SNOOZE_STORAGE_KEY, IDLE_UPDATE_VERSION_STORAGE_KEY, RESTART_REQUIRED_VERSION_STORAGE_KEY);
  await installDesktopHarness(page, {
    updateState: {
      configured: true,
      available: false,
      restart_required: true,
      current_version: "0.4.7",
      latest_version: "0.4.8",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    },
    applyResponse: {
      applied: true,
      needs_restart: true,
      up_to_date: false,
      latest_version: "0.4.8",
      message: "Update takes ~1 second and preserves data. Active agents will be paused.",
    },
  });
  await installUpdatePolicyRoute(page, true);

  await createWorkspaceAndOpenWorkbench(page, `ws-desktop-apply-idle-${Date.now()}`);
  await expect(page.getByTestId("update-available-snackbar")).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Update on Next Idle" }).dispatchEvent("click");
  await expect.poll(async () => desktopCommandCallCount(page, "desktop_restart_app")).toBe(1);
  await expect.poll(async () => desktopCommandCallCount(page, "desktop_apply_app_update")).toBe(0);
  await expect(page.getByRole("button", { name: "Update Now" })).toBeVisible({ timeout: 20_000 });
});

test("desktop auto-apply on launch triggers update apply when available", async ({ page }) => {
  await page.addInitScript((autoApplyKey: string, snoozeKey: string, idleKey: string, restartKey: string) => {
    localStorage.removeItem("ctx_update_check_v1");
    localStorage.removeItem(snoozeKey);
    localStorage.removeItem(idleKey);
    localStorage.setItem(autoApplyKey, "1");
    sessionStorage.removeItem(restartKey);
  }, AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, PROMPT_SNOOZE_STORAGE_KEY, IDLE_UPDATE_VERSION_STORAGE_KEY, RESTART_REQUIRED_VERSION_STORAGE_KEY);
  await installDesktopHarness(page, {
    updateState: {
      configured: true,
      available: true,
      restart_required: false,
      current_version: "0.4.7",
      latest_version: "0.4.8",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    },
    applyResponse: {
      applied: true,
      needs_restart: true,
      up_to_date: false,
      latest_version: "0.4.8",
      message: "Update takes ~1 second and preserves data. Active agents will be paused.",
    },
  });
  await installUpdatePolicyRoute(page, true);

  await createWorkspaceAndOpenWorkbench(page, `ws-desktop-auto-${Date.now()}`);
  await expect.poll(async () => desktopCommandCallCount(page, "desktop_apply_app_update")).toBe(1);
  await expect(page.getByRole("button", { name: "Update Now" })).toBeVisible({ timeout: 20_000 });
});
