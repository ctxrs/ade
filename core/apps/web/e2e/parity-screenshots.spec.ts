import { test, expect, chromium } from "playwright/test";
import { mkdirSync, rmSync, writeFileSync, readFileSync } from "fs";
import { mkdir } from "fs/promises";
import { execFileSync } from "child_process";
import path from "path";
import { fileURLToPath } from "url";

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

async function ensureDir(p: string) {
  await mkdir(p, { recursive: true });
}

function ensureRepo(repo: string) {
  rmSync(repo, { recursive: true, force: true });
  mkdirSync(repo, { recursive: true });
  execFileSync("git", ["init"], { cwd: repo, stdio: "ignore" });
  execFileSync("git", ["config", "user.email", "test@example.com"], {
    cwd: repo,
    stdio: "ignore",
  });
  execFileSync("git", ["config", "user.name", "Test"], {
    cwd: repo,
    stdio: "ignore",
  });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execFileSync("git", ["add", "."], { cwd: repo, stdio: "ignore" });
  execFileSync("git", ["commit", "-m", "init"], { cwd: repo, stdio: "ignore" });
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

  test.setTimeout(300_000);

  test("launcher, new task, active session, settings", async () => {
    rmSync(OUT_DIR, { recursive: true, force: true });
    mkdirSync(OUT_DIR, { recursive: true });
    ensureRepo(REPO_DIR);
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

      // Enter workbench (new task).
      console.log("[parity] web: enter workbench");
      await workspaceLink.first().click();
      await expect(page).toHaveURL(/\/workspaces\/[^/?#]+$/, { timeout: 20_000 });
      const workbenchUrl = new URL(page.url());
      workbenchUrl.searchParams.set("desktop_ui", "1");
      await page.goto(workbenchUrl.toString());
      await expect(page.locator(".wb-root-no-topbar")).toBeVisible({ timeout: 20_000 });

      // Workbench baseline.
      await expect(page.getByText("Archived")).toBeVisible({ timeout: 20_000 });

      // Create a task + session once in web so native sees identical backend state.
      console.log("[parity] web: select harness + send task");
      await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
      await expect(page.locator(".wb-harness-menu")).toBeVisible({ timeout: 20_000 });
      console.log("[parity] web: screenshot harness menu");
      await takeShot(page, "new-task/menu/harness", "web");
      await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
      await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
      await page.locator(".wb-new-composer-stack button[title=\"Harness\"]").click();
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
      await expect(page.getByText("No archived tasks.")).toBeVisible({ timeout: 20_000 });
      await takeShot(page, "archived/empty", "web");
      await page.locator(".wb-section-header-archived .wb-section-toggle").click();
      await expect(page.getByText("No archived tasks.")).toBeHidden({ timeout: 20_000 });

      console.log("[parity] web: open active session");
      await rows.first().click();
      await expect(page.locator(".wb-session textarea.wb-active-textarea").first()).toBeVisible({
        timeout: 20_000,
      });
      await expect(page.locator(".wb-session .wb-assistant-entry")).toHaveCount(1, {
        timeout: 20_000,
      });
      const assistantEntry = page.locator(".wb-session .wb-assistant-entry").first();
      await assistantEntry.scrollIntoViewIfNeeded();
      await expect(assistantEntry).toBeVisible({ timeout: 20_000 });
      await expect(assistantEntry).toContainText(/\S+/, { timeout: 20_000 });

      console.log("[parity] web: screenshot active-session");
      await takeShot(page, "active-session", "web");

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
      const baseURL = new URL(page.url()).origin;
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
            CARGO_TARGET_DIR: NATIVE_CARGO_TARGET_DIR,
          },
        },
      );
    } finally {
      await browser.close();
    }
  });
});
