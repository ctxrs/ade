import { test, expect } from "./utils/fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("workbench: task switching never desyncs selection (no URL state)", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;

  await createWorkspaceAndOpenWorkbench({ page, request: page.request, repo, workspaceName });

  const newComposer = page.locator(".wb-new-composer-stack");
  await expect(newComposer).toBeVisible({ timeout: 20000 });

  // Choose Fake harness so the test doesn't depend on external agents.
  await newComposer.getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
  await expect(newComposer.locator("button[title=\"Harness\"] .wb-switcher-label")).toHaveText(/fake/i, {
    timeout: 20000,
  });

  const msg1 = `task one marker ${Date.now()}`;
  await newComposer.locator("textarea.wb-composer-textarea").fill(msg1);
  await newComposer.locator("button[aria-label=\"Send\"]").click();
  const activeSession = page.locator(".wb-session-slot[aria-hidden=\"false\"]");
  const msg1Entry = activeSession.locator(".wb-assistant-entry").filter({ hasText: `done: ${msg1}` }).first();
  await expect(msg1Entry).toBeVisible({ timeout: 20000 });
  const url1 = new URL(page.url());
  expect(url1.searchParams.get("task")).toBeNull();
  expect(url1.searchParams.get("track")).toBeNull();
  expect(url1.searchParams.get("session")).toBeNull();

  // New Task must clear selection and stay cleared (no snap-back).
  await page.getByRole("button", { name: "New Task" }).click();
  await expect(newComposer.locator("textarea.wb-composer-textarea")).toBeVisible({ timeout: 20000 });
  await expect(page.locator(".wb-task-row.wb-task-row-active")).toHaveCount(0, { timeout: 20000 });
  const urlAfterNew = new URL(page.url());
  expect(urlAfterNew.searchParams.get("task")).toBeNull();
  expect(urlAfterNew.searchParams.get("track")).toBeNull();
  expect(urlAfterNew.searchParams.get("session")).toBeNull();

  const msg2 = `task two marker ${Date.now()}`;
  await newComposer.locator("textarea.wb-composer-textarea").fill(msg2);
  await newComposer.locator("button[aria-label=\"Send\"]").click();
  const msg2Entry = activeSession.locator(".wb-assistant-entry").filter({ hasText: `done: ${msg2}` }).first();
  await expect(msg2Entry).toBeVisible({ timeout: 20000 });
  const url2 = new URL(page.url());
  expect(url2.searchParams.get("task")).toBeNull();
  expect(url2.searchParams.get("track")).toBeNull();
  expect(url2.searchParams.get("session")).toBeNull();

  // Switching tasks must keep sidebar + conversation pane aligned.
  const taskRows = page.locator(".wb-task-row");
  await expect(taskRows).toHaveCount(2, { timeout: 20000 });
  const olderTaskRow = page.locator(".wb-task-row", { hasText: msg1 });
  const newestTaskRow = page.locator(".wb-task-row", { hasText: msg2 });
  await expect(olderTaskRow).toBeVisible({ timeout: 20000 });
  await expect(newestTaskRow).toBeVisible({ timeout: 20000 });

  await olderTaskRow.click();
  await expect(olderTaskRow).toHaveClass(/wb-task-row-active/, { timeout: 20000 });
  await expect(msg1Entry).toBeVisible({ timeout: 20000 });

  await newestTaskRow.click();
  await expect(newestTaskRow).toHaveClass(/wb-task-row-active/, { timeout: 20000 });
  await expect(msg2Entry).toBeVisible({ timeout: 20000 });

  // Refresh should restore the same selection from IndexedDB (window-scoped).
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(msg2Entry).toBeVisible({ timeout: 20000 });
  const urlAfterReload = new URL(page.url());
  expect(urlAfterReload.searchParams.get("task")).toBeNull();
  expect(urlAfterReload.searchParams.get("track")).toBeNull();
  expect(urlAfterReload.searchParams.get("session")).toBeNull();
});
