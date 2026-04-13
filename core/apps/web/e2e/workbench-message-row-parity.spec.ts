import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";
import {
  measureAssistantParity,
  measureAssistantStreamingParity,
  measureMessageParity,
  measureTurnHeaderParity,
  openWorkbenchShell,
} from "./utils/pretextParity";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const EXACT_USER_MESSAGE = [
  "here is another neutral layout idea for a deterministic fixture",
  "",
  "we could ask the model to summarize a sample article and then write ten synthetic review notes.",
  "",
  "then we can compare whether the notes use the same broad style as the reference examples",
  "",
  "for example, a short positive note or a careful critical note can exercise different wrapping without carrying real conversation text.",
  "",
  "what other neutral variations should this layout test include",
].join("\n");

const COLLAPSED_LONG_MESSAGE = Array.from(
  { length: 24 },
  (_, index) => `line ${index + 1} with enough words to wrap a little bit`,
).join("\n");

const ASSISTANT_MARKDOWN = [
  "# Title",
  "",
  "- bullet one",
  "- bullet two",
  "",
  "```ts",
  "const x = 1;",
  "```",
].join("\n");
const ASSISTANT_INLINE_CODE_WRAP_MARKDOWN =
  "begin agent message here with plain text, and now some inline code block: `inline-thing-that-actually-gets-really-long-so-much-so-that-it-actually-wraps-to-2-lines-and-keeps-going-with-extra-path-segments/core/apps/web/src/pages/sessionThread/sessionMarkdownMeasurement.ts`";
const ASSISTANT_LONG_STATUS_MESSAGE = Array.from(
  { length: 48 },
  (_, index) =>
    `Status update ${index + 1}: validating a deterministic workspace message fixture with enough text to exercise wrapping and streaming layout.`,
).join("\n\n");
const TURN_HEADER_TEXT = "and what about the CI smoke failure?";

const IMAGE_DATA_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO2VzJ8AAAAASUVORK5CYII=";

test("workbench: exact multi-paragraph user message planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureMessageParity(page, {
    content: EXACT_USER_MESSAGE,
    expanded: true,
  });

  expect(
    Math.abs(measurement.delta),
    `message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: collapsed toggleable user message planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureMessageParity(page, {
    content: COLLAPSED_LONG_MESSAGE,
    expanded: false,
  });

  expect(
    Math.abs(measurement.delta),
    `collapsed message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: image attachments stay in parity for message rows", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureMessageParity(page, {
    content: "two inline screenshots",
    expanded: true,
    attachments: [
      { kind: "image", mime_type: "image/png", data_base64: IMAGE_DATA_BASE64, name: "one.png" },
      { kind: "image", mime_type: "image/png", data_base64: IMAGE_DATA_BASE64, name: "two.png" },
    ],
  });

  expect(
    Math.abs(measurement.delta),
    `attachment message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: assistant markdown planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureAssistantParity(page, { content: ASSISTANT_MARKDOWN });

  expect(
    Math.abs(measurement.delta),
    `assistant drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: assistant prose with wrapped inline code matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureAssistantParity(page, { content: ASSISTANT_INLINE_CODE_WRAP_MARKDOWN });

  expect(
    Math.abs(measurement.delta),
    `assistant inline-code drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: long multi-paragraph assistant status message matches mounted row height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureAssistantParity(page, { content: ASSISTANT_LONG_STATUS_MESSAGE });

  expect(
    Math.abs(measurement.delta),
    `assistant long-message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual}, hidden ${measurement.hiddenMeasured ?? "n/a"}, viewportWidth ${measurement.viewportWidth ?? "n/a"}, rowWidth ${measurement.rowWidth ?? "n/a"})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: partial assistant streaming stays identical to completed rendering for the same cumulative content", async ({
  page,
}) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureAssistantStreamingParity(page, {
    fragments: [
      "Before",
      "\n\n- partial item with `inline-tail-token/with/path`",
      "\n- second bullet with more prose to wrap near the edge",
    ],
  });

  expect(measurement.steps.length).toBe(3);
  for (const [index, step] of measurement.steps.entries()) {
    expect(
      Math.abs(step.partial.delta),
      `streaming partial step ${index} drifted by ${step.partial.delta}px (planned ${step.partial.planned}, actual ${step.partial.actual})`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(step.complete.delta),
      `streaming complete step ${index} drifted by ${step.complete.delta}px (planned ${step.complete.planned}, actual ${step.complete.actual})`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(step.actualDelta),
      `streaming actual mismatch at step ${index}: partial=${step.partial.actual} complete=${step.complete.actual}`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(step.plannedDelta),
      `streaming planned mismatch at step ${index}: partial=${step.partial.planned} complete=${step.complete.planned}`,
    ).toBeLessThanOrEqual(1);
    expect(step.structureEquivalent, `streaming structure diverged at step ${index} for content:\n${step.content}`).toBe(true);
  }
});

