import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("workbench: second message gets a response", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;

  await createWorkspaceAndOpenWorkbench({ page, request: page.request, repo, workspaceName });

  // Choose Fake harness so the test doesn't depend on external agents.
  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill("hello 1");
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session .wb-assistant-entry")).toHaveCount(1, { timeout: 20000 });

  const sessionComposer = page.locator(".wb-session textarea.wb-active-textarea");
  await expect(sessionComposer).toBeVisible({ timeout: 20000 });
  await sessionComposer.fill("hello 2");
  await expect(page.locator(".wb-session button[aria-label=\"Send\"]")).toBeEnabled({ timeout: 20000 });
  await page.locator(".wb-session button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session .wb-assistant-entry")).toHaveCount(2, { timeout: 20000 });
});
