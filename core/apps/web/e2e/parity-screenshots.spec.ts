import { test, expect, chromium, webkit } from "playwright/test";
import { mkdirSync, rmSync, writeFileSync, readFileSync } from "fs";
import { mkdir } from "fs/promises";
import { execSync } from "child_process";
import path from "path";

const OUT_DIR = process.env.CTX_PARITY_OUT_DIR ?? "/tmp/ctx-parity";
const REPO_DIR = process.env.CTX_PARITY_REPO_DIR ?? "/tmp/ctx-parity-repo";
const WORKSPACE_NAME = process.env.CTX_PARITY_WORKSPACE_NAME ?? "ws-parity";
const DATA_DIR = process.env.CTX_E2E_DATA_DIR ?? "";
const AUTH_FILENAME = "daemon_auth.json";

async function ensureDir(p: string) {
  await mkdir(p, { recursive: true });
}

function ensureRepo(repo: string) {
  rmSync(repo, { recursive: true, force: true });
  mkdirSync(repo, { recursive: true });
  execSync("git init", { cwd: repo, stdio: "ignore" });
  execSync("git config user.email test@example.com", { cwd: repo, stdio: "ignore" });
  execSync("git config user.name Test", { cwd: repo, stdio: "ignore" });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo, stdio: "ignore" });
  execSync("git commit -m init", { cwd: repo, stdio: "ignore" });
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

  // Normalize incidental hover/focus state unless the screenshot is explicitly
  // meant to capture it.
  if (normalizeHover) {
    await page.mouse.move(1270, 710).catch(() => {});
  }
  if (normalizeFocus) {
    await page.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      el?.blur?.();
    }).catch(() => {});
  }
  await page.screenshot({
    path: path.join(dir, `${variant}.png`),
    type: "png",
    animations: "disabled",
    caret: "hide",
  });
}

async function closeWorkbenchMenus(page: any) {
  // Try a few times because menus can have focus traps and/or consume Escape.
  for (let i = 0; i < 3; i++) {
    await page.keyboard.press("Escape").catch(() => {});
    await page.waitForTimeout(50).catch(() => {});
  }
  const menuSelectors = [
    ".wb-harness-menu",
    ".wb-model-menu",
    ".wb-task-menu",
    ".wb-convo-menu",
  ];
  await Promise.all(
    menuSelectors.map((sel) => page.locator(sel).waitFor({ state: "hidden", timeout: 2_000 }).catch(() => {})),
  );
}

