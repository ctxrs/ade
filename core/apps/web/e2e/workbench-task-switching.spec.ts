import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("workbench: task switching never desyncs selection (no URL state)", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "context-e2e-"));
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
  await expect(page.locator(".wb-new-composer-stack button[title=\"Harness\"] .wb-switcher-label")).toHaveText(/fake/i, {
    timeout: 20000,
  });

  // Use Local isolation to keep the test fast and deterministic.
  await page.locator(".wb-new-composer-stack").getByTitle("Isolation").click();
  await page.locator(".wb-exec-menu").getByRole("button", { name: "Local" }).click();
  await expect(page.locator(".wb-new-composer-stack button[title=\"Isolation\"] .wb-switcher-label")).toHaveText(/local/i, {
    timeout: 20000,
  });

  const msg1 = `task one marker ${Date.now()}`;
  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(msg1);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session .wb-assistant-entry").filter({ hasText: `done: ${msg1}` })).toBeVisible({ timeout: 20000 });
  const url1 = new URL(page.url());
  expect(url1.searchParams.get("task")).toBeNull();
  expect(url1.searchParams.get("track")).toBeNull();
  expect(url1.searchParams.get("session")).toBeNull();

  // New Task must clear selection and stay cleared (no snap-back).
  await page.getByRole("button", { name: "New Task" }).click();
  await expect(page.locator(".wb-new-composer-stack textarea.wb-composer-textarea")).toBeVisible({ timeout: 20000 });
  await page.waitForTimeout(500);
  const urlAfterNew = new URL(page.url());
  expect(urlAfterNew.searchParams.get("task")).toBeNull();
  expect(urlAfterNew.searchParams.get("track")).toBeNull();
  expect(urlAfterNew.searchParams.get("session")).toBeNull();

  const msg2 = `task two marker ${Date.now()}`;
  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(msg2);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session .wb-assistant-entry").filter({ hasText: `done: ${msg2}` })).toBeVisible({ timeout: 20000 });
  const url2 = new URL(page.url());
  expect(url2.searchParams.get("task")).toBeNull();
  expect(url2.searchParams.get("track")).toBeNull();
  expect(url2.searchParams.get("session")).toBeNull();

  // Switching tasks must keep sidebar + conversation pane aligned.
  await page.locator(".wb-task-row").filter({ hasText: msg1 }).first().click();
  await expect(page.locator(".wb-session")).toContainText(`done: ${msg1}`, { timeout: 20000 });

  await page.locator(".wb-task-row").filter({ hasText: msg2 }).first().click();
  await expect(page.locator(".wb-session")).toContainText(`done: ${msg2}`, { timeout: 20000 });

  // Refresh should restore the same selection from IndexedDB (window-scoped).
  await page.reload();
  await expect(page.locator(".wb-session")).toContainText(`done: ${msg2}`, { timeout: 20000 });
  const urlAfterReload = new URL(page.url());
  expect(urlAfterReload.searchParams.get("task")).toBeNull();
  expect(urlAfterReload.searchParams.get("track")).toBeNull();
  expect(urlAfterReload.searchParams.get("session")).toBeNull();
});
