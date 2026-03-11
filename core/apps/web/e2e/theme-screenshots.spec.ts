import { argosScreenshot } from "@argos-ci/playwright";
import { test, expect } from "./fixtures";
import type { Page } from "playwright/test";
import os from "os";
import path from "path";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

type ThemeMode = "dark" | "light";

const THEME_STORAGE_KEY = "ctx.theme.mode";

async function setTheme(page: Page, mode: ThemeMode) {
  await page.addInitScript((payload: { theme: ThemeMode; storageKey: string }) => {
    window.localStorage.setItem(payload.storageKey, payload.theme);
    document.documentElement.setAttribute("data-theme", payload.theme);
  }, { theme: mode, storageKey: THEME_STORAGE_KEY });
}

async function openSettings(page: Page, workspaceId: string, mode: ThemeMode) {
  await page.goto(`/settings?ws=${workspaceId}`, { waitUntil: "domcontentloaded" });
  await page.evaluate(
    (payload: { theme: ThemeMode; storageKey: string }) => {
      window.localStorage.setItem(payload.storageKey, payload.theme);
      document.documentElement.setAttribute("data-theme", payload.theme);
    },
    { theme: mode, storageKey: THEME_STORAGE_KEY },
  );
  await expect(page.locator("html")).toHaveAttribute("data-theme", mode);
  await expect(page.getByText("Theme", { exact: true })).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(1000);
}

test.describe.serial("theme screenshots", () => {
  let workspaceId = "";
  const argosRoot = path.join(process.env.CTX_E2E_DATA_DIR ?? os.tmpdir(), "argos-screenshots");

  test.beforeAll(async ({ request }) => {
    const seed = await seedDummyWorkspace(request, {
      tasks: 0,
      sessionsPerTask: 0,
      turnsPerSession: 0,
    });
    workspaceId = seed.workspaceId;
  });

  test("dark theme", async ({ page }) => {
    await setTheme(page, "dark");
    await openSettings(page, workspaceId, "dark");
    await argosScreenshot(page, "settings-theme-dark", {
      root: argosRoot,
      fullPage: true,
    });
  });

  test("light theme", async ({ page }) => {
    await setTheme(page, "light");
    await openSettings(page, workspaceId, "light");
    await argosScreenshot(page, "settings-theme-light", {
      root: argosRoot,
      fullPage: true,
    });
  });
});
