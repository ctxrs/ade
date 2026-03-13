import { writeFile } from "node:fs/promises";
import type { Page, TestInfo } from "playwright/test";
import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const visibleSessionSelector = '.wb-session-slot[aria-hidden="false"] [data-testid="session-view"]';
const visibleScrollerSelector = `${visibleSessionSelector} .wb-thread-scroller`;

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
    style: string | null;
    className: string;
    parent: {
      tagName: string;
      className: string;
      style: string | null;
      dataIndex: string | null;
      knownSize: string | null;
      top: number;
      bottom: number;
      height: number;
    } | null;
  }>;
  scroller: {
    top: number;
    bottom: number;
    height: number;
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
  } | null;
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
  return `slow-diff-test emit-thought ${marker} ${index}
${longBody}
[[tool_calls]]
${JSON.stringify(toolCalls)}
[[/tool_calls]]`;
};

async function scrollVisibleThreadTo(page: Page, fraction: number) {
  const scroller = page.locator(visibleScrollerSelector).first();
  await expect(scroller).toBeVisible({ timeout: 20_000 });
  await expect
    .poll(async () => scroller.evaluate((node) => node.scrollHeight - node.clientHeight), { timeout: 20_000 })
    .toBeGreaterThan(300);
  await scroller.evaluate((node, targetFraction) => {
    const maxTop = Math.max(0, node.scrollHeight - node.clientHeight);
    node.scrollTop = Math.round(maxTop * targetFraction);
    node.dispatchEvent(new Event("scroll"));
  }, fraction);
  await page.waitForTimeout(250);
}

async function readVisibleThreadGeometry(page: Page): Promise<GeometrySnapshot> {
  const sessionView = page.locator(visibleSessionSelector).first();
  return readThreadGeometryFromLocator(sessionView);
}

async function readThreadGeometryBySession(page: Page, sessionId: string): Promise<GeometrySnapshot | null> {
  const sessionView = page.locator(`[data-testid="session-view"][data-session-id="${sessionId}"]`).first();
  if ((await sessionView.count()) === 0) return null;
  return readThreadGeometryFromLocator(sessionView);
}

async function readThreadGeometryFromLocator(sessionView: ReturnType<Page["locator"]>): Promise<GeometrySnapshot> {
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
        const parentRect = parent?.getBoundingClientRect() ?? null;
        return {
          id: el.getAttribute("data-thread-item-id") ?? "",
          top: rect.top,
          bottom: rect.bottom,
          height: rect.height,
          text: (el.innerText || "").slice(0, 180),
          style: el.getAttribute("style"),
          className: el.className,
          parent: parentRect
            ? {
                tagName: parent?.tagName ?? "",
                className: parent?.className ?? "",
                style: parent?.getAttribute("style") ?? null,
                dataIndex: parent?.getAttribute("data-index") ?? null,
                knownSize: parent?.getAttribute("data-known-size") ?? null,
                top: parentRect.top,
                bottom: parentRect.bottom,
                height: parentRect.height,
              }
            : null,
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
      scroller: scrollerRect
        ? {
            top: scrollerRect.top,
            bottom: scrollerRect.bottom,
            height: scrollerRect.height,
            scrollTop: scroller?.scrollTop ?? 0,
            scrollHeight: scroller?.scrollHeight ?? 0,
            clientHeight: scroller?.clientHeight ?? 0,
          }
        : null,
      sessionId: root.getAttribute("data-session-id"),
    };
  });
}

async function recordHiddenGeometry(
  page: Page,
  sessionId: string,
  label: string,
  debugLogs: string[],
) {
  const mountedSlots = await page.locator(".wb-session-slot").evaluateAll((nodes) =>
    nodes.map((node) => {
      const el = node as HTMLElement;
      const sessionView = el.querySelector('[data-testid="session-view"]') as HTMLElement | null;
      return {
        slotAriaHidden: el.getAttribute("aria-hidden"),
        slotStyle: el.getAttribute("style"),
        sessionId: sessionView?.getAttribute("data-session-id") ?? null,
      };
    }),
  );
  debugLogs.push(`${label}:mountedSlots=${JSON.stringify(mountedSlots)}`);
  if (debugLogs.length > 200) debugLogs.splice(0, debugLogs.length - 200);

  const mountedSessions = await page.locator('[data-testid="session-view"]').evaluateAll((nodes) =>
    nodes.map((node) => {
      const el = node as HTMLElement;
      const slot = el.closest(".wb-session-slot") as HTMLElement | null;
      return {
        sessionId: el.getAttribute("data-session-id"),
        slotAriaHidden: slot?.getAttribute("aria-hidden") ?? null,
        slotStyle: slot?.getAttribute("style") ?? null,
      };
    }),
  );
  debugLogs.push(`${label}:mountedSessions=${JSON.stringify(mountedSessions)}`);
  if (debugLogs.length > 200) debugLogs.splice(0, debugLogs.length - 200);

  const geometry = await readThreadGeometryBySession(page, sessionId);
  if (!geometry) {
    debugLogs.push(`${label}:not-mounted`);
    return;
  }
  debugLogs.push(
    JSON.stringify({
      label,
      sessionId,
      overlaps: geometry.overlaps.length,
      rows: geometry.visibleRows.map((row) => ({
        id: row.id,
        top: row.top,
        height: row.height,
        knownSize: row.parent?.knownSize ?? null,
        dataIndex: row.parent?.dataIndex ?? null,
      })),
    }),
  );
  if (debugLogs.length > 200) debugLogs.splice(0, debugLogs.length - 200);
}

