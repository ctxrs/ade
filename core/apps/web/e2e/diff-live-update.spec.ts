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
  prompt: string;
}) {
  const { page, repo, workspaceName, prompt, request } = opts;

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
  let trackId = "";
  let sessionId = "";
  await expect
    .poll(
      async () => {
        const resp = await request.get(`/api/workspaces/${workspaceId}/catchup`);
        if (!resp.ok()) return "";
        const snapshot = (await resp.json()) as any;
        const taskSummary = snapshot?.active?.tasks?.[0];
        if (!taskSummary) return "";
        taskId = readId(taskSummary?.task?.id);
        const trackSummary = taskSummary?.tracks?.[0];
        if (!trackSummary) return "";
        trackId = readId(trackSummary?.track?.id);
        const primarySessionId = readId(trackSummary?.primary_session_id);
        const sessionSummary = trackSummary?.sessions?.[trackSummary?.sessions?.length - 1];
        sessionId = readId(sessionSummary?.session?.id) || primarySessionId;
        return sessionId;
      },
      { timeout: 20_000 }
    )
    .not.toBe("");

  const headResp = await request.get(`/api/sessions/${sessionId}/head?limit=1`);
  expect(headResp.ok()).toBeTruthy();
  const head = (await headResp.json()) as any;
  const session = head?.session ?? null;
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
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;
  const taskTitle = "slow-diff-test";

  const { worktreeRoot, trackId } = await createWorkspaceAndStartRun({
    page,
    request,
    repo,
    workspaceName,
    prompt: taskTitle,
  });

  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nchanged while running\n");

  const diffResp = await request.get(`/api/tracks/${trackId}/diff`);
  expect(diffResp.ok()).toBeTruthy();
  const diff = await diffResp.json();
  expect(String(diff?.diff ?? "")).toContain("file.txt");
});

test("workbench: diff updates for manual edits while idle", async ({ page, request }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;
  const taskTitle = "idle-diff-test";
  const { sessionId, worktreeRoot, trackId } = await createWorkspaceAndStartRun({
    page,
    request,
    repo,
    workspaceName,
    prompt: taskTitle,
  });

  await expect(page.locator(".wb-session .wb-assistant-entry").filter({ hasText: `done: ${taskTitle}` })).toBeVisible({
    timeout: 20_000,
  });

  // Simulate user editing the worktree outside the agent.
  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nchanged while idle\n");

  const diffResp = await request.get(`/api/tracks/${trackId}/diff`);
  expect(diffResp.ok()).toBeTruthy();
  const diff = await diffResp.json();
  expect(String(diff?.diff ?? "")).toContain("file.txt");
});
