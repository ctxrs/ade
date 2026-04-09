import { writeFile } from "node:fs/promises";
import type { Page, TestInfo } from "playwright/test";
import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const visibleSessionSelector = '.wb-session-slot [data-testid="session-view"]';
const visibleScrollerSelector = `${visibleSessionSelector} .wb-thread-scroller`;
const visibleStatusSelector = `${visibleSessionSelector} .wb-turn-status-label`;

type GeometrySnapshot = {
  overlaps: Array<{
    previousId: string;
    nextId: string;
    previousBottom: number;
    nextTop: number;
    previousText: string;
    nextText: string;
  }>;
  visibleRows: Array<{
    id: string;
    top: number;
    bottom: number;
    height: number;
    text: string;
    knownSize: string | null;
    dataIndex: string | null;
  }>;
  sessionId: string | null;
};

const longBody = Array.from({ length: 140 }, (_, index) => `overlap fixture line ${index + 1}`).join("\n");

const buildSlowPrompt = (marker: string, index: number) => {
  const toolCalls = Array.from({ length: 4 }, (_, toolIndex) => ({
    kind: "execute",
    title: `${marker} tool ${toolIndex + 1}`,
    input: { command: `printf '${marker}-${index}-${toolIndex + 1}'` },
    output_text: `${marker} output ${toolIndex + 1}`,
  }));
  return `slow-diff-test stream-assistant-partials emit-thought ${marker} ${index}
${longBody}
[[tool_calls]]
${JSON.stringify(toolCalls)}
[[/tool_calls]]`;
};

async function waitForVisibleSession(page: Page, sessionId: string) {
  await expect(page.locator(visibleSessionSelector).first()).toHaveAttribute("data-session-id", sessionId, {
    timeout: 20_000,
  });
  await expect(page.locator(visibleScrollerSelector).first()).toBeVisible({ timeout: 20_000 });
}

async function waitForVisiblePendingAssistantRow(page: Page) {
  const pendingAssistant = page
    .locator(`${visibleSessionSelector} [data-thread-item-id^="assistant-"][data-thread-item-id$="-pending"]`)
    .first();
  await expect(pendingAssistant).toBeVisible({ timeout: 20_000 });
}

async function readVisibleThreadGeometry(page: Page): Promise<GeometrySnapshot> {
  const sessionView = page.locator(visibleSessionSelector).first();
  return sessionView.evaluate((root) => {
    const scroller = root.querySelector(".wb-thread-scroller") as HTMLElement | null;
    const scrollerRect = scroller?.getBoundingClientRect() ?? null;
    const visibleRows = Array.from(
      root.querySelectorAll('.wb-thread-scroller [role="listitem"][data-thread-item-id]'),
    )
      .map((node) => {
        const el = node as HTMLElement;
        const rect = el.getBoundingClientRect();
        const parent = el.parentElement as HTMLElement | null;
        return {
          id: el.getAttribute("data-thread-item-id") ?? "",
          top: rect.top,
          bottom: rect.bottom,
          height: rect.height,
          text: (el.innerText || "").slice(0, 180),
          knownSize: parent?.getAttribute("data-known-size") ?? null,
          dataIndex: parent?.getAttribute("data-index") ?? null,
        };
      })
      .filter((row) => {
        if (!scrollerRect) return row.height > 1;
        return row.height > 1 && row.bottom > scrollerRect.top + 1 && row.top < scrollerRect.bottom - 1;
      })
      .sort((left, right) => left.top - right.top);

    const overlaps: GeometrySnapshot["overlaps"] = [];
    for (let index = 1; index < visibleRows.length; index += 1) {
      const previous = visibleRows[index - 1];
      const next = visibleRows[index];
      if (next.top < previous.bottom - 1) {
        overlaps.push({
          previousId: previous.id,
          nextId: next.id,
          previousBottom: previous.bottom,
          nextTop: next.top,
          previousText: previous.text,
          nextText: next.text,
        });
      }
    }

    return {
      overlaps,
      visibleRows,
      sessionId: root.getAttribute("data-session-id"),
    };
  });
}

