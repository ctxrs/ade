import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

async function createWorkspaceAndStartRun(opts: {
  page: any;
  request: any;
  repo: string;
  workspaceName: string;
  prompt: string;
}) {
  const { page, repo, workspaceName, prompt, request } = opts;
  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request,
    repo,
    workspaceName,
  });

  // Choose Fake harness so the test doesn't depend on external agents.
  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: "Fake" }).click();

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(prompt);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  const sessionComposer = page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea");
  await expect(sessionComposer).toBeVisible({ timeout: 20_000 });

  const readId = (v: any): string => (typeof v === "string" ? v : "");

  let taskId = "";
  let sessionId = "";
  await expect
    .poll(
      async () => {
        const resp = await request.get(`/api/workspaces/${workspaceId}/active_snapshot`);
        if (!resp.ok()) return "";
        const snapshot = (await resp.json()) as any;
        const taskSummary = snapshot?.active?.tasks?.[0];
        if (!taskSummary) return "";
        taskId = readId(taskSummary?.task?.id);
        const sessionSummary = taskSummary?.sessions?.[taskSummary?.sessions?.length - 1];
        const primarySessionId = readId(taskSummary?.task?.primary_session_id);
        sessionId = readId(sessionSummary?.session?.id) || primarySessionId;
        return sessionId;
      },
      { timeout: 20_000 }
    )
    .not.toBe("");

  const headResp = await request.get(`/api/sessions/${sessionId}/snapshot?limit=1`);
  expect(headResp.ok()).toBeTruthy();
  const snapshot = (await headResp.json()) as any;
  const session = snapshot?.head?.session ?? snapshot?.summary?.session ?? null;
  const worktreeId = readId(session?.worktree_id);
  expect(worktreeId).toBeTruthy();
  const wtResp = await request.get(`/api/worktrees/${worktreeId}`);
  expect(wtResp.ok()).toBeTruthy();
  const wt = (await wtResp.json()) as any;
  const worktreeRoot = String(wt?.root_path ?? "");
  expect(worktreeRoot).toBeTruthy();

  return { sessionId, worktreeRoot };
}

async function waitForSessionCompletion(opts: { request: any; sessionId: string; timeoutMs?: number }) {
  const { request, sessionId, timeoutMs = 30_000 } = opts;
  await expect
    .poll(
      async () => {
        const resp = await request.get(`/api/sessions/${sessionId}/snapshot?include_events=1&limit=60`);
        if (!resp.ok()) return "";
        const data = (await resp.json()) as any;
        const summaryStatus = data?.summary?.activity?.last_turn_status ?? null;
        if (summaryStatus && !["running", "queued"].includes(String(summaryStatus))) return "done";
        const head = data?.head ?? {};
        const headStatus = head?.activity?.last_turn_status ?? null;
        if (headStatus && !["running", "queued"].includes(String(headStatus))) return "done";
        const turns = Array.isArray(head.turns) ? head.turns : [];
        const lastTurn = turns[turns.length - 1];
        const lastStatus = lastTurn?.status ?? null;
        if (lastStatus && !["running", "queued"].includes(String(lastStatus))) return "done";
        return "";
      },
      { timeout: timeoutMs },
    )
    .not.toBe("");
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

  const { worktreeRoot, sessionId } = await createWorkspaceAndStartRun({
    page,
    request,
    repo,
    workspaceName,
    prompt: taskTitle,
  });

  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nchanged while running\n");

  const diffResp = await request.get(`/api/sessions/${sessionId}/diff`);
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
  const { sessionId, worktreeRoot } = await createWorkspaceAndStartRun({
    page,
    request,
    repo,
    workspaceName,
    prompt: taskTitle,
  });

  await waitForSessionCompletion({ request, sessionId });

  // Simulate user editing the worktree outside the agent.
  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nchanged while idle\n");

  const diffResp = await request.get(`/api/sessions/${sessionId}/diff`);
  expect(diffResp.ok()).toBeTruthy();
  const diff = await diffResp.json();
  expect(String(diff?.diff ?? "")).toContain("file.txt");
});
