import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";

test("workbench: recovers when WS misses events (polling backfill)", async ({ page }) => {
  // Simulate a flaky network where the global stream WS is dropped once.
  // The client should reconnect and resume via `after_seq` without stalling the conversation.
  await page.addInitScript(() => {
    const OriginalWebSocket = window.WebSocket as any;
    (window as any).__contextTestClosedStreamOnce ??= false;

    class FlakyStreamWebSocket extends OriginalWebSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        // @ts-expect-error - runtime shim
        super(url, protocols);
        const u = String(url ?? "");
        if ((window as any).__contextTestClosedStreamOnce) return;
        if (!u.includes("/api/stream")) return;

        this.addEventListener("open", () => {
          // Close shortly after open to simulate an interrupted WS.
          setTimeout(() => {
            try {
              (window as any).__contextTestClosedStreamOnce = true;
              this.close();
            } catch {
              // ignore
            }
          }, 50);
        });
      }
    }

    // @ts-expect-error - runtime shim
    window.WebSocket = FlakyStreamWebSocket;
  });

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

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill("hello 1");
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session .wb-assistant-entry")).toHaveCount(1, { timeout: 20000 });

  const sessionComposer = page.locator(".wb-session textarea.wb-active-textarea");
  await expect(sessionComposer).toBeVisible({ timeout: 20000 });
  await sessionComposer.fill("hello 2");
  await page.locator(".wb-session button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session .wb-assistant-entry")).toHaveCount(2, { timeout: 20000 });
});
