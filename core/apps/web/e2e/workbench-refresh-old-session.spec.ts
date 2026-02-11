import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execFileSync, execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

const readId = (v: any): string => (typeof v === "string" ? v : "");

test("workbench: refresh keeps selection, even for older sessions", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;

  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request: page.request,
    repo,
    workspaceName,
  });

  // Choose Fake harness so the test doesn't depend on external agents.
  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
  await expect(
    page.locator(".wb-new-composer-stack button[title=\"Harness\"] .wb-switcher-label"),
  ).toHaveText(/fake/i, { timeout: 20000 });

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill("hello refresh");
  await expect(page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]")).toBeEnabled({ timeout: 20000 });
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  const sessionComposer = page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea");
  try {
    await expect(sessionComposer).toBeVisible({ timeout: 20000 });
  } catch (err) {
    await page.screenshot({ path: path.join(tmpdir(), "ctx-e2e-session-missing.png"), fullPage: true });
    throw err;
  }
  await expect(
    page.locator(".wb-session .wb-assistant-entry").filter({ hasText: "done: hello refresh" }).first(),
  ).toBeVisible({ timeout: 20000 });

  const snapshotResp = await page.request.get(`/api/workspaces/${workspaceId}/active_snapshot`);
  expect(snapshotResp.ok()).toBeTruthy();
  const snapshot = (await snapshotResp.json()) as any;
  const taskSummary =
    snapshot?.active?.tasks?.find((t: any) => String(t?.task?.title ?? "") === "hello refresh") ??
    snapshot?.active?.tasks?.[0];
  expect(taskSummary).toBeTruthy();
  const taskId = readId(taskSummary?.task?.id);
  const sessionSummary = taskSummary?.sessions?.[0];
  const primarySessionId = readId(taskSummary?.primary_session?.session?.id);
  let sessionId =
    readId(sessionSummary?.session?.id) || primarySessionId || readId(taskSummary?.task?.primary_session_id);
  if (!sessionId) {
    const sessionsResp = await page.request.get(`/api/tasks/${taskId}/sessions`);
    if (sessionsResp.ok()) {
      const sessions = (await sessionsResp.json()) as any[];
      sessionId = readId(sessions?.[0]?.id);
    }
  }
  expect(sessionId).toBeTruthy();

  const healthResp = await page.request.get("/api/health");
  expect(healthResp.ok()).toBeTruthy();
  const health = (await healthResp.json()) as any;
  const dataRoot = String(health.data_root ?? "");
  expect(dataRoot).toBeTruthy();

  const dbPath = path.join(dataRoot, "db", "workspaces", workspaceId!, "db.sqlite");
  const sqliteJson = (sql: string) => {
    const out = execFileSync("sqlite3", ["-json", dbPath, sql], { encoding: "utf8" }).trim();
    return out ? (JSON.parse(out) as any[]) : [];
  };

  const shiftMs = 2 * 24 * 60 * 60 * 1000;
  const shift = (iso: string) => new Date(Date.parse(iso) - shiftMs).toISOString();

  const taskRow = sqliteJson(`SELECT created_at, updated_at FROM tasks WHERE id='${taskId}'`)[0];
  const sessionRow = sqliteJson(`SELECT created_at, updated_at FROM sessions WHERE id='${sessionId}'`)[0];
  const messageRows = sqliteJson(`SELECT id, created_at FROM messages WHERE session_id='${sessionId}'`);
  const eventRows = sqliteJson(`SELECT id, created_at FROM session_events WHERE session_id='${sessionId}'`);

  const sqlUpdates = [
    `PRAGMA busy_timeout=5000;`,
    `BEGIN;`,
    `UPDATE tasks SET created_at='${shift(String(taskRow.created_at))}', updated_at='${shift(String(taskRow.updated_at))}' WHERE id='${taskId}';`,
    `UPDATE sessions SET created_at='${shift(String(sessionRow.created_at))}', updated_at='${shift(String(sessionRow.updated_at))}' WHERE id='${sessionId}';`,
    ...messageRows.map((r) => `UPDATE messages SET created_at='${shift(String(r.created_at))}' WHERE id='${String(r.id)}';`),
    ...eventRows.map((r) => `UPDATE session_events SET created_at='${shift(String(r.created_at))}' WHERE id='${String(r.id)}';`),
    `COMMIT;`,
  ].join("\n");

  execFileSync("sqlite3", [dbPath, sqlUpdates]);

  await page.reload();

  const urlAfter = new URL(page.url());
  expect(urlAfter.searchParams.get("task")).toBeNull();
  expect(urlAfter.searchParams.get("track")).toBeNull();
  expect(urlAfter.searchParams.get("session")).toBeNull();

  try {
    await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({ timeout: 20000 });
  } catch (err) {
    await page.screenshot({ path: path.join(tmpdir(), "ctx-e2e-after-reload-session-missing.png"), fullPage: true });
    throw err;
  }
  await expect(page.locator(".wb-session .wb-assistant-entry").filter({ hasText: "done: hello refresh" })).toBeVisible({
    timeout: 20000,
  });

  const composer = page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea");
  await composer.click();
  await composer.type("hello again");
  await expect(composer).toHaveValue("hello again", { timeout: 20000 });
  const sendButton = page.locator(".wb-session-slot[aria-hidden=\"false\"] button[aria-label=\"Send\"]");
  await expect(sendButton).toBeEnabled({ timeout: 20000 });
  await sendButton.click();
  await page.locator(".thread-stack").evaluate((root) => {
    const el = root as HTMLElement;
    const candidates = [el, ...Array.from(el.querySelectorAll<HTMLElement>("*"))];
    const scroller = candidates.find((n) => n.scrollHeight > n.clientHeight && getComputedStyle(n).overflowY !== "visible");
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });
  await expect(page.locator(".wb-session")).toContainText("done: hello again", { timeout: 15000 });
});
