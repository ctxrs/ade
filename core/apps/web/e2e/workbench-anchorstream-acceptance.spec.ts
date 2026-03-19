import { test, expect } from "./fixtures";
import type { APIRequestContext, Page, TestInfo } from "@playwright/test";
import {
  readMessageListDebugStore,
  clearMessageListDebugStore,
  startSessionHistoryCapture,
} from "./utils/taskOpenHistoryRegression";
import {
  collectThreadSamples,
  forceScrollToBottom,
  forceScrollToTop,
  readActiveSessionId,
  readThreadSurfaceSample,
  readRowOffsetById,
  type ThreadSurfaceSample,
} from "./utils/anchorstreamAcceptanceProbes";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";
import { resolveAcceptanceSeedSource } from "./utils/resolveAcceptanceSeedSource";

const WORKSPACE_ID = process.env.ANCHORSTREAM_WORKSPACE_ID ?? "00000000-0000-4000-8000-000000000004";
const WORKSPACE_TOKEN = process.env.ANCHORSTREAM_WORKSPACE_TOKEN ?? process.env.CTX_E2E_AUTH_TOKEN ?? process.env.ANCHORSTREAM_AUTH_TOKEN ?? "00000000-0000-4000-8000-000000000003";
const WORKSPACE_QUERY = new URLSearchParams({ token: WORKSPACE_TOKEN, debug: "1" }).toString();
const TASK_COUNT_REQUIRED = Number(process.env.ANCHORSTREAM_REQUIRED_TASK_COUNT ?? "2");
const SCROLL_SELECTOR = ".wb-thread-scroller";
const OPEN_SAMPLE_MS = Number(process.env.ANCHORSTREAM_OPEN_SAMPLE_MS ?? "2400");
const OPEN_SAMPLE_INTERVAL_MS = Number(process.env.ANCHORSTREAM_OPEN_SAMPLE_INTERVAL_MS ?? "90");
const POST_SWITCH_SETTLE_MS = Number(process.env.ANCHORSTREAM_POST_SWITCH_SETTLE_MS ?? "1000");
const PREPEND_HISTORY_TASK_ATTEMPTS = Number(process.env.ANCHORSTREAM_PREPEND_HISTORY_TASK_ATTEMPTS ?? "0");
const PREPEND_HISTORY_TASK_TITLE = process.env.ANCHORSTREAM_PREPEND_HISTORY_TASK_TITLE ?? "Security Notice Mac";
const TASK_SWITCH_SIGNIFICANT_GROWTH = Number(process.env.ANCHORSTREAM_TASK_SWITCH_SIGNIFICANT_GROWTH ?? "72");
const TASK_SWITCH_MAX_POST_SETTLE_STEP = Number(process.env.ANCHORSTREAM_TASK_SWITCH_MAX_POST_SETTLE_STEP ?? "8");
const PREPEND_MAX_TOP_JITTER_PX = Number(process.env.ANCHORSTREAM_PREPEND_MAX_TOP_JITTER_PX ?? "48");
const BOTTOM_BLANK_TAIL_PX = Number(process.env.ANCHORSTREAM_MAX_BLANK_TAIL_PX ?? "120");
const BOTTOM_DISTANCE_PX = Number(process.env.ANCHORSTREAM_MAX_BOTTOM_DISTANCE_PX ?? "4");
const SHORT_THREAD_OPEN_SAMPLE_MS = Number(process.env.ANCHORSTREAM_SHORT_THREAD_OPEN_SAMPLE_MS ?? "1800");
const SHORT_THREAD_MAX_BLANK_TAIL_PX = Number(process.env.ANCHORSTREAM_SHORT_THREAD_MAX_BLANK_TAIL_PX ?? "6");
const SHORT_THREAD_MAX_SHIFT_PX = Number(process.env.ANCHORSTREAM_SHORT_THREAD_MAX_SHIFT_PX ?? "8");
const SHORT_THREAD_MAX_OVERFLOW_PX = Number(process.env.ANCHORSTREAM_SHORT_THREAD_MAX_OVERFLOW_PX ?? "12");

