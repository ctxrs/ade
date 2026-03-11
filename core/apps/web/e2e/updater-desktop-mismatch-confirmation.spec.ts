import { test, expect } from "./fixtures";
import type { Page } from "playwright/test";

type ConnectionKind = "local" | "ssh";

type HarnessConfig = {
  connectionKind: ConnectionKind;
  desktopVersion: string;
  daemonVersion: string;
  confirmResult: boolean;
};

type HarnessState = {
  config: HarnessConfig;
  invokeCalls: string[];
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
      __ctxUpdaterE2E?: HarnessState;
    };

    const w = window as TauriWindow;
    const state: HarnessState = {
      config: {
        connectionKind: initial.connectionKind,
        desktopVersion: initial.desktopVersion,
        daemonVersion: initial.daemonVersion,
        confirmResult: initial.confirmResult,
      },
      invokeCalls: [],
    };

    const currentDaemonTarget = () => {
      const parsed = new URL(window.location.origin);
      const port = parsed.port
        ? Number(parsed.port)
        : parsed.protocol === "https:"
          ? 443
          : 80;
      return {
        base_url: parsed.origin,
        port,
      };
    };

    const mkHealth = () => ({
      version: state.config.daemonVersion,
      daemon_version: state.config.daemonVersion,
      pid: 1,
      data_root: "/tmp",
      daemon_url: currentDaemonTarget().base_url,
      auth_required: false,
      compatibility: {
        desktop_exact_version: state.config.daemonVersion,
        mobile_api_min: 1,
        mobile_api_max: 1,
      },
    });

    const invoke: TauriInvoke = async (cmd, rawArgs) => {
      const name = String(cmd || "");
      state.invokeCalls.push(name);

      if (name === "plugin:app|version") {
        return state.config.desktopVersion;
      }
      if (name === "desktop_get_connection") {
        const target = currentDaemonTarget();
        if (state.config.connectionKind === "ssh") {
          return {
            kind: "ssh",
            base_url: target.base_url,
            token: "ssh-token",
            host: "example.test",
            user: "devbox",
            remote_port: target.port,
            remote_data_dir: "/tmp/ctx-remote",
          };
        }
        return {
          kind: "local",
          base_url: target.base_url,
          token: "local-token",
        };
      }
      if (name === "desktop_restart_local_daemon") {
        const target = currentDaemonTarget();
        return {
          kind: "local",
          base_url: target.base_url,
          token: "local-token",
        };
      }
      if (name === "desktop_update_remote_daemon") {
        return {
          updated: true,
          message: "Remote daemon updated on channel stable and restarted.",
        };
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
        const path = String(req.path ?? "");

        if (path === "/api/health") {
          return {
            status: 200,
            content_type: "application/json",
            body: JSON.stringify(mkHealth()),
          };
        }
        if (path === "/api/workspaces" || path === "/api/providers") {
          return { status: 200, content_type: "application/json", body: "[]" };
        }
        if (path.startsWith("/api/updates/check")) {
          return {
            status: 200,
            content_type: "application/json",
            body: JSON.stringify({
              channel: "stable",
              latest_version: state.config.desktopVersion,
              current_version: state.config.desktopVersion,
              update_available: false,
            }),
          };
        }
        if (path === "/api/telemetry/client") {
          return { status: 204, content_type: "application/json", body: "" };
        }
        return { status: 200, content_type: "application/json", body: "{}" };
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

    w.__ctxUpdaterE2E = state;
    window.confirm = () => Boolean(w.__ctxUpdaterE2E?.config.confirmResult);
  }, config);
};

const setConfirmResult = async (page: Page, confirmResult: boolean) => {
  await page.evaluate((next: boolean) => {
    const w = window as Window & { __ctxUpdaterE2E?: HarnessState };
    if (!w.__ctxUpdaterE2E) throw new Error("missing updater E2E harness");
    w.__ctxUpdaterE2E.config.confirmResult = next;
  }, confirmResult);
};

const commandCallCount = async (page: Page, command: string): Promise<number> => {
  return await page.evaluate((name: string) => {
    const w = window as Window & { __ctxUpdaterE2E?: HarnessState };
    const calls = w.__ctxUpdaterE2E?.invokeCalls ?? [];
    return calls.filter((cmd) => cmd === name).length;
  }, command);
};

test("local mismatch requires confirm before restart action", async ({ page }) => {
  await installDesktopHarness(page, {
    connectionKind: "local",
    desktopVersion: "2.0.0",
    daemonVersion: "1.0.0",
    confirmResult: false,
  });

  await page.goto("/workspaces", { waitUntil: "domcontentloaded" });

  await expect(page.getByRole("heading", { name: "Desktop and daemon are out of sync" })).toBeVisible();
  await expect(page.getByText(/local daemon is older than this desktop app/i)).toBeVisible();

  const initialCalls = await commandCallCount(page, "desktop_restart_local_daemon");
  await page.getByRole("button", { name: "Restart local daemon" }).click();
  await expect.poll(async () => commandCallCount(page, "desktop_restart_local_daemon")).toBe(initialCalls);

  await setConfirmResult(page, true);
  await page.getByRole("button", { name: "Restart local daemon" }).click();
  await expect
    .poll(async () => commandCallCount(page, "desktop_restart_local_daemon"))
    .toBeGreaterThanOrEqual(initialCalls + 1);
});

test("remote mismatch updates immediately when no active tasks are detected", async ({ page }) => {
  await installDesktopHarness(page, {
    connectionKind: "ssh",
    desktopVersion: "2.0.0",
    daemonVersion: "1.0.0",
    confirmResult: false,
  });

  await page.goto("/workspaces", { waitUntil: "domcontentloaded" });

  await expect(page.getByRole("heading", { name: "Desktop and daemon are out of sync" })).toBeVisible();
  await expect(page.getByText(/remote daemon is older than this desktop app/i)).toBeVisible();

  const initialCalls = await commandCallCount(page, "desktop_update_remote_daemon");
  await page.getByRole("button", { name: "Update remote daemon" }).click();
  await expect
    .poll(async () => commandCallCount(page, "desktop_update_remote_daemon"))
    .toBeGreaterThanOrEqual(initialCalls + 1);
});