async function assertNoVisibleOverlap(
  page: Page,
  testInfo: TestInfo,
  step: string,
  expectedSessionId: string,
  debugLogs: string[],
) {
  let geometry = await readVisibleThreadGeometry(page);
  for (let attempt = 0; attempt < 5; attempt += 1) {
    if (geometry.sessionId === expectedSessionId && geometry.overlaps.length === 0) return;
    await page.waitForTimeout(200);
    geometry = await readVisibleThreadGeometry(page);
  }
  if (geometry.sessionId !== expectedSessionId || geometry.overlaps.length > 0) {
    const sessionMessageListDebug = await page.evaluate((sessionId) => {
      const store = window.__wbSessionMessageListDebug;
      if (!store) return [];
      return store.entries.filter((entry) => entry.sessionId === sessionId).slice(-60);
    }, expectedSessionId);
    const screenshotPath = testInfo.outputPath(`task-overlap-${step}.png`);
    const detailsPath = testInfo.outputPath(`task-overlap-${step}.json`);
    await page.screenshot({ path: screenshotPath, fullPage: true });
    await writeFile(detailsPath, JSON.stringify({ geometry, debugLogs, sessionMessageListDebug }, null, 2), "utf8");
    await testInfo.attach(`task-overlap-${step}.png`, {
      path: screenshotPath,
      contentType: "image/png",
    });
    await testInfo.attach(`task-overlap-${step}.json`, {
      path: detailsPath,
      contentType: "application/json",
    });
    throw new Error(
      `task switch corruption at ${step}: expectedSession=${expectedSessionId} actualSession=${geometry.sessionId} visibleOverlaps=${geometry.overlaps.length}`,
    );
  }
}

test("workbench: switching between running tasks never overlaps visible rows", async ({ page, request }, testInfo) => {
  test.setTimeout(180_000);

  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 1,
    turnsPerSession: 10,
    throttleMs: 5,
    messageBytes: 2200,
    messagePrefix: "overlap seed",
    includeToolSummaries: true,
    toolSummariesPerTurn: 3,
  });

  const [taskAId, taskBId] = seed.taskIds;
  const sessionAId = seed.sessionIdsByTask[taskAId]?.[0];
  const sessionBId = seed.sessionIdsByTask[taskBId]?.[0];
  expect(sessionAId).toBeTruthy();
  expect(sessionBId).toBeTruthy();

  const debugLogs: string[] = [];
  page.on("console", (message) => {
    const text = message.text();
    if (text.includes("[MessageList]")) {
      debugLogs.push(text);
      if (debugLogs.length > 200) debugLogs.shift();
    }
  });

  await page.goto(`/workspaces/${seed.workspaceId}?debug=1`, { waitUntil: "domcontentloaded" });

  const rows = page.locator(".wb-task-row");
  const taskA = rows.filter({ hasText: "fixture task 1" }).first();
  const taskB = rows.filter({ hasText: "fixture task 2" }).first();

  await expect(rows).toHaveCount(2, { timeout: 30_000 });

  await taskA.click();
  await scrollVisibleThreadTo(page, 0.42);
  const sessionAScrollBefore = await page.locator(visibleScrollerSelector).first().evaluate((node) => node.scrollTop);
  expect(sessionAScrollBefore).toBeGreaterThan(100);
  await assertNoVisibleOverlap(page, testInfo, "alpha-initial", sessionAId!, debugLogs);

  await taskB.click();
  await scrollVisibleThreadTo(page, 0.58);
  const sessionBScrollBefore = await page.locator(visibleScrollerSelector).first().evaluate((node) => node.scrollTop);
  expect(sessionBScrollBefore).toBeGreaterThan(100);
  await assertNoVisibleOverlap(page, testInfo, "bravo-initial", sessionBId!, debugLogs);

  await Promise.all([
    request.post(`/api/sessions/${sessionAId}/messages`, {
      data: { content: buildSlowPrompt("ALPHA-SLOW", 1), delivery: "immediate" },
    }),
    request.post(`/api/sessions/${sessionBId}/messages`, {
      data: { content: buildSlowPrompt("BRAVO-SLOW", 1), delivery: "immediate" },
    }),
  ]);

  for (let index = 0; index < 10; index += 1) {
    await recordHiddenGeometry(page, sessionAId!, `alpha-hidden-before-${index + 1}`, debugLogs);
    await taskA.click();
    await expect(page.locator(visibleSessionSelector).first()).toHaveAttribute("data-session-id", sessionAId!, {
      timeout: 20_000,
    });
    await page.waitForTimeout(250);
    await assertNoVisibleOverlap(page, testInfo, `alpha-${index + 1}`, sessionAId!, debugLogs);

    await recordHiddenGeometry(page, sessionBId!, `bravo-hidden-before-${index + 1}`, debugLogs);
    await taskB.click();
    await expect(page.locator(visibleSessionSelector).first()).toHaveAttribute("data-session-id", sessionBId!, {
      timeout: 20_000,
    });
    await page.waitForTimeout(250);
    await assertNoVisibleOverlap(page, testInfo, `bravo-${index + 1}`, sessionBId!, debugLogs);
  }
});
