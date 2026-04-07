import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector = ".wb-session-slot[aria-hidden=\"false\"] [data-pretext-virtualizer-list=\"1\"]";

async function addLongMessages(
  request: Parameters<typeof test>[0]["request"],
  sessionId: string,
  count: number,
) {
  const longText = Array.from({ length: 220 }, (_, index) => `history line ${index + 1}`).join("\n");
  for (let index = 0; index < count; index += 1) {
    await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nwheel-detach ${index + 1}`, delivery: "immediate" },
    });
  }
}

test("workbench: detached upward wheel keeps making progress without pushback", async ({ page, request }) => {
  test.setTimeout(180_000);
  await page.setViewportSize({ width: 1440, height: 900 });

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 10,
    messageBytes: { min: 220, max: 320 },
    messagePrefix: "detached-wheel",
  });

  const taskId = seed.taskIds[0];
  const sessionId = seed.sessionIdsByTask[taskId]?.[0];
  if (!sessionId) {
    throw new Error("Detached-wheel seed did not produce a session");
  }
  await addLongMessages(request, sessionId, 12);

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  await expect(page.locator(".wb-task-row")).toHaveCount(1, { timeout: 20_000 });
  await page.locator(".wb-task-row").first().click();
  const scroller = page.locator(scrollSelector).first();
  await expect(scroller).toBeVisible({ timeout: 20_000 });

  await scroller.evaluate((element) => {
    element.scrollTop = Math.max(0, element.scrollHeight - element.clientHeight);
    element.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(150);
  await scroller.hover();

  const samples: Array<{
    label: string;
    scrollTop: number;
    bottomGap: number;
    pendingRestore: string | null;
    pendingProgrammatic: string | null;
  }> = [];

  const sample = async (label: string) => {
    const snapshot = await scroller.evaluate((element) => ({
      scrollTop: element.scrollTop,
      bottomGap: element.scrollHeight - (element.scrollTop + element.clientHeight),
      pendingRestore: element.getAttribute("data-pretext-virtualizer-pending-restore"),
      pendingProgrammatic: element.getAttribute("data-pretext-virtualizer-programmatic-pending"),
    }));
    samples.push({ label, ...snapshot });
  };

  await sample("start");
  for (const delta of [-60, -50, -40, -30, -20, -10]) {
    await page.mouse.wheel(0, delta);
    await page.waitForTimeout(140);
    await sample(`wheel${delta}`);
  }
  await page.waitForTimeout(1500);
  await sample("settle");

  const wheelSamples = samples.filter((entry) => entry.label.startsWith("wheel"));
  for (let index = 1; index < wheelSamples.length; index += 1) {
    expect(
      wheelSamples[index]!.scrollTop,
      `wheel sample ${wheelSamples[index]!.label} should not reverse upward progress`,
    ).toBeLessThanOrEqual(wheelSamples[index - 1]!.scrollTop);
  }

  const start = samples[0]!;
  const finalWheel = wheelSamples.at(-1)!;
  const settle = samples.at(-1)!;

  expect(finalWheel.bottomGap, "upward wheel should leave the transcript materially detached from bottom").toBeGreaterThanOrEqual(180);
  expect(
    Math.abs(settle.bottomGap - finalWheel.bottomGap),
    "detached upward wheel should not snap back during settle",
  ).toBeLessThanOrEqual(2);
  expect(
    settle.scrollTop,
    "detached settle should stay above the starting bottom position",
  ).toBeLessThan(start.scrollTop);
  expect(settle.pendingRestore).toBe("0");
  expect(settle.pendingProgrammatic).toBe("0");
});
