import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__CTX_FEATURE_FLAGS__ = {
      queued_messages_enabled: true,
    };
  });
});

const setupRunningSession = async (page: any) => {
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

  const slowMessage = `slow-diff-test
[[tool_calls]]
[
  {"kind":"execute","title":"t1","input":{"command":"echo 1"}},
  {"kind":"execute","title":"t2","input":{"command":"echo 2"}},
  {"kind":"execute","title":"t3","input":{"command":"echo 3"}},
  {"kind":"execute","title":"t4","input":{"command":"echo 4"}}
]
[[/tool_calls]]`;

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(slowMessage);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  await expect(page.locator(".wb-session textarea.wb-active-textarea")).toBeVisible({ timeout: 20_000 });
  await expect(page.locator(".wb-session button[aria-label=\"Stop\"]")).toBeVisible({ timeout: 20_000 });
};

const queueMessage = async (page: any, text: string) => {
  const sessionComposer = page.locator(".wb-session textarea.wb-active-textarea");
  await sessionComposer.fill(text);
  await page.locator(".wb-session button[aria-label=\"Send\"]").click();
};

const delayDeleteMessage = async (page: any, delayMs: number) => {
  await page.route("**/api/messages/*", async (route) => {
    const req = route.request();
    if (req.method() !== "DELETE") return route.continue();
    await new Promise((resolve) => setTimeout(resolve, delayMs));
    return route.continue();
  });
};

test("workbench: queued sends do not flash optimistic turn and queue panel clears when started", async ({ page }) => {
  await setupRunningSession(page);

  const queuedText = `queued-msg-${Date.now()}`;

  await page.route("**/api/sessions/*/messages", async (route) => {
    const req = route.request();
    if (req.method() !== "POST") return route.continue();
    const body = req.postData() ?? "";
    if (body.includes(queuedText)) {
      await new Promise((resolve) => setTimeout(resolve, 1500));
    }
    return route.continue();
  });

  await queueMessage(page, queuedText);

  // Queued sends should not appear as an optimistic/pending turn before the server acknowledges them.
  await expect(page.locator(".wb-session")).not.toContainText(queuedText, { timeout: 1200 });

  const queuePanel = page.locator(".wb-session .queue-panel");
  await expect(queuePanel).toBeVisible({ timeout: 20_000 });
  await expect(queuePanel).toContainText(queuedText, { timeout: 20_000 });

  // When the queued message starts running, it should be removed from the queue panel.
  await expect(queuePanel).toHaveCount(0, { timeout: 60_000 });
  await expect(page.locator(".wb-session")).toContainText(queuedText, { timeout: 20_000 });
});

test("workbench: edit queued message hides queue panel immediately", async ({ page }) => {
  await setupRunningSession(page);

  const queuedText = `queued-edit-${Date.now()}`;
  await queueMessage(page, queuedText);

  const queuePanel = page.locator(".wb-session .queue-panel");
  await expect(queuePanel).toBeVisible({ timeout: 20_000 });
  await expect(queuePanel).toContainText(queuedText, { timeout: 20_000 });

  await delayDeleteMessage(page, 1500);

  await queuePanel.getByRole("button", { name: "Edit queued message" }).click();

  await expect(page.locator(".wb-session textarea.wb-active-textarea")).toHaveValue(queuedText);
  await expect(queuePanel).toHaveCount(0, { timeout: 800 });
});

test("workbench: trash queued message hides queue panel immediately", async ({ page }) => {
  await setupRunningSession(page);

  const queuedText = `queued-trash-${Date.now()}`;
  await queueMessage(page, queuedText);

  const queuePanel = page.locator(".wb-session .queue-panel");
  await expect(queuePanel).toBeVisible({ timeout: 20_000 });
  await expect(queuePanel).toContainText(queuedText, { timeout: 20_000 });

  await delayDeleteMessage(page, 1500);

  await queuePanel.getByRole("button", { name: "Cancel queued message" }).click();
  await expect(queuePanel).toHaveCount(0, { timeout: 800 });
});

test("workbench: queued list updates when removing the first item", async ({ page }) => {
  await setupRunningSession(page);

  const firstQueued = `queued-first-${Date.now()}`;
  const secondQueued = `queued-second-${Date.now()}`;
  await queueMessage(page, firstQueued);
  const queuePanel = page.locator(".wb-session .queue-panel");
  await expect(queuePanel).toContainText(firstQueued, { timeout: 20_000 });

  await queueMessage(page, secondQueued);
  await expect(queuePanel).toContainText(secondQueued, { timeout: 20_000 });

  await queuePanel.getByRole("button", { name: "Cancel queued message" }).first().click();

  await expect(queuePanel).toContainText(secondQueued, { timeout: 20_000 });
  await expect(queuePanel).not.toContainText(firstQueued, { timeout: 20_000 });
  await expect(queuePanel.getByRole("button", { name: "Send now" })).toBeVisible({ timeout: 20_000 });
});

test("workbench: send now removes queued message immediately", async ({ page }) => {
  await setupRunningSession(page);

  const queuedText = `queued-send-now-${Date.now()}`;
  await queueMessage(page, queuedText);

  const queuePanel = page.locator(".wb-session .queue-panel");
  await expect(queuePanel).toBeVisible({ timeout: 20_000 });
  await expect(queuePanel).toContainText(queuedText, { timeout: 20_000 });

  await delayDeleteMessage(page, 1500);

  await queuePanel.getByRole("button", { name: "Send now" }).click();
  await expect(queuePanel).toHaveCount(0, { timeout: 800 });
});
