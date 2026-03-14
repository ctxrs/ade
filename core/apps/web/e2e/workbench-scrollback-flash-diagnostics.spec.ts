import { test, expect } from "./fixtures";
import type { APIRequestContext, Page, Route, TestInfo } from "@playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector = ".wb-session-slot[aria-hidden=\"false\"] .wb-thread-scroller";

type FlashTrace = {
  cause?: string;
  sampleCount?: number;
  snapbackDetected?: boolean;
  maxAbsScrollTopDeltaPx?: number;
  maxAbsFirstItemTopDeltaPx?: number;
  maxAbsScrollHeightDeltaPx?: number;
};

type MessageListDebugWindow = Window & {
  __wbSessionMessageListDebug?: {
    seq: number;
    entries: Array<Record<string, unknown>>;
    flashSeq?: number;
    flashTraces?: FlashTrace[];
  };
};

async function addLongMessages(request: APIRequestContext, sessionId: string, count: number) {
  const longText = Array.from({ length: 220 }, (_, index) => `history line ${index + 1}`).join("\n");
  for (let index = 0; index < count; index += 1) {
    const response = await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nhistory scroll ${index + 1}`, delivery: "immediate" },
    });
    expect(response.ok(), `failed to seed scrollback message ${index + 1}`).toBeTruthy();
  }
}

async function addConcurrentLiveMessage(request: APIRequestContext, sessionId: string, label: string) {
  const longText = Array.from({ length: 160 }, (_, index) => `live line ${index + 1}`).join("\n");
  const response = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: { content: `${longText}\n${label}`, delivery: "immediate" },
  });
  expect(response.ok(), `failed to post concurrent live message ${label}`).toBeTruthy();
}

async function clearDebugStore(page: Page) {
  await page.evaluate(() => {
    const win = window as MessageListDebugWindow;
    win.__wbSessionMessageListDebug = {
      seq: 0,
      entries: [],
      flashSeq: 0,
      flashTraces: [],
    };
  });
}

async function readDebugStore(page: Page) {
  return page.evaluate(() => {
    const win = window as MessageListDebugWindow;
    const store = win.__wbSessionMessageListDebug;
    return {
      seq: store?.seq ?? 0,
      flashSeq: store?.flashSeq ?? 0,
      entries: Array.isArray(store?.entries) ? store.entries.slice(-100) : [],
      flashTraces: Array.isArray(store?.flashTraces) ? store.flashTraces.slice(-20) : [],
    };
  });
}

async function forceBottom(page: Page) {
  await page.locator(scrollSelector).first().evaluate((element) => {
    element.scrollTop = Math.max(0, element.scrollHeight - element.clientHeight);
    element.dispatchEvent(new Event("scroll"));
  });
}

async function triggerHistoryLoad(page: Page, request: APIRequestContext, sessionId: string, attempt: number) {
  const historyUrlPattern = `**/api/sessions/${sessionId}/history**`;
  let delayedHistoryCount = 0;
  let concurrentAppendCount = 0;
  const routeHandler = async (route: Route) => {
    delayedHistoryCount += 1;
    if (delayedHistoryCount === 1) {
      await addConcurrentLiveMessage(request, sessionId, `concurrent-live-${attempt}-${Date.now()}`);
      concurrentAppendCount += 1;
      await new Promise((resolve) => setTimeout(resolve, 180));
    }
    await route.continue();
  };
  await page.route(historyUrlPattern, routeHandler);

  const historyResponse = page
    .waitForResponse((response) => response.url().includes(`/api/sessions/${sessionId}/history`), {
      timeout: 10000,
    })
    .catch(() => null);

  try {
    const scroller = page.locator(scrollSelector).first();
    await scroller.hover();
    for (let index = 0; index < 36; index += 1) {
      await page.mouse.wheel(0, -260);
      await page.waitForTimeout(45);
    }
    await historyResponse;
    await page.waitForTimeout(700);
  } finally {
    await page.unroute(historyUrlPattern, routeHandler);
  }

  expect(delayedHistoryCount, "expected a history request during scrollback").toBeGreaterThan(0);
  expect(concurrentAppendCount, "expected a delayed history window to inject a live append").toBeGreaterThan(0);
}

function relevantFlashTraces(traces: FlashTrace[]): FlashTrace[] {
  return traces.filter((trace) =>
    ["history:extend", "data:prepend", "data:reconcile"].includes(String(trace.cause ?? "")),
  );
}

test("workbench: scrollback diagnostics capture prepend/reconcile flash traces", async ({ page, request }, testInfo: TestInfo) => {
  test.setTimeout(180000);
  await page.setViewportSize({ width: 1400, height: 900 });

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 10,
    messageBytes: { min: 220, max: 320 },
    messagePrefix: "scrollback-flash",
  });

  const taskId = seed.taskIds[0];
  const sessionId = seed.sessionIdsByTask[taskId]?.[0] ?? "";
  expect(sessionId).toBeTruthy();
  await addLongMessages(request, sessionId, 12);

  const params = new URLSearchParams();
  params.set("debug", "1");
  await page.goto(`/workspaces/${seed.workspaceId}?${params.toString()}`, { waitUntil: "domcontentloaded" });

  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20000 });
  await rows.first().click();

  await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({
    timeout: 20000,
  });
  await expect(page.locator(scrollSelector).first()).toBeVisible({ timeout: 20000 });

  await forceBottom(page);

  let state = await readDebugStore(page);
  let traces = relevantFlashTraces(state.flashTraces);
  for (let attempt = 0; attempt < 4; attempt += 1) {
    await clearDebugStore(page);
    await triggerHistoryLoad(page, request, sessionId, attempt + 1);
    state = await readDebugStore(page);
    traces = relevantFlashTraces(state.flashTraces);
    const causes = new Set(traces.map((trace) => String(trace.cause ?? "")));
    if (causes.has("history:extend") || causes.has("data:reconcile")) break;
  }

  await testInfo.attach("scrollback-flash-debug.json", {
    body: JSON.stringify(state, null, 2),
    contentType: "application/json",
  });

  expect(traces.length, "expected scrollback instrumentation traces").toBeGreaterThan(0);
  expect(
    traces.some((trace) => ["history:extend", "data:reconcile"].includes(String(trace.cause ?? ""))),
    "expected a mixed history/live-update trace, not only a pure prepend",
  ).toBeTruthy();
  expect(
    traces.some((trace) => Number(trace.sampleCount ?? 0) >= 4),
    "expected at least one trace with multiple frame samples",
  ).toBeTruthy();
  expect(
    traces.some(
      (trace) =>
        Math.max(
          Number(trace.maxAbsFirstItemTopDeltaPx ?? 0),
          Number(trace.maxAbsScrollTopDeltaPx ?? 0),
          Number(trace.maxAbsScrollHeightDeltaPx ?? 0),
        ) >= 4,
    ),
    "expected trace deltas to capture visible movement or height change",
  ).toBeTruthy();

  if (process.env.CTX_E2E_SCROLLBACK_FLASH_STRICT === "1") {
    expect(
      traces.some((trace) => Boolean(trace.snapbackDetected)),
      "strict mode expects a detected scroll snapback",
    ).toBeTruthy();
  }
});
