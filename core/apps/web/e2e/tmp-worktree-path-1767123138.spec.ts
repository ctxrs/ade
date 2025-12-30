import { test, expect } from "playwright/test";
import { execSync } from "child_process";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";

const readId = (value: any): string => {
  if (!value) return "";
  if (typeof value === "string") return value;
  if (typeof value === "object" && typeof value["0"] === "string") return value["0"];
  return "";
};

const worktreeSlugFromPath = (worktreePath: string): string => {
  const trimmed = worktreePath.trim().replace(/[\\/]+$/, "");
  if (!trimmed) return "";
  const parts = trimmed.split(/[\\/]/).filter(Boolean);
  const base = parts[parts.length - 1] ?? "";
  const uuidMatch = base.match(/^([0-9a-f]{8})-[0-9a-f-]{27,}$/i);
  if (uuidMatch) return uuidMatch[1];
  if (base.length <= 16) return base;
  return `${base.slice(0, 16)}...`;
};

test("tmp: worktree path visible", async ({ page, request }) => {
  await page.context().grantPermissions(["clipboard-write"]);
  const repo = mkdtempSync(path.join(tmpdir(), "context-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "fixture\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceResp = await request.post("/api/workspaces", {
    data: { root_path: repo, name: `ws-${Date.now()}` },
  });
  expect(workspaceResp.ok()).toBeTruthy();
  const workspace = (await workspaceResp.json()) as any;
  const workspaceId = readId(workspace?.id);
  expect(workspaceId).not.toEqual("");

  const taskResp = await request.post(`/api/workspaces/${workspaceId}/tasks`, {
    data: { title: "tmp task", create_default_track: false },
  });
  expect(taskResp.ok()).toBeTruthy();
  const task = (await taskResp.json()) as any;
  const taskId = readId(task?.id);
  expect(taskId).not.toEqual("");

  const trackResp = await request.post(`/api/tasks/${taskId}/tracks`, {
    data: { label: "tmp track", env_target: "worktree" },
  });
  expect(trackResp.ok()).toBeTruthy();
  const track = (await trackResp.json()) as any;
  const trackId = readId(track?.id);
  expect(trackId).not.toEqual("");

  const sessionResp = await request.post(`/api/tracks/${trackId}/sessions`, {
    data: { provider_id: "fake", model_id: "fake-model" },
  });
  expect(sessionResp.ok()).toBeTruthy();
  const session = (await sessionResp.json()) as any;
  const sessionId = readId(session?.id);
  expect(sessionId).not.toEqual("");

  const sessionHeadResp = await request.get(`/api/sessions/${sessionId}/head?limit=1`);
  expect(sessionHeadResp.ok()).toBeTruthy();
  const sessionHead = (await sessionHeadResp.json()) as any;
  const worktreeId = readId(sessionHead?.session?.worktree_id);
  expect(worktreeId).not.toEqual("");

  const worktreeResp = await request.get(`/api/worktrees/${worktreeId}`);
  expect(worktreeResp.ok()).toBeTruthy();
  const worktree = (await worktreeResp.json()) as any;
  const worktreePath = String(worktree?.root_path ?? "");
  expect(worktreePath).not.toEqual("");

  await page.goto(`/workspaces/${workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1);
  await rows.first().click();

  const worktreeChip = page.locator(".wb-single-track-meta-left .wb-worktree-chip");
  await expect(worktreeChip).toBeVisible({ timeout: 20000 });
  await expect(worktreeChip).toContainText(worktreeSlugFromPath(worktreePath));

  await page.screenshot({ path: "/tmp/workbench-worktree-inline-normal.png", fullPage: true });

  await worktreeChip.hover();
  await page.screenshot({ path: "/tmp/workbench-worktree-inline-hover.png", fullPage: true });

  await worktreeChip.click();
  await expect(worktreeChip).toHaveClass(/wb-worktree-chip-copied/);
  await page.screenshot({ path: "/tmp/workbench-worktree-inline-checked.png", fullPage: true });
});
