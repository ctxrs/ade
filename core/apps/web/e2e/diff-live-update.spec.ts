import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";

test("workbench: diff updates mid-turn", async ({ page, request }) => {
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
  await page.locator(".wb-harness-menu").getByRole("button", { name: "Fake" }).click();

  // Start a slow run so we can mutate the worktree while it is active.
  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill("slow-diff-test");
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  const sessionComposer = page.locator(".wb-session textarea.wb-active-textarea");
  await expect(sessionComposer).toBeVisible({ timeout: 20_000 });

  // Discover worktree root path via the daemon API and modify a file while the run is still active.
  const sessionId = new URL(page.url()).searchParams.get("session");
  expect(sessionId, "workbench URL includes session id").toBeTruthy();
  const sessionResp = await request.get(`/api/sessions/${sessionId}`);
  expect(sessionResp.ok()).toBeTruthy();
  const session = (await sessionResp.json()) as any;

  const readId = (v: any): string => {
    if (!v) return "";
    if (typeof v === "string") return v;
    if (typeof v === "object" && typeof v["0"] === "string") return v["0"];
    return "";
  };

  const worktreeId = readId(session?.worktree_id);
  expect(worktreeId, "session worktree_id available").toBeTruthy();

  const wtResp = await request.get(`/api/worktrees/${worktreeId}`);
  expect(wtResp.ok()).toBeTruthy();
  const wt = (await wtResp.json()) as any;
  const worktreeRoot = String(wt?.root_path ?? "");
  expect(worktreeRoot, "worktree root_path available").toBeTruthy();

  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nchanged while running\n");

  // Diff panel should appear and show pending changes before the run completes.
  await expect(page.locator(".wb-diff-pill")).toHaveText(/Pending Change/, { timeout: 10_000 });
  await expect(page.locator(".diff-pane")).toContainText("file.txt", { timeout: 10_000 });
});
