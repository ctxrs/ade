import { expect, test, type Page } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

type MarkdownSample = {
  name: string;
  markdown: string;
};

type E2EWindow = Window & {
  __ctxE2E?: {
    measureMarkdownParity?: (samples: readonly MarkdownSample[], width: number) => Promise<Array<{
      name: string;
      planned: number;
      actual: number;
      delta: number;
    }>>;
    measureMarkdownSelectionText?: (markdown: string, width: number) => Promise<string>;
    installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
    removeMarkdownScrollProbe?: () => boolean;
  };
};

async function openEmptyWorkspace(page: Page) {
  const seed = await seedDummyWorkspace(page.request, {
    tasks: 0,
    sessionsPerTask: 0,
    turnsPerSession: 0,
  });
  await page.goto(`/workspaces/${seed.workspaceId}?ctxE2E=1`, { waitUntil: "domcontentloaded" });
}

async function measureMarkdownParity(page: Page, samples: readonly MarkdownSample[], width: number) {
  return page.evaluate(({ samples, width }) => {
    return (window as E2EWindow).__ctxE2E?.measureMarkdownParity?.(samples, width) ?? Promise.resolve([]);
  }, { samples, width });
}

async function measureMarkdownSelectionText(page: Page, markdown: string, width: number) {
  return page.evaluate(({ markdown, width }) => {
    return (window as E2EWindow).__ctxE2E?.measureMarkdownSelectionText?.(markdown, width) ?? Promise.resolve("");
  }, { markdown, width });
}

test("workbench: deterministic markdown planner matches rendered block geometry", async ({ page }) => {
  test.setTimeout(120000);
  await openEmptyWorkspace(page);

  const samples: MarkdownSample[] = [
    {
      name: "inline-code-list",
      markdown: String.raw`A paragraph with inline code like \`pnpm -C core/apps/web typecheck\` and a long URL \`http://192.0.2.19:5182/workspaces/00000000-0000-4000-8000-000000000001?token=00000000-0000-4000-8000-000000000002\`.

- bullet with \`ctx-devapp-5182\`
- second bullet with \`testing/documentation\``,
    },
    {
      name: "fenced-code-short",
      markdown: "Before\n\n```ts\nconst value = 1;\nconsole.log(value);\n```\n\nAfter",
    },
    {
      name: "fenced-code-long-line",
      markdown:
        "Before\n\n```bash\npnpm -C core/apps/web exec playwright test -c playwright.pretext-virtualizer-acceptance.config.ts --grep \"rich markdown assistant rows\"\n```\n\nAfter",
    },
    {
      name: "blockquote-code",
      markdown: "> quoted with `inline code`\n>\n> second line\n\n```txt\nhello\nworld\n```",
    },
    {
      name: "table-inline",
      markdown:
        "| Day | Count | Note |\n|---|---:|---|\n| 2026-04-06 | 8 | `ctx-devapp-5182` |\n| 2026-04-07 | 13 | `testing/documentation` |",
    },
    {
      name: "nested-list",
      markdown: "- outer\n  - nested item with `inline code`\n  - nested two\n- outer two",
    },
    {
      name: "assistant-tail-inline-code-url",
      markdown: String.raw`The synthetic example workspace is available at \`http://192.0.2.19:5182/workspaces/00000000-0000-4000-8000-000000000001?token=00000000-0000-4000-8000-000000000002\`.`,
    },
  ];

  const result = await measureMarkdownParity(page, samples, 788);
  for (const sample of result) {
    expect(
      Math.abs(sample.delta),
      `${sample.name} drifted by ${sample.delta}px (planned ${sample.planned}, actual ${sample.actual})`,
    ).toBeLessThanOrEqual(0.5);
  }
});

test("workbench: vertical wheel over code blocks and tables still scrolls the transcript", async ({ page }) => {
  test.setTimeout(120000);
  await openEmptyWorkspace(page);

  await page.evaluate((markdown) => {
    return (window as E2EWindow).__ctxE2E?.installMarkdownScrollProbe?.(markdown, 788) ?? Promise.resolve(false);
  }, [
    "Before",
    "",
    "```bash",
    "pnpm -C core/apps/web exec playwright test -c playwright.pretext-virtualizer-acceptance.config.ts --grep \"rich markdown assistant rows\"",
    "```",
    "",
    "| Day | Count | Note |",
    "|---|---:|---|",
    "| 2026-04-06 | 8 | a very wide table cell that should require horizontal scrolling |",
    "| 2026-04-07 | 13 | another very wide table cell that should require horizontal scrolling |",
    "",
    "After",
  ].join("\n"));

  const scroller = page.locator("#markdown-scroll-probe [data-pretext-virtualizer-list='1']");
  await expect(scroller).toBeVisible();

  const codeBlock = page.locator("#markdown-scroll-probe .codeblock-pre");
  await expect(codeBlock).toBeVisible();
  const beforeCodeWheel = await scroller.evaluate((el) => el.scrollTop);
  await codeBlock.hover();
  await page.mouse.wheel(0, -240);
  await expect
    .poll(async () => scroller.evaluate((el) => el.scrollTop))
    .toBeLessThan(beforeCodeWheel);

  await scroller.evaluate((el) => {
    el.scrollTop = 520;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  const table = page.locator("#markdown-scroll-probe .wb-md-table-scroll");
  await expect(table).toBeVisible();
  const beforeTableWheel = await scroller.evaluate((el) => el.scrollTop);
  await table.hover();
  await page.mouse.wheel(0, -240);
  await expect
    .poll(async () => scroller.evaluate((el) => el.scrollTop))
    .toBeLessThan(beforeTableWheel);

  await page.evaluate(() => {
    (window as E2EWindow).__ctxE2E?.removeMarkdownScrollProbe?.();
  });
});

test("workbench: normal markdown selection text includes explicit list markers", async ({ page }) => {
  test.setTimeout(120000);
  await openEmptyWorkspace(page);

  const selectionText = await measureMarkdownSelectionText(
    page,
    ["1. first item", "2. second item", "", "- bullet item"].join("\n"),
    788,
  );

  expect(selectionText).toContain("1.");
  expect(selectionText).toContain("2.");
  expect(selectionText).toContain("•");
  expect(selectionText).toContain("bullet item");
});
