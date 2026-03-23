import { test, expect } from "./fixtures";
import type { Page, Response } from "@playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";
import { createTempGitRepo } from "./utils/testRepo";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";
import {
  buildVisualName,
  captureVisual,
  setVisualTheme,
  waitForVisualSettled,
  visualViewportLabel,
  type VisualTheme,
  type VisualViewportName,
} from "./utils/visual";
import {
  activeSessionComposer,
  enableQueuedMessages,
  newTaskComposer,
  openFirstTaskSession,
  openWorkbenchVisualPage,
  selectFakeHarness,
} from "./utils/visualWorkbench";

const THEMES = ["dark", "light"] as const satisfies VisualTheme[];
const TRANSCRIPT_VIEWPORTS = ["desktop", "desktop-tight"] as const satisfies VisualViewportName[];

const toolMarkerFor = (seed: string) =>
  `[[tool_calls]]\n${JSON.stringify([
    {
      kind: "execute",
      title: `Run pwd ${seed}`,
      input: { command: "pwd" },
      output_text: "ok",
    },
  ])}\n[[/tool_calls]]`;

async function setupRunningSession(page: Page, theme: VisualTheme, opts: { queuedMessages?: boolean } = {}) {
  if (opts.queuedMessages) {
    await enableQueuedMessages(page);
  }
  await setVisualTheme(page, theme);
  const repo = createTempGitRepo({
    prefix: "ctx-e2e-visual-thread-",
    files: [{ path: "file.txt", content: "hello\n" }],
  });
  await createWorkspaceAndOpenWorkbench({
    page,
    request: page.request,
    repo,
    workspaceName: `ws-visual-thread-${Date.now()}`,
  });
  await setVisualTheme(page, theme);
  await selectFakeHarness(page);

  const prompt = `visual-thread-running
[[tool_calls]]
[
  {"kind":"execute","title":"t1","input":{"command":"echo 1"}},
  {"kind":"execute","title":"t2","input":{"command":"echo 2"}},
  {"kind":"execute","title":"t3","input":{"command":"echo 3"}},
  {"kind":"execute","title":"t4","input":{"command":"echo 4"}}
]
[[/tool_calls]]`;
  await newTaskComposer(page).fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();
  await expect(activeSessionComposer(page)).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.wb-session-slot button[aria-label="Stop"]')).toBeVisible({
    timeout: 20_000,
  });
  await waitForVisualSettled(page);
}

async function queueMessage(page: Page, text: string) {
  const composer = activeSessionComposer(page);
  await composer.fill(text);
  const sendResponse = page.waitForResponse((response: Response) => {
    if (response.request().method() !== "POST") return false;
    if (!/\/api\/sessions\/[^/]+\/messages$/.test(response.url())) return false;
    return (response.request().postData() ?? "").includes(text);
  });
  await page.locator('.wb-session-slot button[aria-label="Send"]').click();
  const response = await sendResponse;
  expect(response.ok()).toBeTruthy();
}

async function ensureToolRows(page: Page, sessionId: string) {
  const toolRows = page.locator(".wb-tool-row");
  if ((await toolRows.count()) > 0) {
    return toolRows;
  }

  const seed = `${Date.now()}`;
  const response = await page.request.post(`/api/sessions/${sessionId}/messages`, {
    data: {
      content: `visual dense tool seed ${seed}\n${toolMarkerFor(seed)}`,
      delivery: "immediate",
    },
  });
  expect(response.ok(), `tool seed POST failed: ${response.url()}`).toBeTruthy();
  await expect
    .poll(async () => toolRows.count(), { timeout: 30_000 })
    .toBeGreaterThan(0);
  return toolRows;
}

