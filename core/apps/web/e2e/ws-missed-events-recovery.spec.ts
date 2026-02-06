import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("workbench: recovers when workspace stream drops once", async ({ page }) => {
  // Enable E2E hooks for the workspace stream worker.
  await page.addInitScript(() => {
    window.sessionStorage.setItem("ctxE2E", "1");
  });

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;

  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request: page.request,
    repo,
    workspaceName,
  });

  await expect
    .poll(async () =>
      page.evaluate(() => typeof (window as any).__ctxE2E?.workspaceStream?.getConnectionState === "function"),
    )
    .toBe(true);

  // Choose Fake harness so the test doesn't depend on external agents.
  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
  await expect(
    page.locator(".wb-new-composer-stack button[title=\"Harness\"] .wb-switcher-label"),
  ).toHaveText(/fake/i, { timeout: 20000 });

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill("hello 1");
  await expect(page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]")).toBeEnabled({ timeout: 20000 });
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();
  expect(workspaceId).toBeTruthy();

  await expect(page.locator(".wb-session .wb-assistant-entry")).toHaveCount(1, { timeout: 20000 });

  await expect
    .poll(async () => page.evaluate(() => (window as any).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");

  await page.evaluate(() => {
    (window as any).__ctxE2E?.workspaceStream?.close?.();
  });

  await expect
    .poll(async () => page.evaluate(() => (window as any).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("disconnected");

  await expect
    .poll(async () => page.evaluate(() => (window as any).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");

  const sessionComposer = page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea");
  await expect(sessionComposer).toBeVisible({ timeout: 20000 });
  await sessionComposer.fill("hello 2");
  await page.locator(".wb-session-slot[aria-hidden=\"false\"] button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session .wb-assistant-entry")).toHaveCount(2, { timeout: 20000 });
});
