import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("workbench: queued sends do not flash optimistic turn and queue panel clears when started", async ({ page }) => {
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

  const sessionComposer = page.locator(".wb-session textarea.wb-active-textarea");
  await sessionComposer.fill(queuedText);
  await page.locator(".wb-session button[aria-label=\"Send\"]").click();

  // Queued sends should not appear as an optimistic/pending turn before the server acknowledges them.
  await expect(page.locator(".wb-session")).not.toContainText(queuedText, { timeout: 1200 });

  const queuePanel = page.locator(".wb-session .queue-panel");
  await expect(queuePanel).toBeVisible({ timeout: 20_000 });
  await expect(queuePanel).toContainText(queuedText, { timeout: 20_000 });

  // When the queued message starts running, it should be removed from the queue panel.
  await expect(queuePanel).toHaveCount(0, { timeout: 60_000 });
});