test.describe.serial("visual: workbench thread", () => {
  test.describe.configure({ timeout: 180_000 });
  let transcriptSeed = { workspaceId: "", taskId: "", sessionId: "" };
  let denseSeed = { workspaceId: "", taskId: "", sessionId: "" };

  test.beforeAll(async ({ request }) => {
    test.setTimeout(180_000);
    const transcript = await seedDummyWorkspace(request, {
      tasks: 1,
      sessionsPerTask: 1,
      turnsPerSession: 3,
      throttleMs: 0,
    });
    transcriptSeed = {
      workspaceId: transcript.workspaceId,
      taskId: transcript.taskIds[0] ?? "",
      sessionId: transcript.sessionIdsByTask[transcript.taskIds[0] ?? ""]?.[0] ?? "",
    };

    const dense = await seedDummyWorkspace(request, {
      tasks: 1,
      sessionsPerTask: 1,
      turnsPerSession: 3,
      throttleMs: 0,
      includeToolSummaries: true,
      toolSummariesPerTurn: 2,
    });
    denseSeed = {
      workspaceId: dense.workspaceId,
      taskId: dense.taskIds[0] ?? "",
      sessionId: dense.sessionIdsByTask[dense.taskIds[0] ?? ""]?.[0] ?? "",
    };
  });

  for (const theme of THEMES) {
    test(`tool rows use muted thread color ${theme}`, async ({ page }) => {
      await openWorkbenchVisualPage(page, denseSeed.workspaceId, { theme, viewport: "narrow" });
      await openFirstTaskSession(page);
      await ensureToolRows(page, denseSeed.sessionId);

      const toolVerb = page.locator(".wb-tool-row .wb-tool-verb").first();
      const toolRest = page.locator(".wb-tool-row .wb-tool-rest").first();
      await expect(toolVerb).toBeVisible();
      await expect(toolRest).toBeVisible();

      const mutedColor = await page.evaluate(() => {
        const probe = document.createElement("div");
        probe.style.color = "var(--muted)";
        document.body.appendChild(probe);
        const color = window.getComputedStyle(probe).color;
        probe.remove();
        return color;
      });

      await expect(toolVerb).toHaveCSS("color", mutedColor);
      await expect(toolRest).toHaveCSS("color", mutedColor);
    });

    test(`assistant rows use updated vertical padding ${theme}`, async ({ page }) => {
      await openWorkbenchVisualPage(page, transcriptSeed.workspaceId, { theme, viewport: "desktop" });
      await openFirstTaskSession(page);

      const assistantEntry = page.locator(".wb-assistant-entry").first();
      await expect(assistantEntry).toBeVisible();
      await expect(assistantEntry).toHaveCSS("padding-top", "10px");
      await expect(assistantEntry).toHaveCSS("padding-right", "2px");
      await expect(assistantEntry).toHaveCSS("padding-bottom", "10px");
      await expect(assistantEntry).toHaveCSS("padding-left", "2px");
    });

    for (const viewport of TRANSCRIPT_VIEWPORTS) {
      test(`transcript ${theme} ${viewport}`, async ({ page }) => {
        await openWorkbenchVisualPage(page, transcriptSeed.workspaceId, { theme, viewport });
        await openFirstTaskSession(page);
        await expect
          .poll(async () => page.locator(".wb-turn-header-content").count(), { timeout: 20_000 })
          .toBeGreaterThan(0);
        await expect
          .poll(async () => page.locator(".wb-assistant-entry").count(), { timeout: 20_000 })
          .toBeGreaterThan(0);
        await captureVisual(
          page,
          buildVisualName(["workbench-thread", "transcript", theme, visualViewportLabel(viewport)]),
        );
      });
    }

    test(`dense thread ${theme}`, async ({ page }) => {
      await openWorkbenchVisualPage(page, denseSeed.workspaceId, { theme, viewport: "narrow" });
      await openFirstTaskSession(page);
      await ensureToolRows(page, denseSeed.sessionId);
      await captureVisual(
        page,
        buildVisualName(["workbench-thread", "dense", theme, visualViewportLabel("narrow")]),
      );
    });

    test(`running ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 1400, height: 900 });
      await setupRunningSession(page, theme);
      await captureVisual(
        page,
        buildVisualName(["workbench-thread", "running", theme, visualViewportLabel("desktop")]),
      );
    });

    test(`queued ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 1400, height: 900 });
      await setupRunningSession(page, theme, { queuedMessages: true });
      const queuedText = `visual-queued-${theme}-${Date.now()}`;
      await queueMessage(page, queuedText);
      const queuePanel = page.locator(".wb-session .queue-panel");
      await expect(queuePanel).toBeVisible({ timeout: 20_000 });
      await expect(queuePanel).toContainText(queuedText, { timeout: 20_000 });
      await captureVisual(
        page,
        buildVisualName(["workbench-thread", "queued-panel", theme, visualViewportLabel("desktop")]),
        { ready: queuePanel },
      );
    });
  }
});
