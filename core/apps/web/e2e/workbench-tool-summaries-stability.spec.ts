import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test.describe.serial("workbench: tool summaries stability", () => {
  let workspaceId = "";
  const toolMarkerFor = (seed: string) =>
    `[[tool_calls]]\n${JSON.stringify([
      {
        kind: "execute",
        title: `Run pwd ${seed}`,
        input: { command: "pwd" },
        output_text: "ok",
      },
    ])}\n[[/tool_calls]]`;

  const composerLocator = (page: any) =>
    page.locator(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea");

  const sendToolSeed = async (page: any, seed: string) => {
    const composer = composerLocator(page);
    await expect(composer).toBeVisible({ timeout: 20_000 });
    await composer.fill(`tool seed ${seed}\n${toolMarkerFor(seed)}`);
    const sendResp = page.waitForResponse((resp: any) => {
      if (resp.request().method() !== "POST") return false;
      if (!/\/api\/sessions\/[^/]+\/messages$/.test(resp.url())) return false;
      const body = resp.request().postData() ?? "";
      return body.includes(`tool seed ${seed}`);
    });
    await page
      .locator(".wb-session-slot[aria-hidden=\"false\"] button[aria-label=\"Send\"]")
      .click();
    const resp = await sendResp;
    expect(resp.status(), `tool seed POST failed: ${resp.url()}`).toBe(200);
  };

  const ensureToolRows = async (page: any) => {
    const toolRows = page.locator(".wb-tool-row");
    if ((await toolRows.count()) > 0) return toolRows;
    await sendToolSeed(page, `${Date.now()}`);
    await expect
      .poll(async () => toolRows.count(), { timeout: 30_000 })
      .toBeGreaterThan(0);
    return toolRows;
  };

  const ensureScrollableThread = async (page: any, scroller: any, toolRows: any) => {
    for (let attempt = 0; attempt < 8; attempt += 1) {
      const overflow = await scroller.evaluate(
        (el: HTMLElement) => el.scrollHeight - el.clientHeight,
      );
      if (overflow > 160) return;

      const prevCount = await toolRows.count();
      await sendToolSeed(page, `${Date.now()}-${attempt}`);
      await expect
        .poll(async () => toolRows.count(), { timeout: 30_000 })
        .toBeGreaterThan(prevCount);
    }

    await expect
      .poll(
        async () =>
          scroller.evaluate(
            (el: HTMLElement) => el.scrollHeight - el.clientHeight,
          ),
        { timeout: 30_000 },
      )
      .toBeGreaterThan(160);
  };

  test.beforeAll(async ({ request }) => {
    const seed = await seedDummyWorkspace(request, {
      tasks: 1,
      sessionsPerTask: 1,
      turnsPerSession: 1,
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

    const toolRows = await ensureToolRows(page);

    const scroller = page
      .locator(".wb-session-slot[aria-hidden=\"false\"] .wb-thread-scroller")
      .first();
    await expect
      .poll(async () => scroller.evaluate((el) => el.scrollHeight), { timeout: 10_000 })
      .toBeGreaterThan(0);

    await ensureScrollableThread(page, scroller, toolRows);

    await scroller.hover();
    await page.mouse.wheel(0, 400);

    await page.waitForTimeout(400);
    const scrollTop = await scroller.evaluate((el: HTMLElement) => el.scrollTop);
    expect(scrollTop).toBeGreaterThan(100);

    await page.screenshot({
      path: test.info().outputPath("scroll-stable.png"),
      fullPage: false,
    });
  });
});
