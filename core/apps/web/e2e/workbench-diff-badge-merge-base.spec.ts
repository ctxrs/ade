import { test, expect } from "./fixtures";
import { mkdtempSync, realpathSync, writeFileSync } from "fs";
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
  const resolvePath = (value: string) => {
    try {
      return typeof realpathSync.native === "function" ? realpathSync.native(value) : realpathSync(value);
    } catch {
      return path.resolve(value);
    }
  };
  const repoPath = resolvePath(repo);

  await page.goto("/");
  await page.getByLabel("Root path").fill(repo);
  await page.getByLabel("Name (optional)").fill(workspaceName);
  await page.getByRole("button", { name: "Add workspace" }).click();
  await page
    .getByRole("listitem")
    .filter({ hasText: repo })
    .getByRole("link", { name: workspaceName })
    .click();

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
        const ws = workspaces.find((w) => resolvePath(String(w?.root_path ?? "")) === repoPath);
        workspaceId = readId(ws?.id);
        return workspaceId;
      },
      { timeout: 20_000 },
    )
    .not.toBe("");

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

  return { sessionId, worktreeRoot };
}

test("workbench: merge-base diff badge + pane stay consistent across phases", async ({ page, request }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init -b main", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });
  execSync("git branch merge-target", { cwd: repo });
  execSync("mkdir -p .ctx", { cwd: repo });
  writeFileSync(path.join(repo, ".ctx", "config.toml"), `[merge_queue]\ntarget_branch = "merge-target"\n`);

  const workspaceName = `ws-${Date.now()}`;
  const taskTitle = "merge-base-diff-test";

  const { worktreeRoot } = await createWorkspaceAndStartRun({
    page,
    request,
    repo,
    workspaceName,
    prompt: taskTitle,
  });

  const diffButton = page.getByRole("button", { name: "Toggle diff view" });
  const diffBadge = diffButton.locator(".wb-icon-badge");

  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nphase1\n");

  await diffButton.click();
  await expect(page.locator(".wb-right-pane.wb-diff")).toBeVisible({ timeout: 10_000 });
  await expect(page.locator(".cursor-diff-file-header").filter({ hasText: "file.txt" })).toBeVisible({
    timeout: 20_000,
  });
  await expect(diffBadge).toHaveText("1", { timeout: 20_000 });
  await expect(
    page.locator(".wb-right-pane.wb-diff").getByText("No changes on this worktree."),
  ).toHaveCount(0);

  execSync("git add file.txt", { cwd: worktreeRoot });
  execSync("git commit -m phase1", { cwd: worktreeRoot });
  const phase1Sha = execSync("git rev-parse HEAD", { cwd: worktreeRoot }).toString().trim();
  execSync(`git branch -f merge-target ${phase1Sha}`, { cwd: repo });
  await diffButton.click();
  await expect(page.locator(".wb-right-pane.wb-diff")).toHaveCount(0, { timeout: 10_000 });

  await diffButton.click();
  await expect(page.locator(".wb-right-pane.wb-diff")).toBeVisible({ timeout: 10_000 });
  await expect(diffBadge).toHaveCount(0, { timeout: 20_000 });

  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nphase1\nphase2\n");

  await diffButton.click();
  await expect(page.locator(".wb-right-pane.wb-diff")).toHaveCount(0, { timeout: 10_000 });

  await diffButton.click();
  await expect(page.locator(".wb-right-pane.wb-diff")).toBeVisible({ timeout: 10_000 });
  await expect(page.locator(".cursor-diff-file-header").filter({ hasText: "file.txt" })).toBeVisible({
    timeout: 20_000,
  });
  await expect(diffBadge).toHaveText("1", { timeout: 20_000 });
  await expect(
    page.locator(".wb-right-pane.wb-diff").getByText("No changes on this worktree."),
  ).toHaveCount(0);
});
