import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";

test("workbench: deep links + task switching never desync selection", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "context-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;

  await page.goto("/");
  await page.getByLabel("Root path").fill(repo);
  await page.getByLabel("Name (optional)").fill(workspaceName);
  await page.getByRole("button", { name: "Add workspace" }).click();
  await page
    .getByRole("listitem")
    .filter({ hasText: repo })
    .getByRole("link", { name: workspaceName })
    .click();

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
  await expect(page).toHaveURL(/[\?&]task=/, { timeout: 20000 });
  await expect(page).toHaveURL(/[\?&]track=/, { timeout: 60000 });
  await expect(page).toHaveURL(/[\?&]session=/, { timeout: 60000 });
  const url1 = page.url();
  const sel1 = new URL(url1).searchParams;
  const task1 = sel1.get("task");
  const track1 = sel1.get("track");
  const session1 = sel1.get("session");
  expect(task1).toBeTruthy();
  expect(track1).toBeTruthy();
  expect(session1).toBeTruthy();

  // New Task must clear selection and stay cleared (no snap-back).
  await page.getByRole("button", { name: "New Task" }).click();
  await expect(page).not.toHaveURL(/[\?&]task=/, { timeout: 20000 });
  await expect(page.locator(".wb-new-composer-stack textarea.wb-composer-textarea")).toBeVisible({ timeout: 20000 });
  await page.waitForTimeout(500);
  await expect(page).not.toHaveURL(/[\?&]task=/);

  const msg2 = `task two marker ${Date.now()}`;
  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(msg2);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session .wb-assistant-entry").filter({ hasText: `done: ${msg2}` })).toBeVisible({ timeout: 20000 });
  await expect(page).toHaveURL(/[\?&]task=/, { timeout: 20000 });
  const url2 = page.url();
  const task2 = new URL(url2).searchParams.get("task");
  expect(task2).toBeTruthy();
  expect(task2).not.toBe(task1);

  // Switching tasks must keep sidebar + conversation pane aligned.
  await page.locator(".wb-task-row").filter({ hasText: msg1 }).first().click();
  await expect(page).toHaveURL(new RegExp(`[\\?&]task=${task1}`), { timeout: 20000 });
  await expect(page.locator(".wb-session")).toContainText(`done: ${msg1}`, { timeout: 20000 });

  await page.locator(".wb-task-row").filter({ hasText: msg2 }).first().click();
  await expect(page).toHaveURL(new RegExp(`[\\?&]task=${task2}`), { timeout: 20000 });
  await expect(page.locator(".wb-session")).toContainText(`done: ${msg2}`, { timeout: 20000 });

  // Deep link must render the session immediately (no empty-state flash).
  await page.goto(url1);
  const emptyState = page.locator(".wb-session .wb-muted").filter({ hasText: "Select a track with a session." });
  await expect(emptyState).toHaveCount(0);
  await expect(page.locator(".wb-session")).toContainText(`done: ${msg1}`, { timeout: 20000 });
});
