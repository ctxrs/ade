import { test, expect, chromium } from "./fixtures";
import { mkdirSync, rmSync, writeFileSync, readFileSync } from "fs";
import { mkdir } from "fs/promises";
import { execFileSync } from "child_process";
import path from "path";
import { fileURLToPath } from "url";
import WebSocket from "ws";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const VIEWPORT = { width: 1280, height: 720 } as const;

const OUT_DIR = process.env.CTX_PARITY_OUT_DIR ?? "/tmp/ctx-parity";
const REPO_DIR = process.env.CTX_PARITY_REPO_DIR ?? "/tmp/ctx-parity-repo";
const WORKSPACE_NAME = process.env.CTX_PARITY_WORKSPACE_NAME ?? "ws-parity";
const DATA_DIR = process.env.CTX_E2E_DATA_DIR ?? "";
const NATIVE_CARGO_TARGET_DIR =
  process.env.CTX_E2E_CARGO_TARGET_DIR_NATIVE ??
  process.env.CARGO_TARGET_DIR ??
  process.env.CTX_E2E_CARGO_TARGET_DIR ??
  "";
const AUTH_FILENAME = "daemon_auth.json";

const REPO_ROOT = path.resolve(__dirname, "../../../..");
const NATIVE_HEADLESS_RUN = path.join(
  REPO_ROOT,
  "core/apps/native/scripts/headless-run.mjs",
);
const PREPARE_PARITY_REPO = path.join(
  REPO_ROOT,
  "core/scripts/prepare-parity-repo.mjs",
);

async function ensureDir(p: string) {
  await mkdir(p, { recursive: true });
}

function prepareRepo(repo: string, mode: "clean" | "dirty" | "dirty_worktree") {
  execFileSync("node", [PREPARE_PARITY_REPO, "--repo", repo, "--mode", mode], {
    cwd: REPO_ROOT,
    stdio: "ignore",
  });
}

async function readAuthToken(): Promise<string> {
  if (!DATA_DIR) throw new Error("CTX_E2E_DATA_DIR must be set to read daemon auth token");
  const authPath = path.join(DATA_DIR, AUTH_FILENAME);
  let token = "";
  await expect
    .poll(
      () => {
        try {
          const raw = readFileSync(authPath, "utf8");
          const parsed = JSON.parse(raw);
          token = String(parsed?.token ?? "").trim();
          return token;
        } catch {
          token = "";
          return token;
        }
      },
      { timeout: 10_000 },
    )
    .not.toBe("");
  return token;
}

function idToString(value: any): string {
  if (value === null || value === undefined) return "";
  if (typeof value !== "string") {
    throw new Error("Expected id to be a string");
  }
  return value;
}