type SwitchSummary = {
  firstVisibleAtMs: number | null;
  finalItemCount: number;
  maxItemCount: number;
  significantJumps: number;
  lateSignificantJumps: number;
  maxPostSettleStep: number;
  maxOverlappingVisiblePairs: number;
  maxAdjacentVisibleOverlapPx: number;
};

type HistoryProbeResult = {
  historyTriggered: boolean;
  historyOffsetDeltas: number[];
  captures: unknown[];
};

type ShortThreadSummary = {
  firstVisibleAtMs: number | null;
  visibleSampleCount: number;
  maxBlankTailPx: number;
  maxBlankTailDeltaPx: number;
  maxDistanceFromBottomPx: number;
  maxScrollTopDeltaPx: number;
  maxScrollHeightDeltaPx: number;
  maxOverflowPx: number;
  finalBlankTailPx: number | null;
  finalDistanceFromBottomPx: number | null;
};

type ShortThreadSeedSource = {
  workspaceId: string;
  shortMessage: string;
  shortTaskLabel: string;
};

function maxPairwiseDelta<T>(samples: readonly T[], extract: (value: T) => number | null): number {
  let max = 0;
  for (let index = 1; index < samples.length; index += 1) {
    const previous = extract(samples[index - 1]);
    const next = extract(samples[index]);
    if (previous == null || next == null) continue;
    max = Math.max(max, Math.abs(next - previous));
  }
  return max;
}

function summarizeOpenStability(samples: ThreadSurfaceSample[]): SwitchSummary {
  const firstVisible = samples.findIndex(
    (sample) => sample.sessionVisible && sample.scrollerMounted && sample.renderedItemCount > 0,
  );
  if (firstVisible < 0) {
    return {
      firstVisibleAtMs: null,
      finalItemCount: 0,
      maxItemCount: 0,
      significantJumps: 0,
      lateSignificantJumps: 0,
      maxPostSettleStep: 0,
      maxOverlappingVisiblePairs: 0,
      maxAdjacentVisibleOverlapPx: 0,
    };
  }

  const baseline = samples[firstVisible];
  const baseCount = baseline.renderedItemCount;
  const baselineMs = baseline.atMs;
  let significantJumps = 0;
  let lateSignificantJumps = 0;
  let maxPostSettleStep = 0;
  let maxItemCount = baseCount;
  let maxOverlappingVisiblePairs = baseline.overlappingVisiblePairs ?? 0;
  let maxAdjacentVisibleOverlapPx = baseline.maxAdjacentVisibleOverlapPx ?? 0;

  for (let index = firstVisible + 1; index < samples.length; index += 1) {
    const prev = samples[index - 1];
    const current = samples[index];
    if (!prev || !current) continue;
    const countDelta = Math.abs(current.renderedItemCount - prev.renderedItemCount);
    const visibleCountDelta = Math.abs(current.visibleRowCount - prev.visibleRowCount);
    if (countDelta >= TASK_SWITCH_SIGNIFICANT_GROWTH) {
      significantJumps += 1;
      if (current.atMs - baselineMs >= POST_SWITCH_SETTLE_MS) {
        lateSignificantJumps += 1;
      }
    }
    if (current.atMs - baselineMs >= POST_SWITCH_SETTLE_MS) {
      maxPostSettleStep = Math.max(maxPostSettleStep, visibleCountDelta);
    }
    maxItemCount = Math.max(maxItemCount, current.renderedItemCount);
    maxOverlappingVisiblePairs = Math.max(
      maxOverlappingVisiblePairs,
      current.overlappingVisiblePairs ?? 0,
    );
    maxAdjacentVisibleOverlapPx = Math.max(
      maxAdjacentVisibleOverlapPx,
      current.maxAdjacentVisibleOverlapPx ?? 0,
    );
  }

  return {
    firstVisibleAtMs: baselineMs,
    finalItemCount: samples.at(-1)?.renderedItemCount ?? 0,
    maxItemCount,
    significantJumps,
    lateSignificantJumps,
    maxPostSettleStep,
    maxOverlappingVisiblePairs,
    maxAdjacentVisibleOverlapPx,
  };
}

