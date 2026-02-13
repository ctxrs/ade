import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const readId = (value: unknown): string => (typeof value === "string" ? value : "");

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
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

test("workbench: worktree slug is visible for the active session", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
    createDefaultTrack: false,
  });

  const taskId = seed.taskIds[0];
  const sessionId = seed.sessionIdsByTask[taskId][0];

  const sessionResp = await request.get(`/api/sessions/${sessionId}/snapshot?limit=1`);
  expect(sessionResp.ok()).toBeTruthy();
  const sessionSnapshot = asRecord(await sessionResp.json());
  const headSession = asRecord(asRecord(sessionSnapshot.head).session);
  const summarySession = asRecord(asRecord(sessionSnapshot.summary).session);
  const worktreeId = readId(
    headSession.worktree_id ?? summarySession.worktree_id,
  );
  expect(worktreeId).not.toEqual("");

  const worktreeResp = await request.get(`/api/worktrees/${worktreeId}`);
  expect(worktreeResp.ok()).toBeTruthy();
  const worktree = asRecord(await worktreeResp.json());
  const worktreePath = String(worktree.root_path ?? "");
  expect(worktreePath).not.toEqual("");

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1);
  await rows.first().click();

  const worktreeChip = page.getByRole("button", { name: "Copy worktree location" }).first();
  await expect(worktreeChip).toBeVisible({ timeout: 20000 });
  await expect(worktreeChip).toContainText(worktreeSlugFromPath(worktreePath));
});

test("workbench: worktree slug stays visible in single-track view", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
    createDefaultTrack: false,
  });

  const taskId = seed.taskIds[0];
  const sessionId = seed.sessionIdsByTask[taskId][0];

  const sessionResp = await request.get(`/api/sessions/${sessionId}/snapshot?limit=1`);
  expect(sessionResp.ok()).toBeTruthy();
  const sessionSnapshot = asRecord(await sessionResp.json());
  const headSession = asRecord(asRecord(sessionSnapshot.head).session);
  const summarySession = asRecord(asRecord(sessionSnapshot.summary).session);
  const worktreeId = readId(
    headSession.worktree_id ?? summarySession.worktree_id,
  );
  expect(worktreeId).not.toEqual("");

  const worktreeResp = await request.get(`/api/worktrees/${worktreeId}`);
  expect(worktreeResp.ok()).toBeTruthy();
  const worktree = asRecord(await worktreeResp.json());
  const worktreePath = String(worktree.root_path ?? "");
  expect(worktreePath).not.toEqual("");

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1);
  await rows.first().click();

  const worktreeChip = page.getByRole("button", { name: "Copy worktree location" }).first();
  await expect(worktreeChip).toBeVisible({ timeout: 20000 });
  await expect(worktreeChip).toContainText(worktreeSlugFromPath(worktreePath));
});
