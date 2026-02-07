import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("terminal reconnects after websocket drop", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;
  await createWorkspaceAndOpenWorkbench({ page, request: page.request, repo, workspaceName });

  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();

  const prompt = `terminal-reconnect-${Date.now()}`;
  await page
    .locator(".wb-new-composer-stack textarea.wb-composer-textarea")
    .fill(prompt);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea").first()).toBeVisible({
    timeout: 20000,
  });

  const terminalToggle = page.getByRole("button", { name: "Toggle terminal panel" }).first();
  await expect(terminalToggle).toBeVisible({ timeout: 20000 });
  await terminalToggle.click();

  const terminalTabs = page.locator(".wb-terminal-tab");
  if ((await terminalTabs.count()) === 0) {
    await page.getByRole("button", { name: "New terminal" }).click();
  }
  await expect.poll(() => terminalTabs.count()).toBeGreaterThan(0);

  await expect
    .poll(async () => page.evaluate(() => (window as any).__ctxE2ETerminalClients?.size ?? 0))
    .toBeGreaterThan(0);

  await page.evaluate(() => {
    const reg = (window as any).__ctxE2ETerminalClients as Map<string, any> | undefined;
    if (!reg) return;
    for (const handle of reg.values()) {
      try {
        handle.close?.();
      } catch {
        // ignore
      }
    }
  });

  const status = page.locator(".wb-terminal-status").first();
  await expect(status).toContainText(/Reconnecting|Disconnected/, { timeout: 20000 });

  await expect(status).toBeHidden({ timeout: 20000 });
});
