import fs from "node:fs";
import { test, expect } from "./fixtures";
import type { APIRequestContext } from "playwright/test";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";
import { analyzeScrollJankVideo, canUseFfmpeg } from "./utils/videoJankAnalyzer";

const scrollSelector = ".wb-session-slot[aria-hidden=\"false\"] .wb-thread-scroller";

const shouldRun = process.env.CTX_E2E_JANK_VIDEO === "1";
test.use({ video: shouldRun ? "on" : "off" });

const describe = shouldRun ? test.describe : test.describe.skip;

describe("workbench: scrollback jank video", () => {
  async function addLongMessages(request: APIRequestContext, sessionId: string, count: number) {
    const longText = Array.from({ length: 220 }, (_, i) => `history line ${i + 1}`).join("\n");
    for (let i = 0; i < count; i += 1) {
      await request.post(`/api/sessions/${sessionId}/messages`, {
        data: { content: `${longText}\nhistory scroll ${i + 1}`, delivery: "immediate" },
      });
    }
  }

  test("workbench: detect scrollback jank during upward scroll", async ({ page, request }, testInfo) => {
    test.setTimeout(180000);
    if (!canUseFfmpeg()) {
      test.skip(true, "ffmpeg not available");
    }

    await page.setViewportSize({ width: 1400, height: 900 });

    const seed = await seedDummyWorkspace(request, {
      tasks: 1,
      sessionsPerTask: 1,
      turnsPerSession: 10,
      messageBytes: { min: 220, max: 320 },
      messagePrefix: "scrollback-jank",
    });

    const taskId = seed.taskIds[0];
    const sessionId = seed.sessionIdsByTask[taskId][0];
    await addLongMessages(request, sessionId, 12);

    await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
    const rows = page.locator(".wb-task-row");
    await expect(rows).toHaveCount(1, { timeout: 20000 });
    await rows.first().click();

    await expect(page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea")).toBeVisible({
      timeout: 20000,
    });

    const scroller = page.locator(scrollSelector).first();
    await expect(scroller).toBeVisible({ timeout: 20000 });

    await scroller.evaluate((el) => {
      el.scrollTop = Math.max(0, el.scrollHeight - el.clientHeight);
      el.dispatchEvent(new Event("scroll"));
    });
    await page.waitForTimeout(100);

    const historyResponse = page
      .waitForResponse((response) => response.url().includes(`/api/sessions/${sessionId}/history`), {
        timeout: 30000,
      })
      .catch(() => null);

    await scroller.hover();
    for (let i = 0; i < 40; i += 1) {
      await page.mouse.wheel(0, -240);
      await page.waitForTimeout(60);
    }

    await historyResponse;
    await page.waitForTimeout(1200);

    const video = page.video();
    await page.close();
    const videoPath = video ? await video.path() : null;
    if (!videoPath || !fs.existsSync(videoPath)) {
      throw new Error("Playwright video not found for scrollback jank analysis.");
    }

    const thresholdPx = Number(process.env.CTX_E2E_JANK_SHIFT_PX ?? 20);
    const skipFrames = Number(process.env.CTX_E2E_JANK_SKIP_FRAMES ?? 20);
    const analysis = analyzeScrollJankVideo(videoPath, {
      outputDir: testInfo.outputDir,
      fps: 10,
      scaleWidth: 960,
      maxShiftPx: 40,
      thresholdPx,
      jankThresholdPx: thresholdPx,
      skipFrames,
      minBaselineAbsPx: 2,
    });

    await testInfo.attach("scrollback-jank-summary.json", {
      path: analysis.summaryPath,
      contentType: "application/json",
    });
    await testInfo.attach("scrollback-jank-shifts.csv", {
      path: analysis.csvPath,
      contentType: "text/csv",
    });
    await testInfo.attach("scrollback-jank-shifts.json", {
      path: analysis.jsonPath,
      contentType: "application/json",
    });
    for (const diffPath of analysis.diffPaths) {
      await testInfo.attach(`scrollback-jank-${diffPath.split("/").pop()}`, {
        path: diffPath,
        contentType: "image/png",
      });
    }

    expect(analysis.rows.length).toBeGreaterThan(5);
    expect(analysis.maxJankAbsShift).toBeLessThanOrEqual(thresholdPx);
  });
});