test("workbench: turn header planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureTurnHeaderParity(page, { plainText: TURN_HEADER_TEXT });

  expect(
    Math.abs(measurement.delta),
    `turn header drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

type VisibleAssistantParity = {
  id: string;
  actual: number;
  planned: number;
  delta: number;
  text: string;
};

async function readVisibleLatestAssistantParity(page: Page): Promise<VisibleAssistantParity | null> {
  return page.locator('.wb-session-slot [data-testid="session-view"]').first().evaluate((root) => {
    const rows = Array.from(
      root.querySelectorAll<HTMLElement>('[role="listitem"][data-thread-item-id^="assistant-"]'),
    );
    const row = rows.at(-1) ?? null;
    if (!row) return null;
    const shell = row.closest<HTMLElement>(".wb-pretext-virtualizer-row");
    const planned = Number(shell?.getAttribute("data-pretext-virtualizer-planned-height") ?? Number.NaN);
    const actual = row.getBoundingClientRect().height;
    return {
      id: row.getAttribute("data-thread-item-id") ?? "",
      actual,
      planned,
      delta: planned - actual,
      text: (row.innerText || "").slice(0, 240),
    };
  });
}

async function expectVisibleLatestAssistantParity(page: Page, label: string) {
  await expect
    .poll(async () => {
      const measurement = await readVisibleLatestAssistantParity(page);
      if (!measurement) {
        return { ok: false, reason: "missing-row" };
      }
      return {
        ok: Math.abs(measurement.delta) <= 1,
        id: measurement.id,
        planned: measurement.planned,
        actual: measurement.actual,
        delta: measurement.delta,
        text: measurement.text,
      };
    }, {
      timeout: 30_000,
      message: `visible assistant row should stay in parity during ${label}`,
    })
    .toMatchObject({ ok: true });
}

test("workbench: streaming assistant long message stays in parity through completion and reload", async ({ page, request }) => {
  test.setTimeout(180000);

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 6,
    throttleMs: 5,
    messageBytes: 1000,
    messagePrefix: "live-row parity seed",
  });

  const taskId = seed.taskIds[0];
  const sessionId = seed.sessionIdsByTask[taskId!]?.[0];
  expect(sessionId).toBeTruthy();

  await page.goto(`/workspaces/${seed.workspaceId}?debug=1`, { waitUntil: "domcontentloaded" });
  const task = page.locator(".wb-task-row").filter({ hasText: "fixture task 1" }).first();
  await expect(task).toBeVisible({ timeout: 30_000 });
  await task.click();
  await expect(page.locator('.wb-session-slot [data-testid="session-view"]').first()).toHaveAttribute("data-session-id", sessionId!, {
    timeout: 20_000,
  });

  await request.post(`/api/sessions/${sessionId}/messages`, {
    data: {
      content: `slow-diff-test stream-assistant-partials\n${ASSISTANT_LONG_STATUS_MESSAGE}`,
      delivery: "immediate",
    },
  });

  await expect(
    page.locator('.wb-session-slot [role="listitem"][data-thread-item-id$="-pending"]').first(),
  ).toBeVisible({ timeout: 30_000 });

  for (let sample = 0; sample < 5; sample += 1) {
    await expectVisibleLatestAssistantParity(page, `streaming sample ${sample + 1}`);
    await page.waitForTimeout(250);
  }

  await expect(page.locator(".wb-turn-status-label").last()).toHaveText(/Completed/i, { timeout: 60_000 });
  await expectVisibleLatestAssistantParity(page, "post-complete");

  await page.reload({ waitUntil: "domcontentloaded" });
  const reloadedTask = page.locator(".wb-task-row").filter({ hasText: "fixture task 1" }).first();
  await expect(reloadedTask).toBeVisible({ timeout: 30_000 });
  await reloadedTask.click();
  await expect(page.locator('.wb-session-slot [data-testid="session-view"]').first()).toHaveAttribute("data-session-id", sessionId!, {
    timeout: 20_000,
  });
  await expectVisibleLatestAssistantParity(page, "post-reload");
});