async function assertNoVisibleOverlap(
  page: Page,
  testInfo: TestInfo,
  step: string,
  expectedSessionId: string,
  debugLogs: string[],
) {
  const geometry = await readVisibleThreadGeometry(page);
  if (geometry.sessionId === expectedSessionId && geometry.overlaps.length === 0) return;

  const sessionMessageListDebug = await page.evaluate((sessionId) => {
    const store = window.__wbSessionMessageListDebug;
    if (!store) return [];
    return store.entries.filter((entry) => entry.sessionId === sessionId).slice(-60);
  }, expectedSessionId);
  const screenshotPath = testInfo.outputPath(`active-overlap-${step}.png`);
  const detailsPath = testInfo.outputPath(`active-overlap-${step}.json`);
  await page.screenshot({ path: screenshotPath, fullPage: true });
  await writeFile(detailsPath, JSON.stringify({ geometry, debugLogs, sessionMessageListDebug }, null, 2), "utf8");
  await testInfo.attach(`active-overlap-${step}.png`, {
    path: screenshotPath,
    contentType: "image/png",
  });
  await testInfo.attach(`active-overlap-${step}.json`, {
    path: detailsPath,
    contentType: "application/json",
  });
  throw new Error(
    `active-session overlap at ${step}: expectedSession=${expectedSessionId} actualSession=${geometry.sessionId} visibleOverlaps=${geometry.overlaps.length}`,
  );
}

async function monitorNoOverlap(
  page: Page,
  testInfo: TestInfo,
  sessionId: string,
  debugLogs: string[],
  prefix: string,
  samples = 20,
) {
  for (let index = 0; index < samples; index += 1) {
    await page.waitForTimeout(300);
    await assertNoVisibleOverlap(page, testInfo, `${prefix}-${index + 1}`, sessionId, debugLogs);
  }
}

test("workbench: active session streaming never overlaps visible rows", async ({ page, request }, testInfo) => {
  test.setTimeout(180_000);

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 8,
    throttleMs: 5,
    messageBytes: 2200,
    messagePrefix: "overlap seed",
    includeToolSummaries: true,
    toolSummariesPerTurn: 3,
  });

  const taskId = seed.taskIds[0];
  const sessionId = seed.sessionIdsByTask[taskId!]?.[0];
  expect(taskId).toBeTruthy();
  expect(sessionId).toBeTruthy();

  const debugLogs: string[] = [];
  page.on("console", (message) => {
    const text = message.text();
    if (text.includes("[MessageList]")) {
      debugLogs.push(text);
      if (debugLogs.length > 200) debugLogs.shift();
    }
  });

  await page.goto(`/workspaces/${seed.workspaceId}?debug=1`, { waitUntil: "domcontentloaded" });
  const task = page.locator(".wb-task-row").filter({ hasText: "fixture task 1" }).first();
  await expect(task).toBeVisible({ timeout: 30_000 });
  await task.click();
  await waitForVisibleSession(page, sessionId!);

  await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content: buildSlowPrompt("ACTIVE-OVERLAP", 1), delivery: "immediate" },
  });
  await waitForVisiblePendingAssistantRow(page);
  await monitorNoOverlap(page, testInfo, sessionId!, debugLogs, "first");

  await expect(page.locator(visibleStatusSelector).last()).toHaveText(/Completed/i, { timeout: 60_000 });
  await assertNoVisibleOverlap(page, testInfo, "first-complete", sessionId!, debugLogs);

  await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content: buildSlowPrompt("ACTIVE-OVERLAP", 2), delivery: "immediate" },
  });
  await waitForVisiblePendingAssistantRow(page);
  await monitorNoOverlap(page, testInfo, sessionId!, debugLogs, "second");

  await expect(page.locator(visibleStatusSelector).last()).toHaveText(/Completed/i, { timeout: 60_000 });
  await assertNoVisibleOverlap(page, testInfo, "second-complete", sessionId!, debugLogs);
});