function summarizeShortThreadOpen(samples: ThreadSurfaceSample[]): ShortThreadSummary {
  const firstVisibleIndex = samples.findIndex(
    (sample) => sample.sessionVisible && sample.scrollerMounted && sample.renderedItemCount > 0,
  );
  if (firstVisibleIndex < 0) {
    return {
      firstVisibleAtMs: null,
      visibleSampleCount: 0,
      maxBlankTailPx: 0,
      maxBlankTailDeltaPx: 0,
      maxDistanceFromBottomPx: 0,
      maxScrollTopDeltaPx: 0,
      maxScrollHeightDeltaPx: 0,
      maxOverflowPx: 0,
      finalBlankTailPx: null,
      finalDistanceFromBottomPx: null,
    };
  }

  const visibleSamples = samples.slice(firstVisibleIndex).filter(
    (sample) => sample.sessionVisible && sample.scrollerMounted && sample.renderedItemCount > 0,
  );
  const maxBlankTailPx = visibleSamples.reduce(
    (max, sample) => Math.max(max, sample.blankTailPx ?? 0),
    0,
  );
  const maxDistanceFromBottomPx = visibleSamples.reduce(
    (max, sample) => Math.max(max, sample.distanceFromMaxScrollPx ?? 0),
    0,
  );
  const maxOverflowPx = visibleSamples.reduce((max, sample) => {
    if (sample.scrollHeight == null || sample.clientHeight == null) {
      return max;
    }
    return Math.max(max, Math.max(0, sample.scrollHeight - sample.clientHeight));
  }, 0);
  const final = visibleSamples.at(-1) ?? null;

  return {
    firstVisibleAtMs: samples[firstVisibleIndex]?.atMs ?? null,
    visibleSampleCount: visibleSamples.length,
    maxBlankTailPx,
    maxBlankTailDeltaPx: maxPairwiseDelta(visibleSamples, (sample) => sample.blankTailPx),
    maxDistanceFromBottomPx,
    maxScrollTopDeltaPx: maxPairwiseDelta(visibleSamples, (sample) => sample.scrollTop),
    maxScrollHeightDeltaPx: maxPairwiseDelta(visibleSamples, (sample) => sample.scrollHeight),
    maxOverflowPx,
    finalBlankTailPx: final?.blankTailPx ?? null,
    finalDistanceFromBottomPx: final?.distanceFromMaxScrollPx ?? null,
  };
}

function assertBottomStability(sample: ThreadSurfaceSample, label: string) {
  expect(sample.scrollerMounted, `${label}: scroller mounted`).toBeTruthy();
  expect(sample.renderedItemCount, `${label}: visible rows`).toBeGreaterThan(0);
  expect(sample.distanceFromMaxScrollPx ?? Number.POSITIVE_INFINITY, `${label}: at bottom`).toBeLessThanOrEqual(BOTTOM_DISTANCE_PX);
  expect(sample.blankTailPx ?? Number.POSITIVE_INFINITY, `${label}: blank tail`).toBeLessThanOrEqual(BOTTOM_BLANK_TAIL_PX);
  expect(sample.overlappingVisiblePairs, `${label}: no visible overlap pairs`).toBe(0);
  expect(sample.maxAdjacentVisibleOverlapPx ?? Number.POSITIVE_INFINITY, `${label}: no visible overlap`).toBeLessThanOrEqual(1);
  expect(sample.impossibleTail, `${label}: impossible tail`).toBeFalsy();
  expect(sample.isBottom, `${label}: bottom-anchor`).toBeTruthy();
}

