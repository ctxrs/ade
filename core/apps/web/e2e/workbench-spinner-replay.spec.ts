import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("workbench: spinner clears after replayed completion", async ({ page }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1400, height: 900 });

  let blockHead = false;
  let blockedHeadCount = 0;
  await page.route("**/api/sessions/*/snapshot**", async (route) => {
    if (blockHead && blockedHeadCount < 2) {
      blockedHeadCount += 1;
      await new Promise((resolve) => setTimeout(resolve, 20000));
    }
    await route.continue();
  });

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;

  await createWorkspaceAndOpenWorkbench({
    page,
    request: page.request,
    repo,
    workspaceName,
  });

  // Choose Fake harness so the test doesn't depend on external agents.
  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
  await expect(
    page.locator('.wb-new-composer-stack button[title="Harness"] .wb-switcher-label'),
  ).toHaveText(/fake/i, { timeout: 20000 });

  await expect
    .poll(async () =>
      page.evaluate(() => typeof (window as any).__ctxE2E?.workspaceStream?.getConnectionState === "function"),
    )
    .toBe(true);

  await expect
    .poll(async () => page.evaluate(() => (window as any).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");

  const prompt = "slow-diff-test spinner replay";
  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(prompt);
  await page.locator('.wb-new-composer-stack button[aria-label="Send"]').click();

  const activeSpinners = page.locator(
    ".wb-task-row-active .wb-task-spinner:not(.wb-task-spinner-archive)",
  );
  await expect(activeSpinners.first()).toBeVisible({ timeout: 20000 });
  blockHead = true;

  const sessionSlot = page.locator('.wb-session-slot[aria-hidden="false"]');
  await expect(sessionSlot).toBeVisible({ timeout: 20000 });

  const toolSummary = sessionSlot.getByRole("button", { name: "tool" }).first();
  await expect(toolSummary).toBeVisible({ timeout: 20000 });

  await page.evaluate(() => {
    const stream = (window as any).__ctxE2E?.workspaceStream;
    stream?.setDropMessages?.(true);
    stream?.close?.();
  });

  await page.waitForTimeout(3000);

  await page.evaluate(() => {
    const stream = (window as any).__ctxE2E?.workspaceStream;
    stream?.setDropMessages?.(false);
    stream?.close?.();
  });

  await expect
    .poll(async () => page.evaluate(() => (window as any).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");

  await expect(activeSpinners).toHaveCount(0, { timeout: 12000 });
});
