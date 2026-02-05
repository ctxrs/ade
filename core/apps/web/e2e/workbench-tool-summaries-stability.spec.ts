import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test.describe.serial("workbench: tool summaries stability", () => {
  let workspaceId = "";
  const toolMarker = `[[tool_calls]]\n${JSON.stringify([
    {
      kind: "execute",
      title: "Run pwd",
      input: { command: "pwd" },
      output_text: "ok",
    },
  ])}\n[[/tool_calls]]`;

  const ensureToolRows = async (page: any) => {
    const toolRows = page.locator(".wb-tool-row");
    if ((await toolRows.count()) > 0) return toolRows;
    const composer = page.locator(
      ".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea",
    );
    await expect(composer).toBeVisible({ timeout: 20000 });
    await composer.fill(`tool seed ${Date.now()}\n${toolMarker}`);
    await page
      .locator(".wb-session-slot[aria-hidden=\"false\"] button[aria-label=\"Send\"]")
      .click();
    await expect
      .poll(async () => toolRows.count(), { timeout: 30000 })
      .toBeGreaterThan(0);
    return toolRows;
  };

  test.beforeAll(async ({ request }) => {
    const seed = await seedDummyWorkspace(request, {
      tasks: 1,
      sessionsPerTask: 1,
      turnsPerSession: 0,
    });
    workspaceId = seed.workspaceId;
  });

  test("tool summaries render without waterfall", async ({ page }) => {
    await page.goto(`/workspaces/${workspaceId}`, { waitUntil: "domcontentloaded" });
    await page.locator(".wb-task-row").first().click();

    const toolRows = await ensureToolRows(page);

    const sampleCounts: number[] = [];
    const delays = [0, 150, 300, 600];
    for (const delay of delays) {
      if (delay > 0) {
        await page.waitForTimeout(delay);
      }
      sampleCounts.push(await toolRows.count());
    }

    const min = Math.min(...sampleCounts);
    const max = Math.max(...sampleCounts);
    expect(max - min).toBeLessThanOrEqual(1);

    await page.screenshot({
      path: test.info().outputPath("tool-summaries-stable.png"),
      fullPage: false,
    });
  });

  test("scroll position does not snap back", async ({ page }) => {
    await page.goto(`/workspaces/${workspaceId}`, { waitUntil: "domcontentloaded" });
    await page.locator(".wb-task-row").first().click();

    await ensureToolRows(page);

    const scroller = page
      .locator(".wb-session-slot[aria-hidden=\"false\"] [data-virtuoso-scroller]")
      .first();
    await expect
      .poll(async () => scroller.evaluate((el) => el.scrollHeight), { timeout: 10_000 })
      .toBeGreaterThan(0);

    await scroller.hover();
    await page.mouse.wheel(0, 400);

    await page.waitForTimeout(400);
    const scrollTop = await scroller.evaluate((el) => el.scrollTop);
    expect(scrollTop).toBeGreaterThan(100);

    await page.screenshot({
      path: test.info().outputPath("scroll-stable.png"),
      fullPage: false,
    });
  });
});
