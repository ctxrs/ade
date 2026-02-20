import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";
import { selectHarnessBySearch } from "./utils/harnessEndpointAuth";
import type { Page } from "playwright/test";

const AUTH_TOKEN = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";

type TerminalBufferSnapshot = {
  baseY?: number;
  viewportY?: number;
  ydisp?: number;
  length?: number;
};

type TerminalEntry = {
  element?: HTMLElement;
  textarea?: HTMLTextAreaElement;
  focus?: () => void;
  scrollToBottom?: () => void;
  buffer?: { active?: TerminalBufferSnapshot };
};

type TerminalRegistryWindow = Window & {
  __ctxE2ETerminals?: Map<string, TerminalEntry>;
};

type TerminalSession = {
  id: unknown;
  created_at: string;
  status: string;
  task_id?: unknown;
};

const readTerminalId = (value: unknown): string | undefined => {
  if (typeof value === "string" && value) return value;
  if (Array.isArray(value) && typeof value[0] === "string" && value[0]) return value[0];
  return undefined;
};

test("terminal scroll stays consistent after closing and reopening panel while output streams", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-terminal-scroll-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-terminal-scroll-${Date.now()}`;
  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request: page.request,
    repo,
    workspaceName,
    token: AUTH_TOKEN,
  });

  // Create a task so the workbench (and terminal toggle) is visible.
  await selectHarnessBySearch(page, "fake", /fake/i);
  await page.locator("textarea.wb-composer-textarea").first().fill("terminal scroll test");
  await page.getByRole("button", { name: "Send" }).click();

  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20_000 });
  await rows.first().click();
  await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({ timeout: 20_000 });

  await openTerminalPanel(page);

  const panel = page.locator(".wb-terminal-panel-inner");
  // Use Task scope to avoid the workspace auto-terminal creation effect racing this test.
  await panel.getByRole("button", { name: "Task" }).click();

  // Always create a fresh terminal so we don't accidentally target an exited one.
  const createStartedAt = Date.now();
  const beforeTabs = await panel.locator(".wb-terminal-tab").count();
  await panel.getByTitle("New terminal").click();
  await expect.poll(() => panel.locator(".wb-terminal-tab").count()).toBeGreaterThan(beforeTabs);
  await panel.locator(".wb-terminal-tab").last().click();
  await expect(panel.locator(".wb-terminal-tab-active.wb-terminal-tab-exited")).toHaveCount(0);

  const terminalId = await waitForNewestWorkspaceTerminalId(page, workspaceId, createStartedAt, { requireTaskId: true });
  await expect
    .poll(() => getScrollState(page, terminalId).then((s) => s.present), { timeout: 20_000 })
    .toBe(true);
  await expect
    .poll(() => isTerminalVisibleById(page, terminalId), { timeout: 30_000 })
    .toBe(true);

  try {
    // Type into the terminal itself. Using a second WS client to inject input can race
    // with (or replace) the browser's own terminal stream connection, making the test flaky.
    const startBaseY = (await getScrollState(page, terminalId)).baseY;
    await typeCommandInTerminal(page, terminalId, makeStreamingCommand());

    await expect
      .poll(() => getScrollState(page, terminalId).then((s) => s.baseY), { timeout: 20_000 })
      .toBeGreaterThanOrEqual(startBaseY + 20);
    const beforeClose = (await getScrollState(page, terminalId)).baseY;

    await closeTerminalPanel(page);

    // Let output accumulate while the panel is closed.
    await page.waitForTimeout(2500);

    await openTerminalPanel(page);
    await panel.getByRole("button", { name: "Task" }).click();
    // Re-assert the intended tab after reopening; the panel may restore a different active tab.
    await panel.locator(".wb-terminal-tab").last().click();
    await expect
      .poll(() => isTerminalVisibleById(page, terminalId), { timeout: 30_000 })
      .toBe(true);

    // Ensure new output arrived while the panel was closed.
    await expect
      .poll(() => getScrollState(page, terminalId).then((s) => s.baseY), { timeout: 20_000 })
      .toBeGreaterThanOrEqual(beforeClose + 50);

    const baseYTarget = (await getScrollState(page, terminalId)).baseY;
    await scrollTerminalViewportToBottom(page, terminalId);
    await page.waitForTimeout(200);
    await scrollTerminalViewportToBottom(page, terminalId);

    await expect
      .poll(
        async () => {
          const state = await getScrollState(page, terminalId);
          const viewportY = state.ydisp ?? state.viewportY;
          if (viewportY === null) return false;
          // Use a fixed target so the assertion doesn't become a moving goalpost while output continues.
          return viewportY >= baseYTarget;
        },
        { timeout: 20_000 },
      )
      .toBe(true);
  } finally {
    // Ensure we don't leave a runaway terminal behind if the test fails mid-stream.
    await page.request
      .delete(`/api/terminals/${terminalId}`, { headers: { authorization: `Bearer ${AUTH_TOKEN}` } })
      .catch(() => {});
  }
});

function makeStreamingCommand() {
  // Emit enough output to build scrollback while we briefly hide the panel.
  // Keep it finite to avoid leaving runaway processes behind.
  // Avoid fractional sleeps (some shells/environments treat them as errors).
  return `i=0; while [ $i -lt 2500 ]; do i=$((i+1)); echo line $i; if [ $((i % 200)) -eq 0 ]; then sleep 1; fi; done`;
}

async function openTerminalPanel(page: Page) {
  const terminalToggle = page.getByRole("button", { name: "Toggle terminal panel" }).first();
  await expect(terminalToggle).toBeVisible({ timeout: 20_000 });
  const panel = page.locator(".wb-terminal-panel-inner");
  if (!(await panel.isVisible())) {
    await terminalToggle.click();
  }
  await expect(panel).toBeVisible({ timeout: 20_000 });
}

async function closeTerminalPanel(page: Page) {
  const terminalToggle = page.getByRole("button", { name: "Toggle terminal panel" }).first();
  const panel = page.locator(".wb-terminal-panel-inner");
  if (await panel.isVisible()) {
    await terminalToggle.click();
  }
  await expect(panel).toBeHidden({ timeout: 20_000 });
}

async function getScrollState(
  page: Page,
  terminalId: string,
): Promise<{
  present: boolean;
  baseY: number;
  viewportY: number | null;
  ydisp: number | null;
  length: number;
}> {
  return await page.evaluate((id: string) => {
    const reg = (window as TerminalRegistryWindow).__ctxE2ETerminals;
    const term = reg?.get(id);
    const buf = term?.buffer?.active;
    return {
      present: !!term,
      baseY: typeof buf?.baseY === "number" ? buf.baseY : 0,
      viewportY: typeof buf?.viewportY === "number" ? buf.viewportY : null,
      ydisp: typeof buf?.ydisp === "number" ? buf.ydisp : null,
      length: typeof buf?.length === "number" ? buf.length : 0,
    };
  }, terminalId);
}

async function isTerminalVisibleById(page: Page, terminalId: string): Promise<boolean> {
  return await page.evaluate((id: string) => {
    const reg = (window as TerminalRegistryWindow).__ctxE2ETerminals;
    const term = reg?.get(id);
    const el = term?.element as HTMLElement | undefined;
    if (!el || !el.isConnected) return false;
    if (el.closest(".wb-terminal-group-hidden")) return false;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 5 || rect.height <= 5) return false;
    // Avoid false positives for visibility:hidden.
    const style = window.getComputedStyle(el);
    if (style.visibility === "hidden" || style.display === "none") return false;
    return true;
  }, terminalId);
}

async function waitForNewestWorkspaceTerminalId(
  page: Page,
  workspaceId: string,
  createdAfterMs: number,
  opts?: { requireTaskId?: boolean },
): Promise<string> {
  let terminalId = "";
  await expect
    .poll(
      async () => {
        const resp = await page.request.get(`/api/workspaces/${workspaceId}/terminals`, {
          headers: { authorization: `Bearer ${AUTH_TOKEN}` },
        });
        if (!resp.ok()) return "";
        const terminals = (await resp.json()) as TerminalSession[];
        const newest = terminals
          .map((t) => ({
            id: readTerminalId(t.id),
            createdAtMs: Date.parse(t.created_at),
            status: t.status,
            taskId: t.task_id,
          }))
          .filter((t) => Boolean(t.id) && Number.isFinite(t.createdAtMs))
          .filter((t) => (opts?.requireTaskId ? Boolean(t.taskId) : true))
          .sort((a, b) => b.createdAtMs - a.createdAtMs)[0];
        if (!newest?.id) return "";
        // Prefer a running terminal created after we clicked "New terminal".
        if (newest.createdAtMs < createdAfterMs - 1000) return "";
        if (newest.status !== "running") return "";
        terminalId = newest.id;
        return terminalId;
      },
      { timeout: 20_000 },
    )
    .not.toBe("");
  return terminalId;
}

async function typeCommandInTerminal(page: Page, terminalId: string, command: string) {
  await focusTerminalById(page, terminalId);
  await page.keyboard.type(command);
  await page.keyboard.press("Enter");
}

async function focusTerminalById(page: Page, terminalId: string) {
  await page.evaluate((id: string) => {
    const reg = (window as TerminalRegistryWindow).__ctxE2ETerminals;
    const term = reg?.get(id);
    if (!term) return;
    // xterm Terminal has a public focus() method; also try focusing the internal textarea.
    term.focus?.();
    try {
      (term.textarea as HTMLTextAreaElement | undefined)?.focus?.();
    } catch {
      // ignore
    }
  }, terminalId);

  await expect
    .poll(
      async () =>
        page.evaluate((id: string) => {
          const reg = (window as TerminalRegistryWindow).__ctxE2ETerminals;
          const term = reg?.get(id);
          const root = term?.element as HTMLElement | undefined;
          const active = document.activeElement as HTMLElement | null;
          if (!root || !active) return false;
          return root.contains(active);
        }, terminalId),
      { timeout: 20_000 },
    )
    .toBe(true);
}

async function scrollTerminalViewportToBottom(page: Page, terminalId: string) {
  await page.evaluate((id: string) => {
    const reg = (window as TerminalRegistryWindow).__ctxE2ETerminals;
    const term = reg?.get(id);
    term?.scrollToBottom?.();
    const root = term?.element as HTMLElement | undefined;
    const viewport = root?.querySelector(".xterm-viewport") as HTMLElement | null | undefined;
    if (!viewport) return;
    viewport.scrollTop = viewport.scrollHeight;
    viewport.dispatchEvent(new Event("scroll"));
  }, terminalId);
}
