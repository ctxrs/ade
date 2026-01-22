import { test, expect } from "./utils/fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";

test("workbench: recovers when workspace stream drops once", async ({ page }) => {
  // Simulate a flaky network where the workspace stream WS is dropped once.
  // The client should reconnect and keep the conversation moving.
  await page.addInitScript(() => {
    const OriginalWebSocket = window.WebSocket as any;
    (window as any).__contextTestClosedStreamOnce ??= false;

    class FlakyStreamWebSocket extends OriginalWebSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        // @ts-expect-error - runtime shim
        super(url, protocols);
        const u = String(url ?? "");
        if ((window as any).__contextTestClosedStreamOnce) return;
        if (!u.includes("/api/workspaces/") || !u.includes("/active_snapshot/stream")) return;

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

  await newComposer.locator("textarea.wb-composer-textarea").fill("hello 1");
  await newComposer.locator("button[aria-label=\"Send\"]").click();
  const url = new URL(page.url());
  const workspaceId = url.pathname.split("/").filter(Boolean).pop();
  expect(workspaceId).toBeTruthy();

  const readId = (v: any): string => {
    if (!v) return "";
    if (typeof v === "string") return v;
    if (typeof v === "object" && typeof v["0"] === "string") return v["0"];
    return "";
  };

  let sessionId = "";
  await expect
    .poll(
      async () => {
        const resp = await page.request.get(`/api/workspaces/${workspaceId}/active_snapshot`);
        if (!resp.ok()) return "";
        const snapshot = (await resp.json()) as any;
        const taskSummary = snapshot?.active?.tasks?.[0];
        const primarySessionId = readId(taskSummary?.task?.primary_session_id);
        const sessionSummary = taskSummary?.sessions?.[taskSummary?.sessions?.length - 1];
        sessionId = readId(sessionSummary?.session?.id) || primarySessionId;
        return sessionId;
      },
      { timeout: 20000 },
    )
    .not.toBe("");

  await expect
    .poll(
      async () => {
        const resp = await page.request.get(`/api/sessions/${sessionId}/snapshot?limit=50`);
        if (!resp.ok()) return 0;
        const snapshot = (await resp.json()) as any;
        const msgs = snapshot?.head?.messages ?? [];
        return msgs.filter((m: any) => m.role === "assistant").length;
      },
      { timeout: 20000 },
    )
    .toBeGreaterThan(0);

  const activeSession = page.locator(".wb-session-slot[aria-hidden=\"false\"]");
  const sessionComposer = activeSession.locator("textarea.wb-active-textarea");
  await expect(sessionComposer).toBeVisible({ timeout: 20000 });
  await sessionComposer.fill("hello 2");
  await activeSession.locator("button[aria-label=\"Send\"]").click();
  await expect
    .poll(
      async () => {
        const resp = await page.request.get(`/api/sessions/${sessionId}/snapshot?limit=50`);
        if (!resp.ok()) return 0;
        const snapshot = (await resp.json()) as any;
        const msgs = snapshot?.head?.messages ?? [];
        return msgs.filter((m: any) => m.role === "assistant").length;
      },
      { timeout: 20000 },
    )
    .toBeGreaterThanOrEqual(2);
});
