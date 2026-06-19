import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";
import {
  buildVisualName,
  captureVisual,
  visualViewportLabel,
  waitForVisualSettled,
  type VisualTheme,
  type VisualViewportName,
} from "./utils/visual";
import {
  activeSessionComposer,
  openWorkbenchVisualPage,
} from "./utils/visualWorkbench";

type TemplateLabel = "Classic" | "Kanban" | "Multipane" | "Review";

const THEME: VisualTheme = "dark";
const TEMPLATE_LABELS = ["Classic", "Kanban", "Multipane", "Review"] as const satisfies TemplateLabel[];
const TEMPLATE_VIEWPORTS = ["desktop-wide", "laptop", "narrow"] as const satisfies VisualViewportName[];
const TEMPLATE_CLASS_BY_LABEL: Record<TemplateLabel, string> = {
  Classic: "classic",
  Kanban: "kanban",
  Multipane: "multipane",
  Review: "review",
};

async function selectTemplate(page: Page, label: TemplateLabel) {
  await page.getByRole("radio", { name: label, exact: true }).click();
  await expect(page.getByRole("radio", { name: label, exact: true })).toHaveAttribute("aria-checked", "true");
  await expect(page.locator(`.wb-main-template-${TEMPLATE_CLASS_BY_LABEL[label]}`)).toBeVisible({
    timeout: 20_000,
  });
}

function visibleFixtureTasks(page: Page) {
  return page.getByRole("listitem").filter({ hasText: /fixture task/i });
}

async function openFirstVisibleTaskSession(page: Page) {
  const rows = visibleFixtureTasks(page);
  await expect(rows.first()).toBeVisible({ timeout: 20_000 });
  await rows.first().click();
  await expect(activeSessionComposer(page)).toBeVisible({ timeout: 20_000 });
}

async function openSeededTemplateState(
  page: Page,
  opts: { workspaceId: string; viewport: VisualViewportName; template: TemplateLabel },
) {
  await openWorkbenchVisualPage(page, opts.workspaceId, { theme: THEME, viewport: opts.viewport });
  await openFirstVisibleTaskSession(page);
  await selectTemplate(page, opts.template);
  await waitForVisualSettled(page, {
    ready: page.locator(`.wb-main-template-${TEMPLATE_CLASS_BY_LABEL[opts.template]}`),
  });
}

test.describe.serial("visual: workbench templates", () => {
  test.describe.configure({ timeout: 180_000 });
  let templateWorkspaceId = "";
  let denseWorkspaceId = "";

  test.beforeAll(async ({ request }) => {
    test.setTimeout(180_000);
    const templateSeed = await seedDummyWorkspace(request, {
      tasks: 8,
      sessionsPerTask: 1,
      turnsPerSession: 3,
      throttleMs: 0,
      messagePrefix: "template visual fixture",
      messageBodyLines: { min: 2, max: 5 },
      includeToolSummaries: true,
      toolSummariesPerTurn: 2,
      seedTranscriptDirect: true,
      directSeedMaterializedTailTurns: 3,
    });
    templateWorkspaceId = templateSeed.workspaceId;

    const denseSeed = await seedDummyWorkspace(request, {
      tasks: 24,
      sessionsPerTask: 0,
      turnsPerSession: 0,
      throttleMs: 0,
      messagePrefix: "dense visual fixture",
      seedTranscriptDirect: true,
    });
    denseWorkspaceId = denseSeed.workspaceId;
  });

  for (const template of TEMPLATE_LABELS) {
    for (const viewport of TEMPLATE_VIEWPORTS) {
      test(`${template} template ${viewport}`, async ({ page }) => {
        await openSeededTemplateState(page, {
          workspaceId: templateWorkspaceId,
          viewport,
          template,
        });
        await captureVisual(
          page,
          buildVisualName([
            "workbench-template",
            TEMPLATE_CLASS_BY_LABEL[template],
            THEME,
            visualViewportLabel(viewport),
          ]),
        );
      });
    }
  }

  test("classic high-density task list desktop-wide", async ({ page }) => {
    await openWorkbenchVisualPage(page, denseWorkspaceId, { theme: THEME, viewport: "desktop-wide" });
    await expect
      .poll(async () => visibleFixtureTasks(page).count(), { timeout: 20_000 })
      .toBeGreaterThanOrEqual(16);
    await selectTemplate(page, "Classic");
    await captureVisual(
      page,
      buildVisualName(["workbench-template", "classic", "dense-task-list", THEME, visualViewportLabel("desktop-wide")]),
    );
  });

  test("multipane focus and resize sequence desktop-wide", async ({ page }) => {
    await openSeededTemplateState(page, {
      workspaceId: templateWorkspaceId,
      viewport: "desktop-wide",
      template: "Multipane",
    });

    await captureVisual(
      page,
      buildVisualName(["workbench-template", "multipane", "sequence-initial", THEME, "desktop-wide"]),
    );

    await page.getByRole("button", { name: "Split right" }).click();
    const panes = page.locator(".wb-split-pane");
    await expect(panes).toHaveCount(2, { timeout: 20_000 });
    await expect(page.getByRole("separator", { name: "Resize panes" })).toHaveCount(1, { timeout: 20_000 });
    await captureVisual(
      page,
      buildVisualName(["workbench-template", "multipane", "split-right-empty-focused", THEME, "desktop-wide"]),
    );

    await panes.first().click();
    await expect(panes.first()).toHaveClass(/wb-split-pane-active/, { timeout: 20_000 });
    await captureVisual(
      page,
      buildVisualName(["workbench-template", "multipane", "focus-primary-pane", THEME, "desktop-wide"]),
    );

    const resizeHandle = page.getByRole("separator", { name: "Resize panes" });
    await resizeHandle.focus();
    await resizeHandle.press("ArrowRight");
    await expect(resizeHandle).toHaveAttribute("aria-valuenow", "55", { timeout: 20_000 });
    await captureVisual(
      page,
      buildVisualName(["workbench-template", "multipane", "resized-right", THEME, "desktop-wide"]),
    );
  });
});