async function openTaskRow(page: Page, taskTarget: number | string): Promise<void> {
  const row =
    typeof taskTarget === "number"
      ? page.locator(".wb-task-row").nth(taskTarget)
      : page.locator(".wb-task-row").filter({ hasText: taskTarget }).first();
  await row.click();
  await expect(page.locator("textarea.wb-active-textarea")).toBeVisible({ timeout: 30_000 });
  await expect(page.locator(SCROLL_SELECTOR).first()).toBeVisible({ timeout: 30_000 });
}

async function attachJson(testInfo: TestInfo, name: string, payload: unknown) {
  await testInfo.attach(`${name}.json`, {
    body: JSON.stringify(payload, null, 2),
    contentType: "application/json",
  });
}

async function createShortThreadHarnessWorkspace(
  request: APIRequestContext,
): Promise<ShortThreadSeedSource> {
  const sessionSource = await resolveAcceptanceSeedSource(request);
  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 0,
    turnsPerSession: 0,
    throttleMs: 0,
    createDefaultSession: false,
    messagePrefix: "short-thread",
    sessionSource,
  });
  const shortTaskId = seed.taskIds[0];
  if (!shortTaskId) {
    throw new Error("short-thread seed task missing");
  }

  const shortMessage = `short-thread-${Date.now()}`;
  const sessionResp = await request.post(`/api/tasks/${shortTaskId}/sessions`, {
    data: {
      provider_id: sessionSource.providerId,
      model_id: sessionSource.modelId,
      execution_environment: sessionSource.executionEnvironment,
    },
  });
  expect(sessionResp.ok(), "short-thread session creation succeeds").toBeTruthy();
  const session = (await sessionResp.json()) as { id?: string | null };
  const sessionId = typeof session.id === "string" ? session.id : "";
  expect(sessionId, "short-thread session id exists").toBeTruthy();

  const messageResp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: {
      content: shortMessage,
      delivery: "immediate",
    },
  });
  expect(messageResp.ok(), "short-thread immediate message succeeds").toBeTruthy();

  const snapshotDeadline = Date.now() + 20_000;
  while (Date.now() < snapshotDeadline) {
    const snapshotResp = await request.get(`/api/sessions/${sessionId}/snapshot?limit=4`);
    expect(snapshotResp.ok(), "short-thread snapshot succeeds").toBeTruthy();
    const snapshot = (await snapshotResp.json()) as {
      head?: {
        turns?: Array<{ status?: string | null }>;
      };
    };
    const turns = Array.isArray(snapshot.head?.turns) ? snapshot.head?.turns : [];
    if (
      turns.length > 0 &&
      turns.every((turn) => turn?.status === "completed" || turn?.status === "done")
    ) {
      break;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }

  return {
    workspaceId: seed.workspaceId,
    shortMessage,
    shortTaskLabel: "fixture task 1",
  };
}

async function seedAnchorstreamAcceptanceToken(page: Page) {
  await page.addInitScript((token: string) => {
    const raw = window.sessionStorage.getItem("ctxDaemonConnectionV1");
    let current: Record<string, unknown> = {};
    if (raw) {
      try {
        current = JSON.parse(raw) as Record<string, unknown>;
      } catch {
        current = {};
      }
    }
    window.sessionStorage.setItem(
      "ctxDaemonConnectionV1",
      JSON.stringify({
        ...current,
        v: 1,
        authToken: token,
      }),
    );
  }, WORKSPACE_TOKEN);
}

