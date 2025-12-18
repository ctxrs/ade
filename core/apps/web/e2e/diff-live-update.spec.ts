import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";

async function createWorkspaceAndStartRun(opts: {
  page: any;
  request: any;
  repo: string;
  workspaceName: string;
  taskTitle: string;
  prompt: string;
}) {
  const { page, repo, workspaceName, taskTitle, prompt, request } = opts;

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

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(prompt);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  const sessionComposer = page.locator(".wb-session textarea.wb-active-textarea");
  await expect(sessionComposer).toBeVisible({ timeout: 20_000 });

  const readId = (v: any): string => {
    if (!v) return "";
    if (typeof v === "string") return v;
    if (typeof v === "object" && typeof v["0"] === "string") return v["0"];
    return "";
  };

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

  const tracksResp = await request.get(`/api/tasks/${taskId}/tracks`);
  expect(tracksResp.ok()).toBeTruthy();
  const tracks = (await tracksResp.json()) as any[];
  const trackId = readId(tracks[0]?.id);
  expect(trackId).toBeTruthy();

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

  const sessionResp = await request.get(`/api/sessions/${sessionId}`);
  expect(sessionResp.ok()).toBeTruthy();
  const session = (await sessionResp.json()) as any;

  const worktreeId = readId(session?.worktree_id);
  expect(worktreeId).toBeTruthy();
  const wtResp = await request.get(`/api/worktrees/${worktreeId}`);
  expect(wtResp.ok()).toBeTruthy();
  const wt = (await wtResp.json()) as any;
  const worktreeRoot = String(wt?.root_path ?? "");
  expect(worktreeRoot).toBeTruthy();

  return { sessionId, trackId, worktreeRoot };
}

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

  const { worktreeRoot } = await createWorkspaceAndStartRun({
    page,
    request,
    repo,
    workspaceName,
    taskTitle,
    prompt: taskTitle,
  });

  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nchanged while running\n");

  // Diff panel should appear and show pending changes before the run completes.
  await expect(page.locator(".wb-diff-pill")).toHaveText(/Pending Change/, { timeout: 10_000 });
  await expect(page.locator(".diff-pane")).toContainText("file.txt", { timeout: 10_000 });
});

test("workbench: diff updates for manual edits while idle", async ({ page, request }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "context-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;
  const taskTitle = "idle-diff-test";
  const { sessionId, worktreeRoot } = await createWorkspaceAndStartRun({
    page,
    request,
    repo,
    workspaceName,
    taskTitle,
    prompt: taskTitle,
  });

  // Wait until the initial turn finishes so we're simulating "manual edits while idle" (no new events).
  await expect
    .poll(
      async () => {
        const evsResp = await request.get(`/api/sessions/${sessionId}/events`);
        if (!evsResp.ok()) return false;
        const evs = (await evsResp.json()) as any[];
        const lastType = evs.length > 0 ? String(evs[evs.length - 1]?.event_type ?? "") : "";
        return lastType === "done";
      },
      { timeout: 20_000 }
    )
    .toBe(true);

  // Simulate user editing the worktree outside the agent.
  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nchanged while idle\n");

  // Diff should appear without sending another message.
  await expect(page.locator(".wb-diff-pill")).toHaveText(/Pending Change/, { timeout: 10_000 });
  await expect(page.locator(".diff-pane")).toContainText("file.txt", { timeout: 10_000 });
});
