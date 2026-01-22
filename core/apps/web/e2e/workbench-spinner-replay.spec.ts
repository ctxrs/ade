import { test, expect } from "./utils/fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";

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

  await page.addInitScript(() => {
    const OriginalWebSocket = window.WebSocket as any;
    (window as any).__contextSawToolResult = false;
    (window as any).__contextDropUntil = 0;

    class DropAfterToolResultWebSocket extends OriginalWebSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        // @ts-expect-error - runtime shim
        super(url, protocols);
        const u = String(url ?? "");
        if (!u.includes("/api/workspaces/") || !u.includes("/active_snapshot/stream")) return;
        this.addEventListener("open", () => {
          if (Date.now() >= (window as any).__contextDropUntil) return;
          setTimeout(() => {
            try {
              this.close();
            } catch {
              // ignore
            }
          }, 10);
        });
        this.addEventListener("message", (event) => {
          if ((window as any).__contextSawToolResult) return;
          try {
            const parsed = JSON.parse(String(event.data ?? ""));
            if (parsed?.type !== "session_head_delta") return;
            const eventType = parsed?.delta?.event?.event_type;
            if (eventType !== "tool_result") return;
            (window as any).__contextSawToolResult = true;
            (window as any).__contextDropUntil = Date.now() + 3000;
            setTimeout(() => {
              try {
                this.close();
              } catch {
                // ignore
              }
            }, 10);
          } catch {
            // ignore
          }
        });
      }
    }

    // @ts-expect-error - runtime shim
    window.WebSocket = DropAfterToolResultWebSocket;
  });

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
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

  const newComposer = page.locator(".wb-new-composer-stack");
  await expect(newComposer).toBeVisible({ timeout: 20000 });

  // Choose Fake harness so the test doesn't depend on external agents.
  await newComposer.getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
  await expect(
    newComposer.locator(".wb-switcher-wrap button[title=\"Harness\"] .wb-switcher-label"),
  ).toHaveText(/fake/i, { timeout: 20000 });

  const prompt = "slow-diff-test spinner replay";
  await newComposer.locator("textarea.wb-composer-textarea").fill(prompt);
  await newComposer.locator("button[aria-label=\"Send\"]").click();

  const activeRow = page.locator(".wb-task-row.wb-task-row-active").first();
  await expect(activeRow).toBeVisible({ timeout: 20000 });
  const activeSpinners = activeRow.locator(".wb-task-spinner");
  await expect(activeSpinners.first()).toBeVisible({ timeout: 20000 });
  blockHead = true;

  await page.waitForFunction(() => (window as any).__contextSawToolResult === true, undefined, {
    timeout: 20000,
  });

  await expect(activeSpinners).toHaveCount(0, { timeout: 20000 });
});
