import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";

const getWorkspaceIdFromUrl = (url: string): string => {
  const parts = new URL(url).pathname.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? "";
};

const readId = (v: any): string => {
  if (!v) return "";
  if (typeof v === "string") return v;
  if (typeof v === "object" && typeof v["0"] === "string") return v["0"];
  return "";
};

test("workbench: stale cached events do not re-show running", async ({ page }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1400, height: 900 });

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

  // Choose Fake harness so the test doesn't depend on external agents.
  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
  await expect(
    page.locator('.wb-new-composer-stack button[title="Harness"] .wb-switcher-label'),
  ).toHaveText(/fake/i, { timeout: 20000 });

  // Use Local isolation to keep the test fast and deterministic.
  await page.locator(".wb-new-composer-stack").getByTitle("Isolation").click();
  await page.locator(".wb-exec-menu").getByRole("button", { name: "Local" }).click();
  await expect(
    page.locator('.wb-new-composer-stack button[title="Isolation"] .wb-switcher-label'),
  ).toHaveText(/local/i, { timeout: 20000 });

  const prompt = "stale-cache-spinner";
  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill(prompt);
  await page.locator('.wb-new-composer-stack button[aria-label="Send"]').click();

  const workspaceId = getWorkspaceIdFromUrl(page.url());
  expect(workspaceId).not.toBe("");

  let sessionIdValue = "";
  await expect
    .poll(
      async () => {
        const resp = await page.request.get(`/api/workspaces/${workspaceId}/catchup`);
        if (!resp.ok()) return "";
        const data = await resp.json();
        const session = data?.active?.tasks?.[0]?.sessions?.[0]?.session?.id;
        const primary = data?.active?.tasks?.[0]?.task?.primary_session_id;
        sessionIdValue = readId(session) || readId(primary);
        return sessionIdValue;
      },
      { timeout: 20000 },
    )
    .not.toBe("");
  expect(sessionIdValue).not.toBe("");

  await page.waitForFunction(async (sid) => {
    const resp = await fetch(`/api/sessions/${sid}/head?include_events=1&limit=60`);
    if (!resp.ok) return false;
    const data = await resp.json();
    const turns = Array.isArray(data.turns) ? data.turns : [];
    const lastTurn = turns[turns.length - 1];
    const hasAssistant = Array.isArray(data.messages) && data.messages.some((m: any) => m?.role === "assistant");
    return Boolean(hasAssistant && lastTurn?.status === "completed");
  }, sessionIdValue, { timeout: 20000 });

  const head = await page.evaluate(async (sid) => {
    const resp = await fetch(`/api/sessions/${sid}/head?include_events=1&limit=60`);
    if (!resp.ok) return null;
    return resp.json();
  }, sessionIdValue);
  expect(head).not.toBeNull();

  const doneLike = new Set(["done", "assistant_complete", "turn_interrupted"]);
  const staleEvents = Array.isArray(head.events)
    ? head.events.filter((ev: any) => !doneLike.has(String(ev?.event_type ?? "")))
    : [];
  let nextEvents = staleEvents.length > 0 ? staleEvents : head.events ?? [];
  if (nextEvents.length === 0) {
    nextEvents = [
      {
        seq: Math.max(0, (head.last_event_seq ?? 1) - 1),
        id: "stale-event",
        session_id: sessionIdValue,
        run_id: null,
        turn_id: null,
        event_type: "assistant_message_inserted",
        payload_json: {},
        created_at: new Date().toISOString(),
      },
    ];
  }
  const staleHead = { ...head, events: nextEvents, last_event_seq: head.last_event_seq };

  await page.evaluate(
    async ({ sid, stale }) => {
      await new Promise<void>((resolve, reject) => {
        const req = indexedDB.open("ctx-ui", 1);
        req.onupgradeneeded = () => {
          const db = req.result;
          if (!db.objectStoreNames.contains("kv")) {
            db.createObjectStore("kv", { keyPath: "key" });
          }
        };
        req.onerror = () => reject(req.error ?? new Error("IndexedDB open failed"));
        req.onsuccess = () => {
          const db = req.result;
          const tx = db.transaction("kv", "readwrite");
          const store = tx.objectStore("kv");
          const stored = { v: 1, sessionId: sid, head: stale, updatedAtMs: Date.now() };
          store.put({ key: `wb.session_head.v1.${sid}`, value: stored, updatedAtMs: Date.now() });
          tx.oncomplete = () => resolve();
          tx.onerror = () => reject(tx.error ?? new Error("IndexedDB transaction failed"));
          tx.onabort = () => reject(tx.error ?? new Error("IndexedDB transaction aborted"));
        };
      });
    },
    { sid: sessionIdValue, stale: staleHead },
  );

  await page.reload();
  await expect(page.locator(".wb-task-spinner")).toHaveCount(0, { timeout: 5000 });
});