async function probeTaskHistoryCapture(
  page: Page,
  testInfo: TestInfo,
  taskTarget: number | string,
  label: string,
): Promise<HistoryProbeResult | null> {
  await clearMessageListDebugStore(page);
  await openTaskRow(page, taskTarget);
  await page.waitForTimeout(250);

  const activeSessionId = await readActiveSessionId(page);
  if (!activeSessionId) {
    await attachJson(testInfo, `history-attempt-${label}-no-session`, { label, activeSessionId: null });
    return null;
  }

  const beforeOpen = await readThreadSurfaceSample(page, SCROLL_SELECTOR);
  const anchorRowId = beforeOpen.firstItemId;
  if (anchorRowId == null) {
    await attachJson(testInfo, `history-attempt-${label}-no-anchor`, {
      label,
      activeSessionId,
      beforeOpen,
    });
    return null;
  }

  const baseAnchorOffset = await readRowOffsetById(page, anchorRowId, SCROLL_SELECTOR);
  if (baseAnchorOffset == null) {
    await attachJson(testInfo, `history-attempt-${label}-no-anchor-offset`, {
      label,
      activeSessionId,
      anchorRowId,
    });
    return null;
  }

  const historyCapture = await startSessionHistoryCapture(page, activeSessionId);
  const historyOffsetDeltas: number[] = [];
  let historyTriggered = false;

  await forceScrollToTop(page, SCROLL_SELECTOR);
  for (let step = 0; step < 24; step += 1) {
    const baselineCaptures = historyCapture.captures.length;
    await page.mouse.wheel(0, -280);
    await page.waitForTimeout(120);

    if (historyCapture.captures.length <= baselineCaptures) {
      continue;
    }

    historyTriggered = true;
    const latestCapture = historyCapture.captures.at(-1);
    const currentAnchorOffset = await readRowOffsetById(page, anchorRowId, SCROLL_SELECTOR);
    if (currentAnchorOffset != null) {
      historyOffsetDeltas.push(Math.abs(currentAnchorOffset - baseAnchorOffset));
    }

    const sample = await readThreadSurfaceSample(page, SCROLL_SELECTOR);
    await attachJson(testInfo, `history-task-${label}-step-${step}`, sample);
    if (latestCapture?.hasMore === false) {
      break;
    }
  }

  await historyCapture.stop();
  await attachJson(testInfo, `history-task-${label}-captures`, historyCapture.captures);
  if (!historyTriggered) {
    return {
      historyTriggered: false,
      historyOffsetDeltas: [],
      captures: historyCapture.captures,
    };
  }

  return {
    historyTriggered,
    historyOffsetDeltas,
    captures: historyCapture.captures,
  };
}

test.describe.configure({ mode: "serial" });

test("anchorstream: task switching has bounded open growth and stable bottom alignment", async ({ page, request }, testInfo) => {
  test.setTimeout(180_000);
  await page.setViewportSize({ width: 1440, height: 960 });
  await seedAnchorstreamAcceptanceToken(page);

  await page.goto(`/workspaces/${WORKSPACE_ID}?${WORKSPACE_QUERY}`, {
    waitUntil: "domcontentloaded",
    timeout: 30_000,
  });
  const taskRows = page.locator(".wb-task-row");
  await expect(taskRows.first()).toBeVisible({ timeout: 20_000 });
  expect(await taskRows.count(), "task rows are listed").toBeGreaterThanOrEqual(TASK_COUNT_REQUIRED);
  const taskRowCount = await taskRows.count();
  const clickCount = Math.min(taskRowCount, TASK_COUNT_REQUIRED);
  expect(clickCount, "enough tasks available for switch test").toBeGreaterThan(1);

  await clearMessageListDebugStore(page);
  await openTaskRow(page, 0);
  await forceScrollToBottom(page, SCROLL_SELECTOR);
  await page.waitForTimeout(150);

  for (let taskIndex = 1; taskIndex < clickCount; taskIndex += 1) {
    const label = `task-switch-${taskIndex}`;
    await clearMessageListDebugStore(page);
    await openTaskRow(page, taskIndex);

    const samples = await collectThreadSamples(page, {
      scrollerSelector: SCROLL_SELECTOR,
      sampleDurationMs: OPEN_SAMPLE_MS,
      sampleIntervalMs: OPEN_SAMPLE_INTERVAL_MS,
    });
    const summary = summarizeOpenStability(samples);
    await attachJson(testInfo, label, summary);
    await attachJson(testInfo, `${label}-samples`, samples);

    expect(summary.firstVisibleAtMs, `${label}: thread became visible`).not.toBeNull();
    expect(summary.significantJumps, `${label}: significant growth jumps`).toBeLessThanOrEqual(3);
    expect(summary.lateSignificantJumps, `${label}: late growth burst`).toBe(0);
    expect(summary.maxPostSettleStep, `${label}: post settle step`).toBeLessThanOrEqual(TASK_SWITCH_MAX_POST_SETTLE_STEP);
    expect(summary.maxItemCount, `${label}: open item burst`).toBeLessThanOrEqual(summary.finalItemCount + 4);
    expect(summary.maxOverlappingVisiblePairs, `${label}: no visible row overlap pairs`).toBe(0);
    expect(summary.maxAdjacentVisibleOverlapPx, `${label}: no visible row overlap`).toBeLessThanOrEqual(1);

    const debugState = await readMessageListDebugStore(page);
    expect(
      debugState.flashTraces.some((trace) => trace.snapbackDetected),
      `${label}: no flash snapback detected`,
    ).toBeFalsy();

    await forceScrollToBottom(page, SCROLL_SELECTOR);
    await page.waitForTimeout(150);
    const bottom = await readThreadSurfaceSample(page, SCROLL_SELECTOR);
    assertBottomStability(bottom, `${label}-bottom`);
  }
});

