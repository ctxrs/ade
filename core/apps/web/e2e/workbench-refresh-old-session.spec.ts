import { test, expect } from "./utils/fixtures";
import { existsSync, mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execFileSync, execSync } from "child_process";

const readId = (v: any): string => {
  if (!v) return "";
  if (typeof v === "string") return v;
  if (typeof v === "object" && typeof v["0"] === "string") return v["0"];
  return "";
};

test("workbench: refresh keeps selection, even for older sessions", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;

  await page.goto("/");
  await page.getByLabel("Root path").fill(repo);
  await page.getByLabel("Name (optional)").fill(workspaceName);
  await page.getByRole("button", { name: "Add workspace" }).click();
  await page
    .getByRole("listitem")
    .filter({ hasText: repo })
    .getByRole("link", { name: workspaceName })
    .click();

  const newComposer = page.locator(".wb-new-composer-stack");
  await expect(newComposer).toBeVisible({ timeout: 20000 });

  // Choose Fake harness so the test doesn't depend on external agents.
  await newComposer.getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
  await expect(
    newComposer.locator(".wb-switcher-wrap button[title=\"Harness\"] .wb-switcher-label"),
  ).toHaveText(/fake/i, { timeout: 20000 });

  await newComposer.locator("textarea.wb-composer-textarea").fill("hello refresh");
  await expect(newComposer.locator("button[aria-label=\"Send\"]")).toBeEnabled({ timeout: 20000 });
  await newComposer.locator("button[aria-label=\"Send\"]").click();

  const activeSession = page.locator(".wb-session-slot[aria-hidden=\"false\"]");
  const sessionComposer = activeSession.locator("textarea.wb-active-textarea");
  try {
    await expect(sessionComposer).toBeVisible({ timeout: 20000 });
  } catch (err) {
    await page.screenshot({ path: path.join(tmpdir(), "ctx-e2e-session-missing.png"), fullPage: true });
    throw err;
  }
  await expect(
    activeSession.locator(".wb-assistant-entry").filter({ hasText: "done: hello refresh" }).first(),
  ).toBeVisible({ timeout: 20000 });

  const url = new URL(page.url());
  const workspaceId = url.pathname.split("/").filter(Boolean).pop();
  expect(workspaceId).toBeTruthy();

  const snapshotResp = await page.request.get(`/api/workspaces/${workspaceId}/active_snapshot`);
  expect(snapshotResp.ok()).toBeTruthy();
  const snapshot = (await snapshotResp.json()) as any;
  const taskSummary =
    snapshot?.active?.tasks?.find((t: any) => String(t?.task?.title ?? "") === "hello refresh") ??
    snapshot?.active?.tasks?.[0];
  expect(taskSummary).toBeTruthy();
  const taskId = readId(taskSummary?.task?.id);
  const sessionSummary = taskSummary?.sessions?.[0];
  const sessionId = readId(sessionSummary?.session?.id) || readId(taskSummary?.task?.primary_session_id);

  await expect
    .poll(
      async () => {
        const resp = await page.request.get(`/api/sessions/${sessionId}/snapshot`);
        if (!resp.ok()) return 0;
        const snapshot = (await resp.json()) as any;
        const msgs = snapshot?.head?.messages ?? [];
        return msgs.filter((m: any) => m.role === "assistant").length;
      },
      { timeout: 20000 },
    )
    .toBeGreaterThan(0);

  const healthResp = await page.request.get("/api/health");
  expect(healthResp.ok()).toBeTruthy();
  const health = (await healthResp.json()) as any;
  const dataRoot = String(health.data_root ?? "");
  expect(dataRoot).toBeTruthy();

  const workspaceDbPath = path.join(dataRoot, "db", "workspaces", String(workspaceId), "db.sqlite");
  const dbPath = existsSync(workspaceDbPath) ? workspaceDbPath : path.join(dataRoot, "db", "db.sqlite");
  const sqliteJson = (sql: string) => {
    const out = execFileSync("sqlite3", ["-json", dbPath, sql], { encoding: "utf8" }).trim();
    return out ? (JSON.parse(out) as any[]) : [];
  };

  const shiftMs = 2 * 24 * 60 * 60 * 1000;
  const shift = (iso: string) => new Date(Date.parse(iso) - shiftMs).toISOString();

  let taskRow = sqliteJson(`SELECT id, created_at, updated_at FROM tasks WHERE id='${taskId}'`)[0];
  if (!taskRow) {
    taskRow = sqliteJson(
      `SELECT id, created_at, updated_at FROM tasks WHERE title='hello refresh' ORDER BY created_at DESC LIMIT 1`,
    )[0];
  }
  const resolvedTaskId = String(taskRow?.id ?? taskId ?? "");
  expect(resolvedTaskId).not.toEqual("");

  let sessionRow = sqliteJson(`SELECT id, created_at, updated_at FROM sessions WHERE id='${sessionId}'`)[0];
  if (!sessionRow && resolvedTaskId) {
    sessionRow = sqliteJson(
      `SELECT id, created_at, updated_at FROM sessions WHERE task_id='${resolvedTaskId}' ORDER BY created_at DESC LIMIT 1`,
    )[0];
  }
  const resolvedSessionId = String(sessionRow?.id ?? sessionId ?? "");
  expect(resolvedSessionId).not.toEqual("");

  const messageRows = sqliteJson(`SELECT id, created_at FROM messages WHERE session_id='${resolvedSessionId}'`);
  const eventRows = sqliteJson(`SELECT id, created_at FROM session_events WHERE session_id='${resolvedSessionId}'`);

  const sqlUpdates = [
    `PRAGMA busy_timeout=5000;`,
    `BEGIN;`,
    `UPDATE tasks SET created_at='${shift(String(taskRow.created_at))}', updated_at='${shift(String(taskRow.updated_at))}' WHERE id='${resolvedTaskId}';`,
    `UPDATE sessions SET created_at='${shift(String(sessionRow.created_at))}', updated_at='${shift(String(sessionRow.updated_at))}' WHERE id='${resolvedSessionId}';`,
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
    await expect(activeSession.locator("textarea.wb-active-textarea")).toBeVisible({ timeout: 20000 });
  } catch (err) {
    await page.screenshot({ path: path.join(tmpdir(), "ctx-e2e-after-reload-session-missing.png"), fullPage: true });
    throw err;
  }
  await expect(activeSession.locator(".wb-assistant-entry").filter({ hasText: "done: hello refresh" })).toBeVisible({
    timeout: 20000,
  });

  const composer = activeSession.locator("textarea.wb-active-textarea");
  await composer.click();
  await composer.type("hello again");
  await expect(composer).toHaveValue("hello again", { timeout: 20000 });
  const sendButton = activeSession.locator("button[aria-label=\"Send\"]");
  await expect(sendButton).toBeEnabled({ timeout: 20000 });
  await sendButton.click();
  await expect
    .poll(
      async () => {
        const resp = await page.request.get(`/api/sessions/${sessionId}/snapshot?limit=50`);
        if (!resp.ok()) return 0;
        const snapshot = (await resp.json()) as any;
        const msgs = snapshot?.head?.messages ?? [];
        return msgs.filter((m: any) => m.role === "assistant").length;
      },
      { timeout: 20000 },
    )
    .toBeGreaterThanOrEqual(2);
  await page.locator(".thread-stack").evaluate((root) => {
    const el = root as HTMLElement;
    const candidates = [el, ...Array.from(el.querySelectorAll<HTMLElement>("*"))];
    const scroller = candidates.find((n) => n.scrollHeight > n.clientHeight && getComputedStyle(n).overflowY !== "visible");
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });
  await expect(page.locator(".wb-session")).toContainText("done: hello again", { timeout: 15000 });
});
