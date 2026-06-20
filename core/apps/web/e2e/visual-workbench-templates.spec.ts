import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import type { APIRequestContext, Page } from "@playwright/test";
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

const VISUAL_PLUGIN_ID = "visual.review-tools";

async function seedVisualWorkbenchPlugin(request: APIRequestContext) {
  const dataDir = process.env.CTX_E2E_DATA_DIR;
  if (!dataDir) {
    throw new Error("CTX_E2E_DATA_DIR is required for visual plugin seeding");
  }

  const pluginDir = path.join(dataDir, "plugins", "visual-review-tools");
  await mkdir(pluginDir, { recursive: true });
  await writeFile(
    path.join(pluginDir, "ctx-plugin.json"),
    JSON.stringify(
      {
        schema_version: 1,
        id: VISUAL_PLUGIN_ID,
        name: "Visual Review Tools With Very Long Labels",
        version: "0.1.0",
        contributes: {
          ui_surfaces: [
            {
              id: `${VISUAL_PLUGIN_ID}.review_panel`,
              name: "Review telemetry and contribution diagnostics panel",
              surface: "panel",
              contexts: ["workbench", "review"],
            },
          ],
          templates: [
            {
              id: `${VISUAL_PLUGIN_ID}.dense_review_template`,
              name: "Dense review evidence template with long title",
              title: "Dense Review Evidence",
              template: "review",
              contexts: ["workbench"],
              data_sources: ["agent_work", "plugin_registry"],
            },
          ],
          toolbar_actions: [
            {
              id: `${VISUAL_PLUGIN_ID}.focus_work`,
              name: "Focus current work item",
              title: "Focus Work",
              action: "work.focus",
              icon: "crosshair",
              contexts: ["workbench"],
            },
          ],
          artifact_renderers: [
            {
              id: `${VISUAL_PLUGIN_ID}.long_text_artifact`,
              name: "Long text artifact renderer",
              artifact_types: ["text/plain", "application/vnd.ctx.agent-work+json"],
              renderer: "host.text-artifact",
              contexts: ["workbench"],
            },
          ],
          card_renderers: [
            {
              id: `${VISUAL_PLUGIN_ID}.work_summary_card`,
              name: "Work summary card renderer",
              card: "work.summary",
              renderer: "host.work-summary-card",
              contexts: ["workbench"],
            },
          ],
          detail_sections: [
            {
              id: `${VISUAL_PLUGIN_ID}.work_summary_section`,
              name: "Work summary detail section",
              section: "work.summary",
              renderer: "host.work-summary-section",
              contexts: ["workbench"],
            },
          ],
          review_sections: [
            {
              id: `${VISUAL_PLUGIN_ID}.unsupported_custom_review_section`,
              name: "Unsupported custom review section with deliberately long source label",
              section: "review.custom-untrusted",
              renderer: "plugin.visual-review-renderer",
              contexts: ["workbench", "review"],
            },
          ],
        },
      },
      null,
      2,
    ),
    "utf8",
  );

  const reload = await request.post("/api/plugins/reload");
  expect(reload.ok(), await reload.text()).toBeTruthy();
  const inventory = await reload.json();
  expect(inventory.plugins?.some((plugin: { id?: string; status?: string }) => (
    plugin.id === VISUAL_PLUGIN_ID && plugin.status === "loaded"
  ))).toBeTruthy();
}

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

async function expectNoHorizontalOverflow(locator: ReturnType<Page["locator"]>) {
  await expect(locator).toBeVisible({ timeout: 20_000 });
  const overflow = await locator.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(overflow.scrollWidth).toBeLessThanOrEqual(overflow.clientWidth + 2);
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

    await seedVisualWorkbenchPlugin(request);
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

  test("plugin contribution panel desktop-tight", async ({ page }) => {
    await openSeededTemplateState(page, {
      workspaceId: templateWorkspaceId,
      viewport: "desktop-tight",
      template: "Classic",
    });

    const panel = page.getByRole("region", { name: "Workbench contributions" });
    await expect(panel).toContainText("Host-owned projection only", { timeout: 20_000 });
    await expect(panel).toContainText("Visual Review Tools With Very Long Labels");
    await expect(panel).toContainText("Unsupported renderer: plugin.visual-review-renderer");
    await expectNoHorizontalOverflow(page.locator(".wb-main"));
    await expectNoHorizontalOverflow(page.locator(".wb-contribution-projection-list"));

    await captureVisual(
      page,
      buildVisualName(["workbench-contributions", "panel", "ready", THEME, "desktop-tight"]),
      { ready: panel },
    );
  });

  test("plugin contribution panel with kanban narrow layout", async ({ page }) => {
    await openSeededTemplateState(page, {
      workspaceId: templateWorkspaceId,
      viewport: "narrow",
      template: "Kanban",
    });

    const panel = page.getByRole("region", { name: "Workbench contributions" });
    const detailPanel = page.locator(".wb-kanban-detail-panel");
    await expect(panel).toContainText("Visual Review Tools With Very Long Labels", { timeout: 20_000 });
    await expect(detailPanel).toBeVisible({ timeout: 20_000 });
    await expectNoHorizontalOverflow(page.locator(".wb-main"));
    await expectNoHorizontalOverflow(page.locator(".wb-contribution-projection-list"));

    const detailBox = await detailPanel.boundingBox();
    expect(detailBox?.height ?? 0).toBeGreaterThan(120);

    await captureVisual(
      page,
      buildVisualName(["workbench-contributions", "kanban", "narrow", THEME]),
      { ready: panel },
    );
  });
});