test("anchorstream: prepend history keeps top anchor stable and bottom stable after load", async ({ page }, testInfo) => {
  test.setTimeout(200_000);
  await page.setViewportSize({ width: 1440, height: 960 });
  await seedAnchorstreamAcceptanceToken(page);

  await page.goto(`/workspaces/${WORKSPACE_ID}?${WORKSPACE_QUERY}`, {
    waitUntil: "domcontentloaded",
    timeout: 30_000,
  });
  const taskRows = page.locator(".wb-task-row");
  await expect(taskRows.first()).toBeVisible({ timeout: 20_000 });
  expect(await taskRows.count(), "task row exists for prepend test").toBeGreaterThan(0);
  const taskCount = await taskRows.count();
  const configuredHistoryAttempts =
    Number.isFinite(PREPEND_HISTORY_TASK_ATTEMPTS) && PREPEND_HISTORY_TASK_ATTEMPTS > 0
      ? PREPEND_HISTORY_TASK_ATTEMPTS
      : taskCount;
  const maxHistoryAttempts = Math.max(1, Math.min(taskCount, configuredHistoryAttempts));
  let historyResult: HistoryProbeResult | null = null;
  let selectedTaskLabel = "";

  const preferredHistoryTaskCount = await page
    .locator(".wb-task-row")
    .filter({ hasText: PREPEND_HISTORY_TASK_TITLE })
    .count();
  if (preferredHistoryTaskCount > 0) {
    const preferredResult = await probeTaskHistoryCapture(
      page,
      testInfo,
      PREPEND_HISTORY_TASK_TITLE,
      "preferred-history-task",
    );
    if (preferredResult?.historyTriggered) {
      historyResult = preferredResult;
      selectedTaskLabel = PREPEND_HISTORY_TASK_TITLE;
    } else {
      await page.reload({ waitUntil: "domcontentloaded" });
      await page.waitForTimeout(350);
    }
  }

  for (let taskIndex = 0; historyResult == null && taskIndex < maxHistoryAttempts; taskIndex += 1) {
    const nextResult = await probeTaskHistoryCapture(page, testInfo, taskIndex, `task-${taskIndex}`);
    if (nextResult?.historyTriggered) {
      historyResult = nextResult;
      selectedTaskLabel = `task-index-${taskIndex}`;
      break;
    }
    await page.reload({ waitUntil: "domcontentloaded" });
    await page.waitForTimeout(350);
  }

  expect(historyResult, "found task that triggers history load").toBeTruthy();
  if (!historyResult) return;

  expect(selectedTaskLabel, "selected task label").not.toHaveLength(0);
  if (historyResult.historyOffsetDeltas.length > 1) {
    const maxJitter = Math.max(...historyResult.historyOffsetDeltas);
    expect(maxJitter, "top anchor displacement under prepend").toBeLessThanOrEqual(PREPEND_MAX_TOP_JITTER_PX);
  }
  expect(
    historyResult.captures.every((capture) => {
      const hasStatus = typeof (capture as { status?: number }).status === "number";
      const status = (capture as { status?: number }).status ?? 0;
      return hasStatus && status >= 200 && status < 300;
    }),
    `history responses for ${selectedTaskLabel} are successful`,
  ).toBeTruthy();

  for (let bottomCheck = 0; bottomCheck < 4; bottomCheck += 1) {
    await forceScrollToBottom(page, SCROLL_SELECTOR);
    await page.waitForTimeout(120);
    const sample = await readThreadSurfaceSample(page, SCROLL_SELECTOR);
    assertBottomStability(sample, `prepend-bottom-${bottomCheck}`);
  }
});

