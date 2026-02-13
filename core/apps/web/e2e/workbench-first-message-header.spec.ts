import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";
import { clearDiagnostics, expectNoUnexpectedDiagnostics, getDiagnostics } from "./utils/diagnostics";
import { expectWsPathOnCanonicalOrigin } from "./utils/wsUrls";

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

type E2EWindow = Window & {
  __ctxE2E?: {
    workspaceStream?: {
      getConnectionState?: () => string | null;
    };
  };
};

test("workbench: first user message renders from stream when head is stale", async ({ page, request }) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1400, height: 900 });

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
  });
  const sessionId = seed.sessionIdsByTask[seed.taskIds[0]][0];
  const prompt = `first-message-${Date.now()}`;
  let forceStaleHead = true;

  await page.route("**/api/sessions/*/snapshot**", async (route) => {
    const url = route.request().url();
    if (!url.includes(sessionId)) {
      await route.continue();
      return;
    }

    const response = await route.fetch();
    if (!forceStaleHead) {
      await route.fulfill({ response });
      return;
    }

    const snapshot = asRecord(await response.json());
    const snapshotHead = asRecord(snapshot.head);
    const staleHead = {
      ...snapshotHead,
      turns: [],
      messages: [],
      events: [],
      tool_summaries: [],
      has_more_turns: false,
      last_event_seq: 0,
    };
    await route.fulfill({
      response,
      body: JSON.stringify({ ...snapshot, head: staleHead }),
    });
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1);
  await rows.first().click();
  await page.waitForTimeout(400);

  const composer = page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea");
  await expect(composer).toBeVisible({ timeout: 20000 });

  await expect
    .poll(async () =>
      page.evaluate(() => typeof (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState === "function"),
    )
    .toBe(true);

  await expect
    .poll(async () => page.evaluate(() => (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState?.()))
    .toBe("connected");
  await clearDiagnostics(page);

  await composer.fill(prompt);
  await page.locator(".wb-session-slot[aria-hidden=\"false\"] button[aria-label=\"Send\"]").click();

  const header = page.locator(".wb-turn-header-content").filter({ hasText: prompt });
  await expect(header).toBeVisible({ timeout: 20000 });
  await expectWsPathOnCanonicalOrigin(page, "/api/workspaces/");
  await expectNoUnexpectedDiagnostics(page);
  const streamWarnings = (await getDiagnostics(page)).filter((event) =>
    ["workspace.stream_connect_failed", "workspace.stream_connection_missing"].includes(event.code),
  );
  expect(streamWarnings).toEqual([]);

  forceStaleHead = false;
});
