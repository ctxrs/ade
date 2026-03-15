import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";
import {
  buildVisualName,
  captureVisual,
  visualViewportLabel,
  type VisualTheme,
  type VisualViewportName,
} from "./utils/visual";
import {
  newTaskComposer,
  openHarnessMenu,
  openFirstTaskSession,
  openWorkbenchVisualPage,
  selectFakeHarness,
} from "./utils/visualWorkbench";

const THEMES = ["dark", "light"] as const satisfies VisualTheme[];
const EMPTY_VIEWPORTS = ["desktop", "narrow"] as const satisfies VisualViewportName[];

test.describe.serial("visual: workbench shell", () => {
  test.describe.configure({ timeout: 180_000 });
  let emptyWorkspaceId = "";
  let archivedWorkspaceId = "";
  let activeWorkspaceId = "";
  let activeSessionId = "";

  test.beforeAll(async ({ request }) => {
    test.setTimeout(180_000);
    const empty = await seedDummyWorkspace(request, {
      tasks: 0,
      sessionsPerTask: 0,
      turnsPerSession: 0,
    });
    emptyWorkspaceId = empty.workspaceId;

    const archived = await seedDummyWorkspace(request, {
      tasks: 12,
      sessionsPerTask: 0,
      turnsPerSession: 0,
      throttleMs: 0,
    });
    archivedWorkspaceId = archived.workspaceId;
    for (const taskId of archived.taskIds.slice(-8)) {
      const response = await request.post(`/api/tasks/${taskId}/archive`, {});
      expect(response.ok()).toBeTruthy();
    }

    const active = await seedDummyWorkspace(request, {
      tasks: 1,
      sessionsPerTask: 1,
      turnsPerSession: 1,
      throttleMs: 0,
    });
    activeWorkspaceId = active.workspaceId;
    activeSessionId = active.sessionIdsByTask[active.taskIds[0] ?? ""]?.[0] ?? "";
  });

  const seedActiveSession = async (page: Page) => {
    await openFirstTaskSession(page);
    await expect
      .poll(async () => page.locator(".wb-turn-header-content").count(), { timeout: 20_000 })
      .toBeGreaterThan(0);
  };

  const attachComposerImage = async (page: Page) => {
    await newTaskComposer(page).evaluate((el) => {
      const base64Png =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+lmZYAAAAASUVORK5CYII=";
      const bytes = Uint8Array.from(atob(base64Png), (c) => c.charCodeAt(0));
      const file = new File([bytes], "drop.png", { type: "image/png" });
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(file);

      const rect = el.getBoundingClientRect();
      const clientX = Math.floor(rect.left + rect.width / 2);
      const clientY = Math.floor(rect.top + rect.height / 2);

      el.dispatchEvent(
        new DragEvent("dragover", {
          bubbles: true,
          cancelable: true,
          clientX,
          clientY,
          dataTransfer,
        }),
      );
      el.dispatchEvent(
        new DragEvent("drop", {
          bubbles: true,
          cancelable: true,
          clientX,
          clientY,
          dataTransfer,
        }),
      );
    });
  };

  for (const theme of THEMES) {
    for (const viewport of EMPTY_VIEWPORTS) {
      test(`empty workbench ${theme} ${viewport}`, async ({ page }) => {
        await openWorkbenchVisualPage(page, emptyWorkspaceId, { theme, viewport });
        await expect(newTaskComposer(page)).toBeVisible({ timeout: 20_000 });
        await captureVisual(
          page,
          buildVisualName(["workbench-shell", "empty", theme, visualViewportLabel(viewport)]),
        );
      });
    }

    test(`harness menu ${theme}`, async ({ page }) => {
      await openWorkbenchVisualPage(page, emptyWorkspaceId, { theme, viewport: "desktop-tight" });
      const menu = await openHarnessMenu(page);
      await captureVisual(
        page,
        buildVisualName(["workbench-shell", "harness-menu-open", theme, visualViewportLabel("desktop-tight")]),
        { ready: menu },
      );
    });

    test(`slash commands ${theme}`, async ({ page }) => {
      await openWorkbenchVisualPage(page, emptyWorkspaceId, { theme, viewport: "narrow" });
      const composer = newTaskComposer(page);
      await composer.fill("/");
      const autocomplete = page.locator(".composer-ac");
      await expect(autocomplete).toBeVisible({ timeout: 20_000 });
      await captureVisual(
        page,
        buildVisualName(["workbench-shell", "slash-commands-open", theme, visualViewportLabel("narrow")]),
        { ready: autocomplete },
      );
    });

    test(`archived tasks ${theme}`, async ({ page }) => {
      await openWorkbenchVisualPage(page, archivedWorkspaceId, { theme, viewport: "desktop-tight" });
      await page.getByRole("button", { name: "Archived Tasks" }).click();
      const archivedRows = page.getByRole("listitem").filter({ hasText: /fixture task/i }).first();
      await expect(archivedRows).toBeVisible({ timeout: 20_000 });
      await captureVisual(
        page,
        buildVisualName(["workbench-shell", "archived-open", theme, visualViewportLabel("desktop-tight")]),
        { ready: archivedRows },
      );
    });

    test(`mixed task list ${theme}`, async ({ page, request }) => {
      const mixed = await seedDummyWorkspace(request, {
        tasks: 3,
        sessionsPerTask: 1,
        turnsPerSession: 1,
        throttleMs: 0,
      });
      await openWorkbenchVisualPage(page, mixed.workspaceId, { theme, viewport: "desktop" });
      await expect(page.locator(".wb-task-row")).toHaveCount(3, { timeout: 20_000 });
      await selectFakeHarness(page);
      const composer = newTaskComposer(page);
      await composer.fill(`visual-shell-running-${theme}`);
      await page.getByRole("button", { name: "Send" }).click();
      await expect(page.locator('.wb-session-slot button[aria-label="Stop"]')).toBeVisible({
        timeout: 20_000,
      });
      await expect(page.locator(".wb-task-row")).toHaveCount(4, { timeout: 20_000 });
      await captureVisual(
        page,
        buildVisualName(["workbench-shell", "mixed-task-list", theme, visualViewportLabel("desktop")]),
      );
    });

    test(`composer image attachment ${theme}`, async ({ page }) => {
      await openWorkbenchVisualPage(page, activeWorkspaceId, { theme, viewport: "desktop-tight" });
      await openFirstTaskSession(page);
      await page.getByRole("button", { name: "New Task" }).click();
      await expect(newTaskComposer(page)).toBeVisible({ timeout: 20_000 });
      await attachComposerImage(page);
      const attachments = page.locator(".wb-composer-attachments");
      await expect(attachments.locator(".wb-attach-thumb-img")).toHaveCount(1, { timeout: 20_000 });
      await captureVisual(
        page,
        buildVisualName(["workbench-shell", "composer-image-attachment", theme, visualViewportLabel("desktop-tight")]),
        { ready: attachments },
      );
    });

    test(`terminal panel ${theme}`, async ({ page }) => {
      await openWorkbenchVisualPage(page, activeWorkspaceId, { theme, viewport: "desktop-tight" });
      await seedActiveSession(page);
      await page.getByRole("button", { name: "Toggle terminal panel" }).click();
      const terminalPane = page.locator(".wb-terminal-panel-inner");
      await expect(terminalPane).toBeVisible({ timeout: 20_000 });
      await captureVisual(
        page,
        buildVisualName(["workbench-shell", "terminal-panel-open", theme, visualViewportLabel("desktop-tight")]),
        { ready: terminalPane },
      );
    });

    test(`artifacts pane ${theme}`, async ({ page }) => {
      await page.route(`**/api/sessions/${activeSessionId}/artifacts`, async (route) => {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify([]),
        });
      });
      await openWorkbenchVisualPage(page, activeWorkspaceId, { theme, viewport: "desktop-tight" });
      await seedActiveSession(page);
      await page.getByRole("button", { name: "Toggle artifacts" }).click();
      const artifactsPane = page.locator(".wb-artifacts");
      await expect(artifactsPane).toContainText("No artifacts yet.", { timeout: 20_000 });
      await captureVisual(
        page,
        buildVisualName(["workbench-shell", "artifacts-pane-open", theme, visualViewportLabel("desktop-tight")]),
        { ready: artifactsPane },
      );
    });
  }
});