test("anchorstream: short-thread opens bottom-aligned without lower blank or post-open shift", async ({
  page,
  request,
}, testInfo) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 960 });
  await seedAnchorstreamAcceptanceToken(page);

  const seed = await createShortThreadHarnessWorkspace(request);

  await page.goto(`/workspaces/${seed.workspaceId}?${new URLSearchParams({ token: WORKSPACE_TOKEN, debug: "1" }).toString()}`, {
    waitUntil: "domcontentloaded",
    timeout: 30_000,
  });
  await expect(page.locator(".wb-task-row").first()).toBeVisible({ timeout: 20_000 });

  await clearMessageListDebugStore(page);
  const shortTaskRow = page.locator(".wb-task-row").filter({ hasText: seed.shortTaskLabel }).first();
  await shortTaskRow.click();
  await expect(page.locator(SCROLL_SELECTOR).first()).toBeVisible({ timeout: 20_000 });
  await expect(page.locator(".wb-session-slot .wb-thread-scroller").first()).toContainText(seed.shortMessage, { timeout: 20_000 });

  const samples = await collectThreadSamples(page, {
    scrollerSelector: SCROLL_SELECTOR,
    sampleDurationMs: SHORT_THREAD_OPEN_SAMPLE_MS,
    sampleIntervalMs: OPEN_SAMPLE_INTERVAL_MS,
  });
  const summary = summarizeShortThreadOpen(samples);
  const debugState = await readMessageListDebugStore(page);

  await attachJson(testInfo, "short-thread-open-summary", summary);
  await attachJson(testInfo, "short-thread-open-samples", samples);
  await attachJson(testInfo, "short-thread-open-debug", debugState);
  await attachJson(testInfo, "short-thread-open-fixture", seed);

  expect(summary.firstVisibleAtMs, "short-thread: thread became visible").not.toBeNull();
  expect(summary.visibleSampleCount, "short-thread: collected visible samples").toBeGreaterThan(4);
  expect(summary.maxOverflowPx, "short-thread: transcript remains shorter than viewport").toBeLessThanOrEqual(
    SHORT_THREAD_MAX_OVERFLOW_PX,
  );
  expect(summary.maxDistanceFromBottomPx, "short-thread: stays at bottom").toBeLessThanOrEqual(BOTTOM_DISTANCE_PX);
  expect(summary.maxBlankTailPx, "short-thread: no large lower blank gap").toBeLessThanOrEqual(
    SHORT_THREAD_MAX_BLANK_TAIL_PX,
  );
  expect(summary.maxBlankTailDeltaPx, "short-thread: lower gap does not shift after open").toBeLessThanOrEqual(
    SHORT_THREAD_MAX_SHIFT_PX,
  );
  expect(summary.maxScrollTopDeltaPx, "short-thread: scrollTop stays stable after first paint").toBeLessThanOrEqual(
    SHORT_THREAD_MAX_SHIFT_PX,
  );
  expect(
    summary.maxScrollHeightDeltaPx,
    "short-thread: scrollHeight does not materially change after the first visible frame",
  ).toBeLessThanOrEqual(SHORT_THREAD_MAX_SHIFT_PX);
  expect(
    debugState.flashTraces.some((trace) => trace.snapbackDetected),
    "short-thread: no snapback trace recorded",
  ).toBeFalsy();
});