test("parity screenshots: launcher, new task, active session, settings", async () => {
  ensureRepo(REPO_DIR);
  const token = await readAuthToken();

  const chromiumBrowser = await chromium.launch();
  const webkitBrowser = await webkit.launch();
  const chromiumPage = await chromiumBrowser.newPage({
    viewport: { width: 1280, height: 720 },
    deviceScaleFactor: 1,
    reducedMotion: "reduce",
  });
  const webkitPage = await webkitBrowser.newPage({
    viewport: { width: 1280, height: 720 },
    deviceScaleFactor: 1,
    reducedMotion: "reduce",
  });
  await webkitPage.addInitScript(() => {
    // Simulate native desktop chrome (topbar should be absent in desktop).
    // @ts-ignore
    window.__CTX_DESKTOP_UI__ = true;
  });

  try {
    // Launcher (ensure workspace exists so both screenshots have identical state).
    await chromiumPage.goto(`/workspaces?token=${encodeURIComponent(token)}`);
    await expect(chromiumPage.getByRole("button", { name: "Add workspace" })).toBeVisible({ timeout: 20_000 });

    const chromiumWorkspaceLink = chromiumPage
      .getByRole("listitem")
      .filter({ hasText: REPO_DIR })
      .getByRole("link", { name: WORKSPACE_NAME });

    if ((await chromiumWorkspaceLink.count()) === 0) {
      await chromiumPage.getByLabel("Root path").fill(REPO_DIR);
      await chromiumPage.getByLabel("Name (optional)").fill(WORKSPACE_NAME);
      await chromiumPage.getByRole("button", { name: "Add workspace" }).click();
    }

    await expect(chromiumWorkspaceLink.first()).toBeVisible({ timeout: 20_000 });

    await webkitPage.goto(`/workspaces?token=${encodeURIComponent(token)}`);
    await expect(webkitPage.getByRole("button", { name: "Add workspace" })).toBeVisible({ timeout: 20_000 });
    const webkitWorkspaceLink = webkitPage
      .getByRole("listitem")
      .filter({ hasText: REPO_DIR })
      .getByRole("link", { name: WORKSPACE_NAME });
    await expect(webkitWorkspaceLink.first()).toBeVisible({ timeout: 20_000 });

    await takeShot(chromiumPage, "launcher", "web");
    await takeShot(webkitPage, "launcher", "native");

    // Launcher focus state.
    await chromiumPage.getByLabel("Root path").click();
    await webkitPage.getByLabel("Root path").click();
    await takeShot(chromiumPage, "launcher/focus/root-path", "web");
    await takeShot(webkitPage, "launcher/focus/root-path", "native");

    // Enter workbench (new task).
    await chromiumWorkspaceLink.first().click();
    await expect(chromiumPage).toHaveURL(/\/workspaces\/[^/?#]+$/, { timeout: 20_000 });
    const url = new URL(chromiumPage.url());
    const match = url.pathname.match(/\/workspaces\/([^/]+)$/);
    if (!match) throw new Error(`Failed to parse workspace id from URL: ${chromiumPage.url()}`);
    const workspaceId = match[1];

    await webkitPage.goto(`/workspaces/${workspaceId}`);

    await expect(chromiumPage.locator(".wb-main")).toBeVisible({ timeout: 20_000 });
    await expect(chromiumPage.locator(".wb-new-composer-stack textarea.wb-composer-textarea")).toBeVisible({
      timeout: 20_000,
    });
    await expect(chromiumPage.getByText("Archived")).toBeVisible({ timeout: 20_000 });

    await expect(webkitPage.locator(".wb-main")).toBeVisible({ timeout: 20_000 });
    await expect(webkitPage.locator(".wb-new-composer-stack textarea.wb-composer-textarea")).toBeVisible({
      timeout: 20_000,
    });
    await expect(webkitPage.getByText("Archived")).toBeVisible({ timeout: 20_000 });

    await takeShot(chromiumPage, "new-task", "web");
    await takeShot(webkitPage, "new-task", "native");

    // New task focus state.
    await chromiumPage.locator(".wb-new-composer-stack textarea.wb-composer-textarea").click();
    await webkitPage.locator(".wb-new-composer-stack textarea.wb-composer-textarea").click();
    await takeShot(chromiumPage, "new-task/focus/composer", "web");
    await takeShot(webkitPage, "new-task/focus/composer", "native");

    // Composer menus.
    await chromiumPage.locator(".wb-new-composer-stack").getByTitle("Harness").click();
    await webkitPage.locator(".wb-new-composer-stack").getByTitle("Harness").click();
    await expect(chromiumPage.locator(".wb-harness-menu")).toBeVisible({ timeout: 20_000 });
    await expect(webkitPage.locator(".wb-harness-menu")).toBeVisible({ timeout: 20_000 });
    await takeShot(chromiumPage, "new-task/menu/harness", "web");
    await takeShot(webkitPage, "new-task/menu/harness", "native");

    // Close the harness menu by toggling the trigger (Escape is not reliable in all engines).
    await chromiumPage.locator(".wb-new-composer-stack button[title=\"Harness\"]").click();
    await webkitPage.locator(".wb-new-composer-stack button[title=\"Harness\"]").click();
    await chromiumPage.locator(".wb-harness-menu").waitFor({ state: "hidden", timeout: 5_000 }).catch(() => {});
    await webkitPage.locator(".wb-harness-menu").waitFor({ state: "hidden", timeout: 5_000 }).catch(() => {});

    await chromiumPage.locator(".wb-new-composer-stack button[title=\"Model\"]").click();
    await webkitPage.locator(".wb-new-composer-stack button[title=\"Model\"]").click();
    await takeShot(chromiumPage, "new-task/menu/model", "web");
    await takeShot(webkitPage, "new-task/menu/model", "native");
    await chromiumPage.locator(".wb-new-composer-stack button[title=\"Model\"]").click();
    await webkitPage.locator(".wb-new-composer-stack button[title=\"Model\"]").click();
    await chromiumPage.locator(".wb-model-menu").waitFor({ state: "hidden", timeout: 5_000 }).catch(() => {});
    await webkitPage.locator(".wb-model-menu").waitFor({ state: "hidden", timeout: 5_000 }).catch(() => {});

    await chromiumPage.locator(".wb-new-composer-stack button[title=\"Mode\"]").click();
    await webkitPage.locator(".wb-new-composer-stack button[title=\"Mode\"]").click();
    await takeShot(chromiumPage, "new-task/menu/mode", "web");
    await takeShot(webkitPage, "new-task/menu/mode", "native");
    await chromiumPage.locator(".wb-new-composer-stack button[title=\"Mode\"]").click();
    await webkitPage.locator(".wb-new-composer-stack button[title=\"Mode\"]").click();
    await closeWorkbenchMenus(chromiumPage);
    await closeWorkbenchMenus(webkitPage);

    // Active session (create once in Chromium, then open in WebKit).
    await chromiumPage.locator(".wb-new-composer-stack").getByTitle("Harness").click();
    await chromiumPage.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
    await chromiumPage.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();
    await chromiumPage.locator(".wb-new-composer-stack button[title=\"Harness\"]").click();
    await chromiumPage.locator(".wb-harness-menu").waitFor({ state: "hidden", timeout: 5_000 }).catch(() => {});

    await chromiumPage.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill("hello");
    await chromiumPage.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();

    const chromiumRows = chromiumPage.locator(".wb-task-row");
    await expect(chromiumRows).toHaveCount(1, { timeout: 20_000 });
    await chromiumRows.first().click();
    await expect(chromiumPage.locator(".wb-session textarea.wb-active-textarea")).toBeVisible({ timeout: 20_000 });
    await expect(chromiumPage.locator(".wb-session .wb-assistant-entry")).toHaveCount(1, { timeout: 20_000 });

    const webkitRows = webkitPage.locator(".wb-task-row").filter({ hasText: "hello" });
    await expect(webkitRows).toHaveCount(1, { timeout: 20_000 });
    await webkitRows.first().click();
    await expect(webkitPage.locator(".wb-session textarea.wb-active-textarea")).toBeVisible({ timeout: 20_000 });
    await expect(webkitPage.locator(".wb-session .wb-assistant-entry")).toHaveCount(1, { timeout: 20_000 });

    await takeShot(chromiumPage, "active-session", "web");
    await takeShot(webkitPage, "active-session", "native");

    // Task row menu (more actions).
    const chromiumTaskRow = chromiumPage.locator(".wb-task-row").first();
    const webkitTaskRow = webkitPage.locator(".wb-task-row").first();
    await chromiumTaskRow.hover();
    await webkitTaskRow.hover();
    await chromiumTaskRow.locator("button[aria-label=\"More actions\"]").click();
    await webkitTaskRow.locator("button[aria-label=\"More actions\"]").click();
    await expect(chromiumPage.locator(".wb-task-menu")).toBeVisible({ timeout: 20_000 });
    await expect(webkitPage.locator(".wb-task-menu")).toBeVisible({ timeout: 20_000 });
    await takeShot(chromiumPage, "active-session/menu/task-actions", "web");
    await takeShot(webkitPage, "active-session/menu/task-actions", "native");
    await closeWorkbenchMenus(chromiumPage);
    await closeWorkbenchMenus(webkitPage);

    // Settings (pick a mostly-static section).
    await chromiumPage.goto("/settings#context_pack");
    await webkitPage.goto("/settings#context_pack");

    await expect(chromiumPage.locator(".settings-root")).toBeVisible({ timeout: 20_000 });
    await expect(chromiumPage.locator(".settings-main-title")).toHaveText("ctx pack", { timeout: 20_000 });
    await expect(chromiumPage.locator(".settings-empty")).toBeHidden({ timeout: 20_000 });

    await expect(webkitPage.locator(".settings-root")).toBeVisible({ timeout: 20_000 });
    await expect(webkitPage.locator(".settings-main-title")).toHaveText("ctx pack", { timeout: 20_000 });
    await expect(webkitPage.locator(".settings-empty")).toBeHidden({ timeout: 20_000 });

    await takeShot(chromiumPage, "settings", "web");
    await takeShot(webkitPage, "settings", "native");

    // Settings focus state.
    await chromiumPage.locator(".settings-search-input").click();
    await webkitPage.locator(".settings-search-input").click();
    await takeShot(chromiumPage, "settings/focus/search", "web");
    await takeShot(webkitPage, "settings/focus/search", "native");
  } finally {
    await chromiumBrowser.close();
    await webkitBrowser.close();
  }
});
