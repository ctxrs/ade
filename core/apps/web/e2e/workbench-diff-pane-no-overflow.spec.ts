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
  const { page, request, repo, workspaceName, prompt } = opts;

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

  // Choose Fake harness so the test doesn't depend on external agents.
  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: "Fake" }).click();

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(prompt);
  await page.locator('.wb-new-composer-stack button[aria-label="Send"]').click();

  const sessionComposer = page.locator('.wb-session-slot[aria-hidden="false"] textarea.wb-active-textarea');
  await expect(sessionComposer).toBeVisible({ timeout: 20_000 });

  const readId = (v: any): string => (typeof v === "string" ? v : "");

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

  return { workspaceId, worktreeId, worktreeRoot };
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

test("workbench: diff pane never overflows container", async ({ page, request }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init -b main", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  const longPath = path.join(
    repo,
    "a-very-very-very-long-directory-name-to-force-overflow-in-the-right-diff-pane",
    "another-very-very-very-long-directory-name-to-force-overflow-in-the-right-diff-pane",
    "file-with-a-super-long-name-to-force-overflow-in-the-right-diff-pane-because-path-is-long-and-unbreakable.txt",
  );
  execSync(`mkdir -p ${JSON.stringify(path.dirname(longPath))}`, { cwd: repo });
  writeFileSync(longPath, "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  await page.setViewportSize({ width: 1600, height: 900 });

  const workspaceName = `ws-${Date.now()}`;
  const taskTitle = "diff-pane-no-overflow";
  const { workspaceId, worktreeId, worktreeRoot } = await createWorkspaceAndStartRun({
    page,
    request,
    repo,
    workspaceName,
    prompt: taskTitle,
  });

  // Create a change so the diff pane actually renders file rows.
  writeFileSync(path.join(worktreeRoot, "file.txt"), "hello\nchanged\n");
  writeFileSync(
    path.join(
      worktreeRoot,
      "a-very-very-very-long-directory-name-to-force-overflow-in-the-right-diff-pane",
      "another-very-very-very-long-directory-name-to-force-overflow-in-the-right-diff-pane",
      "file-with-a-super-long-name-to-force-overflow-in-the-right-diff-pane-because-path-is-long-and-unbreakable.txt",
    ),
    "hello\nchanged\n",
  );
  await waitForWorktreeSummary({ request, workspaceId, worktreeId });

  const diffButton = page.getByRole("button", { name: "Toggle diff view" });
  await diffButton.click();
  await expect(page.locator(".wb-right-pane.wb-diff")).toBeVisible({ timeout: 20_000 });

  // Wait for at least one diff row to appear.
  await expect(page.locator(".cursor-diff-file-header").first()).toBeVisible({ timeout: 20_000 });

  // Make the right pane as narrow as possible.
  const splitter = page.locator(".wb-splitter");
  const splitterBox = await splitter.boundingBox();
  expect(splitterBox).not.toBeNull();
  await page.mouse.move(splitterBox!.x, splitterBox!.y + 60);
  await page.mouse.down();
  await page.mouse.move(splitterBox!.x + 2000, splitterBox!.y + 60);
  await page.mouse.up();

  // Screenshot should show a clearly narrow right pane.
  await page.screenshot({ path: "/tmp/ctx-diff-pane-right-narrow.png" });

  // Regression assertion: the right diff pane stays within the viewport and
  // the diff rows themselves never paint outside the right pane.
  const overflows = await page.evaluate(() => {
    const pane = document.querySelector(".wb-right") as HTMLElement | null;
    if (!pane) return null;
    const rect = pane.getBoundingClientRect();

    const rowRightEdges = Array.from(
      document.querySelectorAll(".cursor-diff-file-header, .cursor-diff-file-path"),
    ).map((el) => (el as HTMLElement).getBoundingClientRect().right);
    const maxRowRight = rowRightEdges.length > 0 ? Math.max(...rowRightEdges) : null;

    return {
      right: rect.right,
      width: rect.width,
      maxRowRight,
      viewportWidth: window.innerWidth,
      overflowX: document.documentElement.scrollWidth > document.documentElement.clientWidth,
    };
  });

  expect(overflows).not.toBeNull();
  expect(overflows!.right).toBeLessThanOrEqual(overflows!.viewportWidth + 0.5);
  expect(overflows!.overflowX).toBeFalsy();
  expect(overflows!.width).toBeLessThanOrEqual(360);
  expect(overflows!.maxRowRight).not.toBeNull();
  expect(overflows!.maxRowRight!).toBeLessThanOrEqual(overflows!.right + 0.5);

  // Also assert at a narrow overall viewport.
  await page.setViewportSize({ width: 760, height: 900 });
  const overflowsSmallViewport = await page.evaluate(() => {
    const pane = document.querySelector(".wb-right") as HTMLElement | null;
    if (!pane) return null;
    const rect = pane.getBoundingClientRect();

    const rowRightEdges = Array.from(
      document.querySelectorAll(".cursor-diff-file-header, .cursor-diff-file-path"),
    ).map((el) => (el as HTMLElement).getBoundingClientRect().right);
    const maxRowRight = rowRightEdges.length > 0 ? Math.max(...rowRightEdges) : null;

    return {
      right: rect.right,
      maxRowRight,
      viewportWidth: window.innerWidth,
      overflowX: document.documentElement.scrollWidth > document.documentElement.clientWidth,
    };
  });
  expect(overflowsSmallViewport).not.toBeNull();
  expect(overflowsSmallViewport!.right).toBeLessThanOrEqual(overflowsSmallViewport!.viewportWidth + 0.5);
  expect(overflowsSmallViewport!.overflowX).toBeFalsy();
  expect(overflowsSmallViewport!.maxRowRight).not.toBeNull();
  expect(overflowsSmallViewport!.maxRowRight!).toBeLessThanOrEqual(overflowsSmallViewport!.right + 0.5);
  await page.screenshot({ path: "/tmp/ctx-diff-pane-right-narrow-small-viewport.png" });
});
