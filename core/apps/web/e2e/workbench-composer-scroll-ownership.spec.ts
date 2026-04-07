import { expect, test } from "./fixtures";
import type { APIRequestContext } from "playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

const scrollSelector = '.wb-session-slot[aria-hidden="false"] .wb-thread-scroller';

async function addLongMessages(request: APIRequestContext, sessionId: string) {
  const longText = Array.from({ length: 220 }, (_, index) => `scroll ownership line ${index + 1}`).join("\n");
  for (let index = 0; index < 8; index += 1) {
    const response = await request.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: `${longText}\nblock ${index + 1}`, delivery: "immediate" },
    });
    expect(response.ok()).toBeTruthy();
  }
}

test("workbench: composer wheel ownership never moves both the textarea and transcript for one gesture", async ({
  page,
  request,
}) => {
  test.setTimeout(120000);
  await page.setViewportSize({ width: 1440, height: 960 });

  const seed = await seedDummyWorkspace(request, {
    tasks: 1,
    sessionsPerTask: 1,
    turnsPerSession: 0,
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20000 });
  await rows.first().click();

  const composer = page.locator('.wb-session-slot[aria-hidden="false"] textarea.wb-active-textarea');
  await expect(composer).toBeVisible({ timeout: 20000 });

  const sessionId = seed.sessionIdsByTask[seed.taskIds[0]][0];
  await addLongMessages(request, sessionId);
  await expect(page.locator(".wb-session")).toContainText("block 8", { timeout: 20000 });

  const scroller = page.locator(scrollSelector).first();
  await expect(scroller).toBeVisible({ timeout: 20000 });
  await expect
    .poll(async () => scroller.evaluate((element) => element.scrollHeight - element.clientHeight), {
      timeout: 10000,
    })
    .toBeGreaterThan(180);

  const overflowingComposerText = Array.from(
    { length: 40 },
    (_, index) => `composer overflow ${index + 1} ${"x".repeat(48)}`,
  ).join("\n");
  await composer.fill(overflowingComposerText);
  await expect
    .poll(
      async () =>
        composer.evaluate((element) => ({
          scrollHeight: element.scrollHeight,
          clientHeight: element.clientHeight,
        })),
      { timeout: 5000 },
    )
    .toMatchObject({});
  await expect
    .poll(async () => composer.evaluate((element) => element.clientHeight), { timeout: 5000 })
    .toBeGreaterThanOrEqual(200);
  await expect
    .poll(async () => composer.evaluate((element) => element.scrollHeight - element.clientHeight), {
      timeout: 5000,
    })
    .toBeGreaterThan(120);

  await scroller.evaluate((element) => {
    element.scrollTop = 720;
  });
  await composer.evaluate((element) => {
    element.scrollTop = 40;
  });

  const composerOwnedBefore = {
    composerTop: await composer.evaluate((element) => element.scrollTop),
    scrollerTop: await scroller.evaluate((element) => element.scrollTop),
  };

  await composer.hover();
  await page.mouse.wheel(0, 180);

  await expect
    .poll(async () => composer.evaluate((element) => element.scrollTop), { timeout: 2000 })
    .toBeGreaterThan(composerOwnedBefore.composerTop + 20);
  const scrollerAfterComposerOwned = await scroller.evaluate((element) => element.scrollTop);
  expect(Math.abs(scrollerAfterComposerOwned - composerOwnedBefore.scrollerTop)).toBeLessThanOrEqual(4);

  await scroller.evaluate((element) => {
    element.scrollTop = 720;
  });
  await composer.evaluate((element) => {
    element.scrollTop = 0;
  });

  const transcriptOwnedBefore = {
    composerTop: await composer.evaluate((element) => element.scrollTop),
    scrollerTop: await scroller.evaluate((element) => element.scrollTop),
  };

  await composer.hover();
  await page.mouse.wheel(0, -220);

  await expect
    .poll(async () => scroller.evaluate((element) => element.scrollTop), { timeout: 2000 })
    .toBeLessThan(transcriptOwnedBefore.scrollerTop - 20);
  const composerAfterTranscriptOwned = await composer.evaluate((element) => element.scrollTop);
  expect(composerAfterTranscriptOwned).toBeLessThanOrEqual(transcriptOwnedBefore.composerTop + 1);
});
