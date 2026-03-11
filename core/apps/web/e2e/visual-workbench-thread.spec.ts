import { test, expect } from "./fixtures";
import type { Page, Response } from "@playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";
import { createTempGitRepo } from "./utils/testRepo";
import { seedVisualSessionHead, type VisualTurnFixture } from "./utils/visualSessionHeads";
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
  await expect(page.locator('.wb-session-slot[aria-hidden="false"] button[aria-label="Stop"]')).toBeVisible({
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
  await page.locator('.wb-session-slot[aria-hidden="false"] button[aria-label="Send"]').click();
  const response = await sendResponse;
  expect(response.ok()).toBeTruthy();
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
      turnsPerSession: 0,
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
      turnsPerSession: 0,
      throttleMs: 0,
    });
    denseSeed = {
      workspaceId: dense.workspaceId,
      taskId: dense.taskIds[0] ?? "",
      sessionId: dense.sessionIdsByTask[dense.taskIds[0] ?? ""]?.[0] ?? "",
    };
  });

  const transcriptTurns: VisualTurnFixture[] = [
    {
      turnId: "visual-transcript-turn-1",
      userContent: "Summarize the current changes in the worktree.",
      assistantContent:
        "I reviewed the updated worktree, grouped the changes by area, and listed the tests that still need to run before the branch is ready.",
    },
    {
      turnId: "visual-transcript-turn-2",
      userContent: "What is still risky about this rollout?",
      assistantContent:
        "The remaining risk is visual drift on narrow layouts, especially where long tool titles and dense banners share the same vertical space.",
      toolSummaries: [
        {
          toolCallId: "visual-transcript-tool-search",
          title: "Search worktree",
          kind: "search",
          inputPreview: { query: "narrow layout regressions" },
          outputPreview: "Found 4 references to compact layout handling.",
        },
      ],
    },
    {
      turnId: "visual-transcript-turn-3",
      userContent: "Give me the exact follow-up plan.",
      assistantContent:
        "Next I will finish the remaining captures, review each diff manually, and flag any surface that looks visually off before asking for approval.",
    },
  ];

  const denseTurns: VisualTurnFixture[] = [
    {
      turnId: "visual-dense-turn-1",
      userContent: "Audit the workspace setup wizard for rough edges.",
      assistantContent:
        "I found two areas to tighten: the location step auto-advances abruptly, and the merge queue screen needs more breathing room once advanced fields are open.",
      toolSummaries: [
        {
          toolCallId: "visual-dense-tool-1",
          title: "Read wizard flow",
          kind: "read",
          inputPreview: { path: "src/pages/workspaceSetup" },
          outputPreview: "Inspected setup flow components and routing helpers.",
        },
        {
          toolCallId: "visual-dense-tool-2",
          title: "Check selectors",
          kind: "search",
          inputPreview: { query: "wizard-option-source-import" },
          outputPreview: "Located stable test ids for source and merge screens.",
        },
      ],
    },
    {
      turnId: "visual-dense-turn-2",
      userContent: "Show me the command output that matters.",
      assistantContent:
        "The targeted visual run passed for the settings and diff surfaces, while the thread captures needed deterministic seeded heads instead of live fake-harness completions.",
      toolSummaries: [
        {
          toolCallId: "visual-dense-tool-3",
          title: "Run targeted visual suite",
          kind: "execute",
          inputPreview: { command: "pnpm -C core/apps/web test:e2e:visual" },
          outputPreview: "Settings and diff surfaces passed; thread seeding required revision.",
        },
        {
          toolCallId: "visual-dense-tool-4",
          title: "Inspect traces",
          kind: "read",
          inputPreview: { path: "e2e/test-results/visual" },
          outputPreview: "Reviewed failing traces for wizard and thread states.",
        },
      ],
    },
    {
      turnId: "visual-dense-turn-3",
      userContent: "What needs manual design review?",
      assistantContent:
        "The merge queue advanced form, updater messaging, and any long assistant transcript on desktop-tight widths should all be reviewed by eye before approval.",
      toolSummaries: [
        {
          toolCallId: "visual-dense-tool-5",
          title: "Summarize review queue",
          kind: "execute",
          inputPreview: { command: "ls argos-screenshots" },
          outputPreview: "Collected screenshot artifacts for manual review.",
        },
      ],
    },
  ];

  for (const theme of THEMES) {
    for (const viewport of TRANSCRIPT_VIEWPORTS) {
      test(`transcript ${theme} ${viewport}`, async ({ page }) => {
        await openWorkbenchVisualPage(page, transcriptSeed.workspaceId, { theme, viewport });
        await openFirstTaskSession(page);
        await seedVisualSessionHead(page, {
          workspaceId: transcriptSeed.workspaceId,
          taskId: transcriptSeed.taskId,
          sessionId: transcriptSeed.sessionId,
          turns: transcriptTurns,
        });
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
      await seedVisualSessionHead(page, {
        workspaceId: denseSeed.workspaceId,
        taskId: denseSeed.taskId,
        sessionId: denseSeed.sessionId,
        turns: denseTurns,
      });
      await expect
        .poll(async () => page.locator(".wb-tool-row").count(), { timeout: 20_000 })
        .toBeGreaterThan(0);
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
