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
  const taskTitle = "slow-diff-test";

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
  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(taskTitle);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  const sessionComposer = page.locator(".wb-session textarea.wb-active-textarea");
  await expect(sessionComposer).toBeVisible({ timeout: 20_000 });

  const readId = (v: any): string => {
    if (!v) return "";
    if (typeof v === "string") return v;
    if (typeof v === "object" && typeof v["0"] === "string") return v["0"];
    return "";
  };

  // Discover session/worktree info via the daemon API (workbench no longer encodes session in the URL).
  let workspaceId = "";
  await expect
    .poll(
      async () => {
        const workspacesResp = await request.get("/api/workspaces");
        if (!workspacesResp.ok()) return "";
        const workspaces = (await workspacesResp.json()) as any[];
        const ws = workspaces.find((w) => path.resolve(String(w?.root_path ?? "")) === path.resolve(repo));
        workspaceId = readId(ws?.id);
        return workspaceId;
      },
      { timeout: 20_000 }
    )
    .not.toBe("");
  expect(workspaceId, "workspace id available").toBeTruthy();

  let taskId = "";
  await expect
    .poll(
      async () => {
        const resp = await request.get(`/api/workspaces/${workspaceId}/tasks`);
        if (!resp.ok()) return "";
        const tasks = (await resp.json()) as any[];
        const task = tasks.find((t) => String(t?.title ?? "") === taskTitle) ?? null;
        taskId = readId(task?.id);
        return taskId;
      },
      { timeout: 20_000 }
    )
    .not.toBe("");
  expect(taskId, "task id available").toBeTruthy();

  const tracksResp = await request.get(`/api/tasks/${taskId}/tracks`);
  expect(tracksResp.ok()).toBeTruthy();
  const tracks = (await tracksResp.json()) as any[];
  expect(tracks.length, "task has at least one track").toBeGreaterThan(0);
  const trackId = readId(tracks[0]?.id);
  expect(trackId, "track id available").toBeTruthy();

  let sessionId = "";
  await expect
    .poll(
      async () => {
        const sessionsResp = await request.get(`/api/tracks/${trackId}/sessions`);
        if (!sessionsResp.ok()) return "";
        const sessions = (await sessionsResp.json()) as any[];
        sessionId = readId(sessions[sessions.length - 1]?.id);
        return sessionId;
      },
      { timeout: 20_000 }
    )
    .not.toBe("");
  expect(sessionId, "session id available").toBeTruthy();

  const sessionResp = await request.get(`/api/sessions/${sessionId}`);
  expect(sessionResp.ok()).toBeTruthy();
  const session = (await sessionResp.json()) as any;

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
