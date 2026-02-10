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

  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: "Fake" }).click();

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(prompt);
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  const sessionComposer = page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea");
  await expect(sessionComposer).toBeVisible({ timeout: 20_000 });

  const readId = (v: any): string => (typeof v === "string" ? v : "");

  let sessionId = "";
  await expect
    .poll(
      async () => {
        const resp = await request.get(`/api/workspaces/${workspaceId}/active_snapshot`);
        if (!resp.ok()) return "";
        const snapshot = (await resp.json()) as any;
        const taskSummary = snapshot?.active?.tasks?.[0];
        if (!taskSummary) return "";
        const sessionSummary = taskSummary?.sessions?.[taskSummary?.sessions?.length - 1];
        const primarySessionId = readId(taskSummary?.task?.primary_session_id);
        sessionId = readId(sessionSummary?.session?.id) || primarySessionId;
        return sessionId;
      },
      { timeout: 20_000 },
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

  return { sessionId, worktreeRoot, workspaceId, worktreeId };
}

async function waitForWorktreeSummary(opts: {
  request: any;
  workspaceId: string;
  worktreeId: string;
  timeoutMs?: number;
}) {
  const { request, workspaceId, worktreeId, timeoutMs = 20_000 } = opts;
  await expect
    .poll(
      async () => {
        const resp = await request.get(`/api/workspaces/${workspaceId}/active_snapshot?limit=5`);
        if (!resp.ok()) return null;
        const snapshot = (await resp.json()) as any;
        const entry = snapshot?.worktree_vcs_snapshots?.find((item: any) => item?.worktree_id === worktreeId);
        if (!entry) return null;
        if (entry?.compute_state !== "ready") return null;
        const fileCount = entry?.summary?.file_count ?? null;
        const lineCount = entry?.summary?.line_count ?? null;
        if (fileCount === null && lineCount === null) return null;
        return Number(fileCount ?? lineCount);
      },
      { timeout: timeoutMs },
    )
    .toBeGreaterThan(0);
}

test("workbench: diff fetch failure does not render false no-changes state", async ({ page, request }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init -b main", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const { worktreeRoot, workspaceId, worktreeId } = await createWorkspaceAndStartRun({
    page,
    request,
    repo,
    workspaceName: `ws-${Date.now()}`,
    prompt: "diff fetch failure",
  });

  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nphase1\n");
  await waitForWorktreeSummary({ request, workspaceId, worktreeId });

  await page.route(/\/api\/sessions\/[^/]+\/diff$/, async (route) => {
    await route.fulfill({
      status: 500,
      contentType: "application/json",
      body: JSON.stringify({ error: "simulated diff fetch failure" }),
    });
  });

  const diffButton = page.getByRole("button", { name: "Toggle diff view" });
  const diffBadge = diffButton.locator(".wb-icon-badge");
  await expect(diffBadge).toHaveText("1", { timeout: 20_000 });
  await diffButton.click();

  const diffPane = page.locator(".wb-right-pane.wb-diff");
  await expect(diffPane).toBeVisible({ timeout: 10_000 });
  await expect(diffPane).toContainText("Failed to load diff content", { timeout: 20_000 });
  await expect(diffPane.getByText("No changes.")).toHaveCount(0);
  await expect(diffPane.getByText("No changes on this worktree.")).toHaveCount(0);
});
