import { test, expect } from "playwright/test";
import * as fs from "fs";
import path from "path";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const OUT_DIR = process.env.CONTEXT_SETTINGS_SCREENSHOT_DIR ?? "/tmp/context-settings-screens";

test("settings screenshots", async ({ page, request }) => {
  fs.mkdirSync(OUT_DIR, { recursive: true });

  await seedDummyWorkspace(request, { tasks: 1, sessionsPerTask: 0, turnsPerSession: 0, throttleMs: 0 });

  await page.setViewportSize({ width: 1400, height: 820 });

  await page.goto("/settings#general", { waitUntil: "domcontentloaded" });
  await expect(page.locator(".settings-shell")).toBeVisible();
  await page.waitForTimeout(250);
  await page.screenshot({ path: path.join(OUT_DIR, "settings-general.png"), fullPage: true });

  await page.goto("/settings#agent_harnesses", { waitUntil: "domcontentloaded" });
  await expect(page.locator(".settings-shell")).toBeVisible();
  await page.waitForTimeout(350);
  await page.screenshot({ path: path.join(OUT_DIR, "settings-agent-harnesses.png"), fullPage: true });

  await page.goto("/settings#dictation", { waitUntil: "domcontentloaded" });
  await expect(page.locator(".settings-shell")).toBeVisible();
  await page.waitForTimeout(250);
  await page.screenshot({ path: path.join(OUT_DIR, "settings-dictation.png"), fullPage: true });
});
