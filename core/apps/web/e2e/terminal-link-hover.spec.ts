import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import WebSocket from "ws";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

const AUTH_TOKEN = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";
const LINK_TEXT = "https://example.com";
const TERMINAL_DONE = "TERM_LINK_DONE";

test("terminal links underline on modifier hover", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-terminal-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-terminal-${Date.now()}`;
  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request: page.request,
    repo,
    workspaceName,
    token: AUTH_TOKEN,
  });

  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill("terminal link check");
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20_000 });
  await rows.first().click();
  await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({ timeout: 20_000 });

  await openTerminalPanel(page);
  await ensureTerminalVisible(page);
  const terminalId = await waitForWorkspaceTerminal(page, AUTH_TOKEN, workspaceId);

  const baseURL = new URL(page.url()).origin;
  await seedTerminalOutput(baseURL, AUTH_TOKEN, terminalId, LINK_TEXT);

  const metrics = await getTerminalMetrics(page);
  const hoverX = metrics.left + metrics.cellWidth * 2.5;
  const hoverY = metrics.top + metrics.cellHeight * 0.5;
  const clip = {
    x: Math.max(0, Math.floor(metrics.left)),
    y: Math.max(0, Math.floor(metrics.top)),
    width: Math.ceil(metrics.cellWidth * (LINK_TEXT.length + 2)),
    height: Math.ceil(metrics.cellHeight),
  };

  await page.mouse.move(hoverX, hoverY);

  const baseShot = await page.screenshot({ clip });
  const modifierKey = process.platform === "darwin" ? "Meta" : "Control";
  await page.keyboard.down(modifierKey);
  await page.waitForTimeout(100);

  const modifiedShot = await page.screenshot({ clip });
  await page.keyboard.up(modifierKey);

  const diffCount = diffPngPixels(baseShot, modifiedShot);
  const minDiff = Math.max(8, Math.floor((clip.width * clip.height) / 200));
  expect(diffCount).toBeGreaterThan(minDiff);
});

async function openTerminalPanel(page: any) {
  const terminalToggle = page.getByRole("button", { name: "Toggle terminal panel" }).first();
  await expect(terminalToggle).toBeVisible({ timeout: 20_000 });
  const panel = page.locator(".wb-terminal-panel-inner");
  if (!(await panel.isVisible())) {
    await terminalToggle.click();
  }
  await expect(panel).toBeVisible({ timeout: 20_000 });
  await panel
    .getByRole("button", { name: "Workspace" })
    .click()
    .catch(() => {});
}

async function ensureTerminalVisible(page: any) {
  const panel = page.locator(".wb-terminal-panel-inner");
  const xterm = page.locator(".xterm");
  if (await xterm.count()) {
    await expect(xterm.first()).toBeVisible({ timeout: 30_000 });
    return;
  }
  const openWorktree = page.getByRole("button", { name: "Open worktree terminal" });
  if (await openWorktree.isEnabled()) {
    await openWorktree.click();
  } else {
    await panel.getByRole("button", { name: "New terminal" }).click();
    const tabs = panel.locator(".wb-terminal-tab");
    await expect(tabs.first()).toBeVisible({ timeout: 20_000 });
    await tabs.first().click();
  }
  await expect(xterm.first()).toBeVisible({ timeout: 30_000 });
}

async function waitForWorkspaceTerminal(
  page: any,
  token: string,
  workspaceId: string,
): Promise<string> {
  const listTerminals = async () => {
    const res = await page.request.get(`/api/workspaces/${workspaceId}/terminals`, {
      headers: { authorization: `Bearer ${token}` },
    });
    if (!res.ok()) {
      throw new Error(`list terminals failed: HTTP ${res.status()} ${res.statusText()}`);
    }
    const data = (await res.json().catch(() => [])) as any[];
    return data.map((entry) => readId(entry?.id)).filter(Boolean);
  };

  await expect.poll(listTerminals, { timeout: 20_000 }).not.toHaveLength(0);
  const ids = await listTerminals();
  return ids[ids.length - 1];
}

function terminalWsUrl(baseURL: string, token: string, terminalId: string): string {
  const url = new URL(`/api/terminals/${terminalId}/stream`, baseURL);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.searchParams.set("token", token);
  return url.toString();
}

async function seedTerminalOutput(baseURL: string, token: string, terminalId: string, link: string) {
  const wsUrl = terminalWsUrl(baseURL, token, terminalId);
  const linkLine = JSON.stringify(`${link}\n`);
  const command = [
    "export PS1='$ ' PROMPT_COMMAND=''",
    "printf '\\033[2J\\033[H'",
    `printf ${linkLine}`,
    `echo ${TERMINAL_DONE}`,
  ].join(" && ");

  await new Promise<void>((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    const deadline = setTimeout(() => {
      try {
        ws.close();
      } catch {}
      reject(new Error("Timed out seeding terminal output"));
    }, 20_000);

    const finish = () => {
      clearTimeout(deadline);
      try {
        ws.close();
      } catch {}
      resolve();
    };

    ws.on("open", () => {
      ws.send(JSON.stringify({ kind: "resize", cols: 120, rows: 30 }));
      ws.send(JSON.stringify({ kind: "input", data: `${command}\n` }));
    });

    ws.on("message", (data) => {
      const buf = typeof data === "string" ? Buffer.from(data) : Buffer.from(data as any);
      if (buf.includes(Buffer.from(TERMINAL_DONE))) {
        finish();
      }
    });

    ws.on("error", (err) => {
      clearTimeout(deadline);
      reject(err);
    });
  });
}

async function getTerminalMetrics(page: any): Promise<{
  left: number;
  top: number;
  cellWidth: number;
  cellHeight: number;
}> {
  await page.waitForFunction(() => {
    const term = document.querySelector(".wb-terminal-panel-inner .xterm");
    const screen = term?.querySelector(".xterm-screen");
    const measure = term?.querySelector(".xterm-char-measure-element");
    if (!term || !screen || !measure) return false;
    const screenRect = screen.getBoundingClientRect();
    const measureRect = measure.getBoundingClientRect();
    return (
      screenRect.width > 0 &&
      screenRect.height > 0 &&
      measureRect.width > 0 &&
      measureRect.height > 0
    );
  });

  const metrics = await page.evaluate(() => {
    const term = document.querySelector(".wb-terminal-panel-inner .xterm");
    const screen = term?.querySelector(".xterm-screen");
    const measure = term?.querySelector(".xterm-char-measure-element");
    if (!term || !screen || !measure) return null;
    const screenRect = screen.getBoundingClientRect();
    const measureRect = measure.getBoundingClientRect();
    return {
      left: screenRect.left,
      top: screenRect.top,
      cellWidth: measureRect.width,
      cellHeight: measureRect.height,
    };
  });

  if (!metrics) {
    throw new Error("Failed to read terminal metrics");
  }
  return metrics;
}

function readId(value: any): string {
  if (value === null || value === undefined) return "";
  if (typeof value !== "string") {
    throw new Error("Expected id to be a string");
  }
  return value;
}

function diffPngPixels(baseShot: Buffer, modifiedShot: Buffer): number {
  const basePng = PNG.sync.read(baseShot);
  const modifiedPng = PNG.sync.read(modifiedShot);
  if (basePng.width !== modifiedPng.width || basePng.height !== modifiedPng.height) {
    throw new Error(
      `Screenshot mismatch (${basePng.width}x${basePng.height}) vs (${modifiedPng.width}x${modifiedPng.height})`,
    );
  }
  const diff = new PNG({ width: basePng.width, height: basePng.height });
  return pixelmatch(basePng.data, modifiedPng.data, diff.data, basePng.width, basePng.height, {
    threshold: 0.1,
  });
}