function workspaceIdFromPath(pathname: string): string {
  const match = String(pathname || "").match(/\/workspaces\/([^/?#]+)/);
  return match?.[1] ?? "";
}

async function listWorkspaceTasks(page: any, token: string, workspaceId: string): Promise<any[]> {
  const res = await page.request.get(`/api/workspaces/${workspaceId}/tasks`, {
    headers: { authorization: `Bearer ${token}` },
  });
  if (!res.ok()) {
    throw new Error(`list tasks failed: HTTP ${res.status()} ${res.statusText()}`);
  }
  const data = await res.json().catch(() => []);
  return Array.isArray(data) ? data : [];
}

async function listTaskSessions(page: any, token: string, taskId: string): Promise<any[]> {
  const res = await page.request.get(`/api/tasks/${taskId}/sessions`, {
    headers: { authorization: `Bearer ${token}` },
  });
  if (!res.ok()) {
    throw new Error(`list task sessions failed: HTTP ${res.status()} ${res.statusText()}`);
  }
  const data = await res.json().catch(() => []);
  return Array.isArray(data) ? data : [];
}

async function resolveTaskSessionId(
  page: any,
  token: string,
  workspaceId: string,
  taskTitle: string,
): Promise<{ taskId: string; sessionId: string }> {
  const tasks = await listWorkspaceTasks(page, token, workspaceId);
  const trimmedTitle = String(taskTitle ?? "").trim();
  const matching = tasks.filter((t) => String((t as any)?.title ?? "").trim() === trimmedTitle);
  if (matching.length === 0) {
    throw new Error(`Failed to resolve task for title "${trimmedTitle}"`);
  }
  matching.sort((a, b) =>
    String((b as any)?.updated_at ?? "").localeCompare(String((a as any)?.updated_at ?? "")),
  );
  const task = matching[0] as any;
  const taskId = idToString(task?.id);
  let sessionId = idToString(task?.primary_session_id);
  if (!sessionId && taskId) {
    const sessions = await listTaskSessions(page, token, taskId);
    sessionId = idToString((sessions as any[])?.[0]?.id);
  }
  if (!taskId || !sessionId) {
    throw new Error(`Failed to resolve task/session ids: ${JSON.stringify({ taskId, sessionId })}`);
  }
  return { taskId, sessionId };
}

async function waitForSessionCompleted(page: any, token: string, sessionId: string) {
  await expect
    .poll(
      async () => {
        const res = await page.request.get(`/api/sessions/${sessionId}/snapshot?limit=100`, {
          headers: { authorization: `Bearer ${token}` },
        });
        if (!res.ok()) return false;
        const snapshot = await res.json().catch(() => null);
        const head = snapshot?.head ?? {};
        const messages = Array.isArray(head.messages) ? head.messages : [];
        const turns = Array.isArray(head.turns) ? head.turns : [];
        const hasAssistant = messages.some(
          (m: any) => m?.role === "assistant" && String(m?.content ?? "").trim().length > 0,
        );
        const hasCompletedTurn = turns.some((t: any) => t?.status === "completed");
        return hasAssistant && hasCompletedTurn;
      },
      { timeout: 30_000 },
    )
    .toBe(true);
}

async function resolveWorktreeRootForSession(page: any, token: string, sessionId: string): Promise<string> {
  const snapRes = await page.request.get(`/api/sessions/${sessionId}/snapshot?limit=1`, {
    headers: { authorization: `Bearer ${token}` },
  });
  if (!snapRes.ok()) {
    throw new Error(`get session snapshot failed: HTTP ${snapRes.status()} ${snapRes.statusText()}`);
  }
  const snapshot = await snapRes.json().catch(() => null);
  const worktreeId = idToString(snapshot?.summary?.session?.worktree_id ?? snapshot?.head?.session?.worktree_id);
  if (!worktreeId) {
    throw new Error(`Failed to resolve worktree id from session snapshot: ${JSON.stringify(snapshot?.summary?.session)}`);
  }

  const wtRes = await page.request.get(`/api/worktrees/${worktreeId}`, {
    headers: { authorization: `Bearer ${token}` },
  });
  if (!wtRes.ok()) {
    throw new Error(`get worktree ${worktreeId} failed: HTTP ${wtRes.status()} ${wtRes.statusText()}`);
  }
  const worktree = await wtRes.json().catch(() => null);
  const rootPath = String(worktree?.root_path ?? "").trim();
  if (!rootPath) {
    throw new Error(`worktree ${worktreeId} missing root_path`);
  }
  return rootPath;
}

async function seedQueuedTranscriptMessages(page: any, token: string, sessionId: string) {
  for (let i = 1; i <= 28; i += 1) {
    const content = `scroll seed ${String(i).padStart(2, "0")}`;
    const res = await page.request.post(`/api/sessions/${sessionId}/messages`, {
      headers: { authorization: `Bearer ${token}` },
      data: { content, delivery: "queued" },
    });
    if (!res.ok()) {
      throw new Error(`seed transcript failed on message ${i}: HTTP ${res.status()} ${res.statusText()}`);
    }
  }
}

async function seedArtifacts(page: any, token: string, sessionId: string) {
  const a = path.join(OUT_DIR, "artifact-seed-a.txt");
  const b = path.join(OUT_DIR, "artifact-seed-b.txt");
  writeFileSync(a, "parity artifact A\n", "utf8");
  writeFileSync(b, "parity artifact B\n", "utf8");
  const res = await page.request.post(`/api/sessions/${sessionId}/artifacts`, {
    headers: { authorization: `Bearer ${token}` },
    data: {
      artifacts: [
        { absolute_file_path: a, name: "artifact-a.txt", mime_type: "text/plain" },
        { absolute_file_path: b, name: "artifact-b.txt", mime_type: "text/plain" },
      ],
    },
  });
  if (!res.ok()) {
    throw new Error(`set artifacts failed: HTTP ${res.status()} ${res.statusText()}`);
  }
}

async function waitForGitStatusIncludes(page: any, token: string, sessionId: string, needles: string[]) {
  await expect.poll(
    async () => {
      const res = await page.request.get(`/api/sessions/${sessionId}/git/status`, {
        headers: { authorization: `Bearer ${token}` },
      });
      if (!res.ok()) return false;
      const json = await res.json().catch(() => null);
      const raw = String(json?.raw ?? "");
      return needles.every((needle) => raw.includes(needle));
    },
    { timeout: 20_000 },
  ).toBe(true);
}

function terminalWsUrl(baseURL: string, token: string, terminalId: string): string {
  const url = new URL(`/api/terminals/${terminalId}/stream`, baseURL);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.searchParams.set("token", token);
  return url.toString();
}

async function seedTerminalOutput(baseURL: string, token: string, terminalId: string, repoDir: string) {
  const wsUrl = terminalWsUrl(baseURL, token, terminalId);
  const command = [
    "export PS1='$ ' PROMPT_COMMAND=''",
    `cd ${JSON.stringify(repoDir)}`,
    "printf '\\033[2J\\033[H'",
    "i=1; while [ $i -le 220 ]; do printf 'parity term %03d\\n' $i; i=$((i+1)); done",
    "echo PARITY_TERM_DONE",
  ].join(" && ");

  await new Promise<void>((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    const deadline = setTimeout(() => {
      try {
        ws.close();
      } catch {}
      reject(new Error("Timed out seeding terminal output"));
    }, 25_000);

    const finish = () => {
      clearTimeout(deadline);
      try {
        ws.close();
      } catch {}
      resolve();
    };

    ws.on("open", () => {
      ws.send(JSON.stringify({ kind: "resize", cols: 120, rows: 34 }));
      ws.send(JSON.stringify({ kind: "input", data: `${command}\n` }));
    });

    ws.on("message", (data) => {
      const buf = typeof data === "string" ? Buffer.from(data) : Buffer.from(data as any);
      if (buf.includes(Buffer.from("PARITY_TERM_DONE"))) {
        finish();
      }
    });

    ws.on("error", (err) => {
      clearTimeout(deadline);
      reject(err);
    });
  });
}

async function setTranscriptScroll(page: any) {
  const list = page.locator(".wb-thread-list").first();
  await expect(list).toBeVisible({ timeout: 20_000 });

  const metrics = await list
    .evaluate((el) => ({
      sh: (el as HTMLElement).scrollHeight,
      ch: (el as HTMLElement).clientHeight,
    }))
    .catch(() => null);
  if (!metrics || metrics.sh <= metrics.ch) return;

  await list
    .evaluate((el) => {
      const element = el as HTMLElement;
      const target = Math.max(0, element.scrollHeight - element.clientHeight - 520);
      element.scrollTop = target;
    })
    .catch(() => {});
}

async function listWorkspaceTerminals(page: any, token: string, workspaceId: string): Promise<any[]> {
  const res = await page.request.get(`/api/workspaces/${workspaceId}/terminals`, {
    headers: { authorization: `Bearer ${token}` },
  });
  if (!res.ok()) {
    throw new Error(`list terminals failed: HTTP ${res.status()} ${res.statusText()}`);
  }
  const data = await res.json().catch(() => []);
  return Array.isArray(data) ? data : [];
}

async function deleteTerminal(page: any, token: string, terminalId: string): Promise<void> {
  if (!terminalId) return;
  const res = await page.request.delete(`/api/terminals/${terminalId}`, {
    headers: { authorization: `Bearer ${token}` },
  });
  if (!res.ok()) {
    throw new Error(`delete terminal ${terminalId} failed: HTTP ${res.status()} ${res.statusText()}`);
  }
}

async function createWorkspaceTerminal(page: any, token: string, workspaceId: string, cwd: string | null) {
  const res = await page.request.post(`/api/workspaces/${workspaceId}/terminals`, {
    headers: { authorization: `Bearer ${token}` },
    data: {
      cwd,
    },
  });
  if (!res.ok()) {
    throw new Error(`create terminal failed: HTTP ${res.status()} ${res.statusText()}`);
  }
  const created = await res.json().catch(() => null);
  return idToString((created as any)?.id);
}

async function ensureNoWorkspaceTerminals(page: any, token: string, workspaceId: string) {
  if (!workspaceId) throw new Error("Failed to determine workspace id for terminal cleanup");
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    const terminals = await listWorkspaceTerminals(page, token, workspaceId);
    if (terminals.length === 0) return;
    await Promise.all(
      terminals.map((t) => deleteTerminal(page, token, idToString((t as any)?.id)).catch(() => {})),
    );
    await page.waitForTimeout(200);
  }
  const terminals = await listWorkspaceTerminals(page, token, workspaceId);
  throw new Error(`Timed out deleting terminals (remaining: ${terminals.length})`);
}

async function takeShot(page: any, screen: string, variant: string) {
  const dir = path.join(OUT_DIR, screen);
  await ensureDir(dir);
  const normalizeHover = !screen.includes("/hover/");
  const normalizeFocus = !screen.includes("/focus/") && !screen.includes("/menu/");

  if (normalizeHover) {
    await page.mouse.move(1270, 710).catch(() => {});
  }
  if (normalizeFocus) {
    await page
      .evaluate(() => {
        const el = document.activeElement as HTMLElement | null;
        el?.blur?.();
      })
      .catch(() => {});
  }
  try {
    await page.screenshot({
      path: path.join(dir, `${variant}.png`),
      type: "png",
      animations: "disabled",
      caret: "hide",
      timeout: 20_000,
    });
  } catch (err: any) {
    throw new Error(`takeShot(${screen}/${variant}) failed: ${err?.message ?? String(err)}`);
  }
}

test.describe("parity screenshots (web vs GPUI native)", () => {
  test.skip(
    process.env.CTX_GPUI_PARITY !== "1",
    "Set CTX_GPUI_PARITY=1 (or use playwright.parity.config.ts) to run this opt-in parity harness.",
  );

  test.setTimeout(600_000);

	  test("launcher, new task, active session, settings", async () => {
	    rmSync(OUT_DIR, { recursive: true, force: true });
	    mkdirSync(OUT_DIR, { recursive: true });
	    prepareRepo(REPO_DIR, "clean");
	    const token = await readAuthToken();

    const browser = await chromium.launch();
    const page = await browser.newPage({
      viewport: VIEWPORT,
      deviceScaleFactor: 1,
      reducedMotion: "reduce",
    });

    try {
      console.log("[parity] web: open workspaces");
      await page.goto(
        `/workspaces?token=${encodeURIComponent(token)}&desktop_ui=1`,
      );
      await expect(page.getByRole("button", { name: "Add workspace" })).toBeVisible({
        timeout: 20_000,
      });

      const workspaceLink = page
        .getByRole("listitem")
        .filter({ hasText: REPO_DIR })
        .getByRole("link", { name: WORKSPACE_NAME });

      if ((await workspaceLink.count()) === 0) {
        await page.getByLabel("Root path").fill(REPO_DIR);
        await page.getByLabel("Name (optional)").fill(WORKSPACE_NAME);
        await page.getByRole("button", { name: "Add workspace" }).click();
      }

      await expect(workspaceLink.first()).toBeVisible({ timeout: 20_000 });
      console.log("[parity] web: screenshot launcher");
      await takeShot(page, "launcher", "web");

      const workspaceHref = (await workspaceLink.first().getAttribute("href")) ?? "";
      const workspaceId = workspaceIdFromPath(workspaceHref);
      if (!workspaceId) {
        throw new Error(`Failed to parse workspace id from link href: ${workspaceHref || "<empty>"}`);
      }
      await ensureNoWorkspaceTerminals(page, token, workspaceId);
      await createWorkspaceTerminal(page, token, workspaceId, REPO_DIR);

      // Enter workbench (new task).
      console.log("[parity] web: enter workbench");
      await workspaceLink.first().click();
      await expect(page).toHaveURL(/\/workspaces\/[^/?#]+$/, { timeout: 20_000 });
      const workbenchUrl = new URL(page.url());
      workbenchUrl.searchParams.set("desktop_ui", "1");
      await page.goto(workbenchUrl.toString());

      // Workbench baseline.
      await expect(page.getByText("Archived").first()).toBeVisible({ timeout: 20_000 });

      // Create a task + session once in web so native sees identical backend state.
      console.log("[parity] web: select harness + send task");
      await page.locator(".wb-new-composer-stack").getByTitle("Harness").first().click();
      await expect(page.locator(".wb-harness-menu")).toBeVisible({ timeout: 20_000 });
      console.log("[parity] web: screenshot harness menu");
      await takeShot(page, "new-task/menu/harness", "web");
      await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
      await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).first().click();
      await page.locator(".wb-new-composer-stack button[title=\"Harness\"]").first().click();
      await page
        .locator(".wb-harness-menu")
        .waitFor({ state: "hidden", timeout: 5_000 })
        .catch(() => {});

      await page
        .locator(".wb-new-composer-stack textarea.wb-composer-textarea")
        .fill("hello");
      await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

      const rows = page.locator(".wb-task-row");
      await expect(rows).toHaveCount(1, { timeout: 20_000 });

      // Return to the New Task composer view (sending typically navigates to the created task/session).
      await page.locator(".wb-sidebar-action[title=\"New Task\"]").click();
      await expect(page.locator(".wb-new-composer-stack textarea.wb-composer-textarea")).toBeVisible({
        timeout: 20_000,
      });
	      await expect(rows.first()).toBeVisible({ timeout: 20_000 });
	      await expect(rows.filter({ hasText: "hello" }).first()).toBeVisible({ timeout: 20_000 });

	      const { sessionId } = await resolveTaskSessionId(page, token, workspaceId, "hello");
	      await waitForSessionCompleted(page, token, sessionId);
	      const worktreeRoot = await resolveWorktreeRootForSession(page, token, sessionId);
	      prepareRepo(worktreeRoot, "dirty_worktree");
	      await seedQueuedTranscriptMessages(page, token, sessionId);
	      await seedArtifacts(page, token, sessionId);
	      await waitForGitStatusIncludes(page, token, sessionId, ["file.txt", "z_added.txt"]);

      // New task baseline (after task exists, so the sidebar state matches native).
      console.log("[parity] web: screenshot new-task");
      await takeShot(page, "new-task", "web");

      // New task focus state.
      console.log("[parity] web: screenshot new-task focus");
      await page
        .locator(".wb-new-composer-stack textarea.wb-composer-textarea")
        .click({ timeout: 10_000 });
      await takeShot(page, "new-task/focus/composer", "web");

      console.log("[parity] web: screenshot new-task search focus");
      await page.locator("input.wb-search").click({ timeout: 10_000 });
      await takeShot(page, "new-task/focus/search", "web");

      console.log("[parity] web: screenshot archived empty");
      await page.locator(".wb-section-header-archived .wb-section-toggle").click();
      await expect(page.getByText("No archived tasks.").first()).toBeVisible({ timeout: 20_000 });
      await takeShot(page, "archived/empty", "web");
      await page.locator(".wb-section-header-archived .wb-section-toggle").click();
      await expect(page.getByText("No archived tasks.").first()).toBeHidden({ timeout: 20_000 });

      console.log("[parity] web: open active session");
      await rows.first().click();
      await expect(page.locator(".wb-session textarea.wb-active-textarea").first()).toBeVisible({
        timeout: 20_000,
      });
      await expect(page.locator(".wb-thread-list").first()).toBeVisible({ timeout: 20_000 });
      await setTranscriptScroll(page);

      console.log("[parity] web: screenshot active-session");
      await takeShot(page, "active-session", "web");

      // Integrated terminal (open panel).
      console.log("[parity] web: open terminal panel");
      const terminalToggle = page.getByRole("button", { name: "Toggle terminal panel" }).first();
      await expect(terminalToggle).toBeVisible({ timeout: 20_000 });
      await terminalToggle.click();
      await expect(page.locator(".wb-terminal-panel-inner")).toBeVisible({ timeout: 20_000 });

      const baseURL = new URL(page.url()).origin;
      const terminals = await listWorkspaceTerminals(page, token, workspaceId);
      const terminalId = idToString((terminals as any[])?.[0]?.id);
      if (!terminalId) {
        throw new Error(`Failed to resolve parity terminal id: ${JSON.stringify({ terminals_len: terminals.length })}`);
      }
      await seedTerminalOutput(baseURL, token, terminalId, REPO_DIR);

      await page
        .locator(".wb-terminal-panel-inner")
        .getByRole("button", { name: "Workspace" })
        .click()
        .catch(() => {});
      await expect(page.locator(".wb-terminal-panel-inner .wb-terminal-tab")).toHaveCount(1, {
        timeout: 20_000,
      });
      await expect(page.locator(".wb-terminal-panel-inner .wb-terminal-empty")).toBeHidden({
        timeout: 20_000,
      });
      await expect(page.locator(".wb-terminal-panel-inner .xterm")).toBeVisible({ timeout: 20_000 });
      console.log("[parity] web: screenshot terminal");
      await takeShot(page, "terminal", "web");
      await terminalToggle.click();
      await expect(page.locator(".wb-terminal-panel-inner")).toBeHidden({ timeout: 20_000 });

      // Active session with right pane (artifacts).
      console.log("[parity] web: open artifacts pane");
      const artifactsToggle = page.getByRole("button", { name: "Toggle artifacts" }).first();
      await expect(artifactsToggle).toBeVisible({ timeout: 20_000 });
      await artifactsToggle.click();
      await expect(page.locator(".wb-artifacts")).toBeVisible({ timeout: 20_000 });
      await expect(page.locator(".wb-artifact-card").first()).toBeVisible({ timeout: 20_000 });
      console.log("[parity] web: screenshot active-session artifacts pane");
      await takeShot(page, "active-session/right-pane/artifacts", "web");
      await takeShot(page, "right-pane/artifacts", "web");

      // Active session with right pane (diff).
      console.log("[parity] web: open diff pane");
      const diffToggle = page.getByRole("button", { name: "Toggle diff view" }).first();
      await expect(diffToggle).toBeVisible({ timeout: 20_000 });
      await diffToggle.click();
      await expect(page.locator(".wb-right-pane.wb-diff")).toBeVisible({ timeout: 20_000 });
      await expect(page.locator(".wb-diff-status-title")).toHaveText("Repo status", {
        timeout: 20_000,
      });
      await expect(page.locator(".wb-git-status-path").filter({ hasText: "file.txt" }).first()).toBeVisible({
        timeout: 20_000,
      });
      console.log("[parity] web: screenshot active-session diff pane");
      await takeShot(page, "right-pane/diff", "web");

      // Settings (pick a mostly-static section).
      console.log("[parity] web: open settings");
      await page.goto("/settings?desktop_ui=1#context_pack");
      await expect(page.locator(".settings-root")).toBeVisible({ timeout: 20_000 });
      await expect(page.locator(".settings-main-title")).toHaveText("ctx pack", {
        timeout: 20_000,
      });
      await expect(page.locator(".settings-empty")).toBeHidden({ timeout: 20_000 });
      console.log("[parity] web: screenshot settings");
      await takeShot(page, "settings", "web");

      await page.locator(".settings-search-input").click();
      console.log("[parity] web: screenshot settings focus");
      await takeShot(page, "settings/focus/search", "web");

      // Native capture (headless X + GPUI automation).
      console.log("[parity] native: capture screenshots");
      execFileSync(
        "node",
        [
          NATIVE_HEADLESS_RUN,
          "--window-size",
          `${VIEWPORT.width}x${VIEWPORT.height}`,
          "--screenshot-dir",
          OUT_DIR,
          "--automation-script",
          "core/apps/native/scripts/gpui-parity-screenshots.mjs",
          "--automation-settle-ms",
          "150",
        ],
        {
          cwd: REPO_ROOT,
          stdio: "inherit",
          env: {
            ...process.env,
            CTX_DATA_DIR: DATA_DIR,
            CTX_DAEMON_URL: baseURL,
            CTX_PARITY_WORKSPACE_NAME: WORKSPACE_NAME,
            CTX_PARITY_TASK_TEXT: "hello",
            CTX_PARITY_REPO_DIR: REPO_DIR,
            CARGO_TARGET_DIR: NATIVE_CARGO_TARGET_DIR,
          },
        },
      );
    } finally {
      await browser.close();
    }
  });
});
