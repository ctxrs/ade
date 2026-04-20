import { promises as fs } from "fs";
import { expect, test } from "./fixtures";
import {
  type MarkdownSample,
  measureAssistantParity,
  measureMarkdownParity,
  measureMessageParity,
  measureTurnHeaderParity,
  openWorkbenchShell,
} from "./utils/pretextParity";

const ENFORCE = process.env.CTX_PRETEXT_PARITY_ENFORCE === "1";
const WIDTHS = [540, 620, 788];
const THRESHOLD_WIDTHS = [472, 768, 788];
const MARKDOWN_THRESHOLD_PX = 1;
const ROW_THRESHOLD_PX = 1;

const IMAGE_DATA_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO2VzJ8AAAAASUVORK5CYII=";

const MARKDOWN_CORPUS: MarkdownSample[] = [
  {
    name: "two-chip-threshold-wrap",
    markdown:
      "The fixture document contains `7` active rows, including `Example Taxonomy`, so the threshold sample has stable code chips.",
  },
  {
    name: "adjacent-inline-code",
    markdown:
      "Paragraph with `alpha-beta-gamma-delta/ctx/path/one` `second-inline-token/with/path/two` beside prose and punctuation.",
  },
  {
    name: "code-comma-tail",
    markdown: "Reopen `origin/main`, then inspect the queue again.",
  },
  {
    name: "three-chip-prose",
    markdown: "Compare `origin/main`, `Test Taxonomy`, and `ctx serve` before replying.",
  },
  {
    name: "period-after-code-tail",
    markdown: "We shipped `ctx serve`. Then we reopened `origin/main` again.",
  },
  {
    name: "colon-command-chip-after-prose",
    markdown:
      "This sample keeps the inline measurement path, viewport controller, and `verify:quick` label in one long paragraph. The extra prose gives the browser enough ordinary text to exercise a code-chip seam without relying on a recorded work log.",
  },
  {
    name: "whitespace-code-comma-tail-long-prose",
    markdown:
      "The current fixture is concrete and repeatable: `sample-layout-runner` uses a helper with Python `shutil.copytree(..., dirs_exist_ok=True)`, which keeps the long inline-code tail beside ordinary prose. The width sample stays useful while the browser comparison remains deterministic.",
  },
  {
    name: "inline-code-link-prose",
    markdown:
      "Use [`ctx docs`](https://example.com/docs) with `pnpm -C core/apps/web test:e2e:pretext:parity:webkit` and more prose to wrap near the edge.",
  },
  {
    name: "blockquote-list-code",
    markdown:
      "> quoted intro with `inline-code-token`\n>\n> second line with a [link](https://example.com/path)\n\n- bullet with `nested-inline-code`\n- bullet with trailing prose for wrap pressure",
  },
  {
    name: "fenced-code-long-line",
    markdown:
      "Before\n\n```bash\npnpm -C core/apps/web exec playwright test -c playwright.pretext-virtualizer-acceptance.config.ts e2e/workbench-pretext-parity-corpus.spec.ts --browser=webkit --workers=1\n```\n\nAfter",
  },
  {
    name: "table-inline-code",
    markdown:
      "| Env | Command | Note |\n|---|---|---|\n| dev | `pnpm dev` | wraps with prose and punctuation |\n| test | `pnpm -C core/apps/web test:e2e:pretext:parity:chromium` | very long note that should keep wrapping |",
  },
  {
    name: "table-inline-command-threshold",
    markdown:
      "| Left | Token | Note |\n| --- | --- | --- |\n| fragment session | `pnpm -C core/apps/web test:e2e:pretext:parity:webkit` | entry header fragment virtualizer summary fragment thread composer |",
  },
  {
    name: "hard-break-inline-code",
    markdown:
      "First line with `ctx run --mode strict`\nsecond line with `very-long-inline-token/that/should/wrap` and more prose after it.",
  },
  {
    name: "nested-list-wrap",
    markdown:
      "- outer item with `inline-code-token`\n  - nested item with long prose and `ctx/pretext/pathlike/token`\n  - nested sibling with [docs](https://example.com) and punctuation",
  },
  {
    name: "emoji-cjk-inline-code",
    markdown: "Status check 🙂 with `inline-token-path/segment` and mixed CJK text 你好 世界 to stress shaping.",
  },
  {
    name: "emoji-cjk-inline-code-threshold",
    markdown:
      "summary 🙂 測試 佈局 `core/e2e/pretextVirtualizerRowLayout.ts/sessionThreadDomMeasurement.tsx/apps/turn-header`.",
  },
  {
    name: "blockquote-link-code-tail",
    markdown:
      "> [summary message summary](https://example.com/inline-code/parity/webkit/parity?ref=781) `cargo test -p ctx-store`:",
  },
];

type SoftBreakMarkdownSample = MarkdownSample & {
  width: number;
  browsers?: Array<"chromium" | "webkit">;
};

const SOFT_BREAK_MARKDOWN_REGRESSIONS: SoftBreakMarkdownSample[] = [
  {
    name: "soft-break-path-start-later-fragment",
    width: 518.64,
    markdown: [
      "Buffer session summary ~~turn~~ ⚙️ 你好 世界 `core/blockquote/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx` ~~command~~ ~~command command~~ **context**: `workbenchShell/fixtures/turn-header/table`",
      "Stream browser agent ⚙️ 測試 佈局 header deterministic thread summary buffer entry context ⚙️ 你好 世界 `web/fixtures/core/fixtures`. `pnpm -C core/apps/web test:e2e:pretext:corpus:webkit`",
    ].join("\n"),
  },
  {
    name: "soft-break-mixed-inline-seams",
    width: 445.04,
    markdown: [
      "Fragment fragment command [thread deterministic](https://example.com/webkit/docs/chromium/transcript?ref=917) `pnpm -C core/apps/web test:e2e:pretext:guardrail` **render** 🧪 段落 換行; `sessionMarkdownMeasurement.ts/pretextVirtualizerRowLayout.ts/src/core`",
      "Command message composer buffer *session parity* 🙂 測試 佈局 *virtualizer buffer*; `git status`",
      "Thread inline `sessionMarkdownMeasurement.ts/e2e/e2e/web/turn-header/turn-header` *stream entry browser* `pnpm -C core/apps/web test:e2e:pretext:parity:chromium` 📏 你好 世界. `turn-header/pages/sessionThreadDomMeasurement.tsx/pages/table/e2e`",
      "Parity render probe fragment *layout* `git rev-parse HEAD` **context** buffer stream `e2e/pretextVirtualizerRowLayout.ts/apps/table/table/src`. `pnpm -C core/apps/web test:e2e:pretext:parity:chromium`",
    ].join("\n"),
  },
  {
    name: "soft-break-fresh-line-path-full-chrome",
    width: 516,
    markdown: [
      "Fragment fragment command [thread deterministic](https://example.com/webkit/docs/chromium/transcript?ref=917) `pnpm -C core/apps/web test:e2e:pretext:guardrail` **render** 🧪 段落 換行; `sessionMarkdownMeasurement.ts/pretextVirtualizerRowLayout.ts/src/core`",
      "Command message composer buffer *session parity* 🙂 測試 佈局 *virtualizer buffer*; `git status`",
      "Thread inline `sessionMarkdownMeasurement.ts/e2e/e2e/web/turn-header/turn-header` *stream entry browser* `pnpm -C core/apps/web test:e2e:pretext:parity:chromium` 📏 你好 世界. `turn-header/pages/sessionThreadDomMeasurement.tsx/pages/table/e2e`",
    ].join("\n"),
  },
  {
    name: "soft-break-friendly-boundary-prose-tail",
    width: 445.04,
    markdown: [
      "Delta agent browser entry `sessionThread/src/sessionThread/e2e/sessionThread/table` `table/sessionThread/apps/workbenchShell/fixtures/turn-header` inline pretext parity. `table/web/fixtures/table`",
      "Message deterministic virtualizer `apps/workbenchShell/src/src/sessionMarkdownMeasurement.ts/e2e` *pretext summary turn* padding fragment thread turn context pretext turn buffer summary [stream pretext](https://example.com/transcript/webkit?ref=266). `cargo test -p ctx-http`",
    ].join("\n"),
  },
  {
    name: "soft-break-attached-trailing-plain-path-start",
    width: 445.04,
    markdown: [
      "Render deterministic [command session deterministic](https://example.com/docs/inline-code/measurement?ref=150) *token virtualizer* ~~agent shell~~ session parity shell inline fragment; `e2e/turn-header/workbenchShell/table`",
      "Virtualizer probe render 🙂 段落 換行 *composer browser render* session browser header layout command session layout parity command: `sessionThread/table/web/workbenchShell/apps/blockquote`",
      "Stream token stream buffer entry fragment marker 📏 段落 換行 ~~token token~~ [buffer](https://example.com/parity/streaming-tail?ref=574) 🧪 段落 換行 ~~fragment~~. `ctx run start --mode sandbox`",
    ].join("\n"),
  },
  {
    name: "soft-break-wide-glyph-friendly-boundary",
    width: 445.04,
    browsers: ["webkit"],
    markdown: [
      "Layout composer fragment padding `sessionMarkdownMeasurement.ts/src/sessionThreadDomMeasurement.tsx/web/blockquote/workbenchShell/workbenchShell` **command inline** `cargo test -p ctx-store` ⚙️ 測試 佈局 [agent fragment](https://example.com/transcript/chromium?ref=812) *summary fragment*. `apps/fixtures/src/src/sessionMarkdownMeasurement.ts/pages`",
      "Virtualizer probe render 🙂 段落 換行 *composer browser render* session browser header layout command session layout parity command: `sessionThread/table/web/workbenchShell/apps/blockquote`",
    ].join("\n"),
  },
  {
    name: "soft-break-command-option-hyphen-tail",
    width: 382.48,
    markdown: [
      "Parity render probe fragment *layout* `git rev-parse HEAD` **context** buffer stream `e2e/pretextVirtualizerRowLayout.ts/apps/table/table/src`. `pnpm -C core/apps/web test:e2e:pretext:parity:chromium`",
      "Entry shell agent command buffer agent command inline browser token deterministic padding [agent](https://example.com/streaming-tail/transcript/streaming-tail/transcript?ref=527) [agent composer render](https://example.com/measurement/webkit?ref=119): `pnpm -C core/apps/web test:e2e:pretext:parity:webkit`",
    ].join("\n"),
  },
  {
    name: "soft-break-command-leading-hang",
    width: 382.48,
    markdown: [
      "Layout composer fragment padding `sessionMarkdownMeasurement.ts/src/sessionThreadDomMeasurement.tsx/web/blockquote/workbenchShell/workbenchShell` command inline `cargo test -p ctx-store` agent fragment summary fragment. `apps/fixtures/src/src/sessionMarkdownMeasurement.ts/pages`",
      "Inline stream browser context thread layout padding fragment header probe session summary token inline; `git rev-parse HEAD`",
      "Render deterministic command session deterministic token virtualizer agent shell session parity shell inline fragment; `e2e/turn-header/workbenchShell/table`",
    ].join("\n"),
  },
  {
    name: "list-inline-code-prose-tail",
    width: 588,
    markdown:
      "- Command fragment [stream layout](https://example.com/docs/parity/webkit?ref=298) *agent virtualizer* ~~inline~~ `pages/fixtures/sessionMarkdownMeasurement.ts/sessionMarkdownMeasurement.ts/blockquote/inline-code/e2e` render parity composer probe render header render summary;",
  },
  {
    name: "path-inline-code-prose-tail",
    width: 564,
    markdown:
      "Command fragment [stream layout](https://example.com/docs/parity/webkit?ref=298) *agent virtualizer* ~~inline~~ `pages/fixtures/sessionMarkdownMeasurement.ts/sessionMarkdownMeasurement.ts/blockquote/inline-code/e2e` render parity composer probe render header render summary;",
  },
  {
    name: "longer-path-prefix-code-only",
    width: 382.48,
    markdown: "- Delta parity `table/fixtures/sessionThreadDomMeasurement.tsx/workbenchShell`",
  },
  {
    name: "decorated-tail-after-path-code",
    width: 620,
    markdown:
      "- Agent token marker `src/src/pretextVirtualizerRowLayout.ts/core/blockquote` *padding entry* **marker stream**:",
  },
];

const USER_MESSAGE_CORPUS = [
  {
    name: "multi-paragraph-user",
    params: {
      content: [
        "This is a longer user message with a paragraph that should wrap comfortably inside the message bubble.",
        "It also includes `inline-code-token/with/path/segments` plus more prose afterward to keep the wrap pressure realistic.",
        "The final paragraph mentions `pnpm -C core/apps/web test:e2e:pretext:parity:chromium` to mimic real commands.",
      ].join("\n\n"),
      expanded: true,
    },
  },
  {
    name: "threshold-user-tail",
    params: {
      content: [
        "Keep `7` active tasks, including `Test Taxonomy`, aligned before replying.",
        "Compare `origin/main`, `ctx serve`, and `stable lane` again after the transcript reload.",
      ].join("\n\n"),
      expanded: true,
    },
  },
  {
    name: "collapsed-user",
    params: {
      content: Array.from(
        { length: 28 },
        (_, index) => `line ${index + 1} with enough words to wrap and keep the collapsed toggle alive`,
      ).join("\n"),
      expanded: false,
    },
  },
  {
    name: "user-with-images",
    params: {
      content: "two inline screenshots and a path `ctx/pretext/harness/attachment-check`",
      expanded: true,
      attachments: [
        { kind: "image" as const, mime_type: "image/png", data_base64: IMAGE_DATA_BASE64, name: "one.png" },
        { kind: "image" as const, mime_type: "image/png", data_base64: IMAGE_DATA_BASE64, name: "two.png" },
      ],
    },
  },
];

const ASSISTANT_CORPUS = [
  {
    name: "two-chip-threshold-wrap",
    params: {
      content:
        "The fixture document contains `7` active rows, including `Example Taxonomy`, so the threshold sample has stable code chips.",
    },
  },
  {
    name: "wrapped-inline-code",
    params: {
      content:
        "begin agent message with plain text and now `inline-thing-that-actually-gets-really-long-so-much-so-that-it-wraps-to-multiple-lines/core/apps/web/src/pages/sessionThread/sessionMarkdownMeasurement.ts` after the prose",
    },
  },
  {
    name: "adjacent-inline-code",
    params: {
      content:
        "multiple chips `first-very-long-token/with/path/a` and `second-even-longer-token/with/path/b` in one sentence with trailing prose",
    },
  },
  {
    name: "code-comma-tail",
    params: {
      content: "Reopen `origin/main`, then inspect the queue again.",
    },
  },
  {
    name: "three-chip-prose",
    params: {
      content: "Compare `origin/main`, `Test Taxonomy`, and `ctx serve` before replying.",
    },
  },
  {
    name: "period-after-code-tail",
    params: {
      content: "We shipped `ctx serve`. Then we reopened `origin/main` again.",
    },
  },
  {
    name: "colon-command-chip-after-prose",
    params: {
      content:
        "This sample keeps the inline measurement path, viewport controller, and `verify:quick` label in one long paragraph. The extra prose gives the browser enough ordinary text to exercise a code-chip seam without relying on a recorded work log.",
    },
  },
  {
    name: "whitespace-code-comma-tail-long-prose",
    params: {
      content:
        "The current fixture is concrete and repeatable: `sample-layout-runner` uses a helper with Python `shutil.copytree(..., dirs_exist_ok=True)`, which keeps the long inline-code tail beside ordinary prose. The width sample stays useful while the browser comparison remains deterministic.",
    },
  },
  {
    name: "inline-code-link-mix",
    params: {
      content:
        "Check [`docs`](https://example.com/docs) and run `pnpm -C core/apps/web test:e2e:pretext:parity:webkit` before replying.",
    },
  },
  {
    name: "blockquote-and-code",
    params: {
      content: "> quote with `inline code`\n>\n> second line\n\nfollowed by prose after the quote for more wrapping.",
    },
  },
  {
    name: "list-fence-mix",
    params: {
      content: "- bullet one with `inline-code-token`\n- bullet two\n\n```ts\nconst value = 'ctx';\nconsole.log(value);\n```",
    },
  },
  {
    name: "streaming-tail",
    params: {
      content: "Before\n\n- partial item with `inline-tail-token/with/path`",
      isComplete: false,
    },
  },
];

const THRESHOLD_MARKDOWN_SAMPLE_NAMES = new Set([
  "two-chip-threshold-wrap",
  "code-comma-tail",
  "three-chip-prose",
  "period-after-code-tail",
  "colon-command-chip-after-prose",
]);
const THRESHOLD_USER_SAMPLE_NAMES = new Set(["threshold-user-tail"]);
const THRESHOLD_ASSISTANT_SAMPLE_NAMES = new Set([
  "two-chip-threshold-wrap",
  "code-comma-tail",
  "three-chip-prose",
  "period-after-code-tail",
  "colon-command-chip-after-prose",
]);

const TURN_HEADER_CORPUS = [
  {
    name: "url-heavy",
    params: {
      content: [
        "- https://example.com/a/really/long/path/that/keeps/wrapping?token=12345 should not drift when wrapped inside the turn header bubble.",
        "- Follow-up line with /Users/example-user/.ctx/worktrees/00000000-0000-4000-8000-000000000001 path pressure.",
      ].join("\n"),
    },
  },
  {
    name: "multiline-wrap",
    params: {
      content: [
        "Need follow-up on the long transcript planner path and the completion row interaction.",
        "Also verify that narrow widths keep the header height in sync after wrap pressure.",
      ].join("\n"),
    },
  },
  {
    name: "url-command-path-threshold",
    params: {
      content: [
        "Message summary context https://example.com/assistant/streaming-tail/streaming-tail?ref=656 e2e/fixtures/src/web/pages/turn-header.",
        "Summary layout thread pnpm -C core/apps/web test:e2e:pretext:parity:chromium workbenchShell/pretextVirtualizerRowLayout.ts/inline-code/e2e.",
        "Marker stream entry summary session 測試 佈局 🙂.",
      ].join("\n"),
    },
  },
];

type MarkdownSummaryEntry = {
  width: number;
  name: string;
  planned: number;
  actual: number;
  delta: number;
};

type RowSummaryEntry = {
  kind: "message" | "assistant" | "turn_header";
  width: number;
  name: string;
  planned: number;
  actual: number;
  delta: number;
};

function formatFailures(kind: string, failures: Array<{ name: string; width: number; delta: number; planned: number; actual: number }>): string {
  return `${kind} parity failures:\n${failures
    .map((failure) => `- width ${failure.width} ${failure.name}: delta=${failure.delta}px planned=${failure.planned} actual=${failure.actual}`)
    .join("\n")}`;
}

test("workbench: pretext markdown corpus parity sweep", async ({ page }, testInfo) => {
  test.setTimeout(180000);
  test.slow();
  await openWorkbenchShell(page);

  const summary: MarkdownSummaryEntry[] = [];
  for (const width of WIDTHS) {
    const measurements = await measureMarkdownParity(page, MARKDOWN_CORPUS, width);
    summary.push(...measurements.map((measurement) => ({ width, ...measurement })));
  }

  const report = {
    enforce: ENFORCE,
    thresholdPx: MARKDOWN_THRESHOLD_PX,
    widths: WIDTHS,
    samples: summary,
  };
  const reportPath = testInfo.outputPath("pretext-markdown-corpus-parity.json");
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), "utf8");
  await testInfo.attach("pretext-markdown-corpus-parity.json", {
    path: reportPath,
    contentType: "application/json",
  });

  if (!ENFORCE) return;

  const failures = summary.filter((entry) => Math.abs(entry.delta) > MARKDOWN_THRESHOLD_PX);
  expect(
    failures,
    formatFailures("markdown", failures.map((failure) => ({
      name: failure.name,
      width: failure.width,
      delta: failure.delta,
      planned: failure.planned,
      actual: failure.actual,
    }))),
  ).toEqual([]);
});

test("workbench: pretext threshold seam parity sweep", async ({ page }) => {
  test.setTimeout(180000);
  test.slow();
  await openWorkbenchShell(page);

  const markdownSamples = MARKDOWN_CORPUS.filter((sample) => THRESHOLD_MARKDOWN_SAMPLE_NAMES.has(sample.name));
  const userSamples = USER_MESSAGE_CORPUS.filter((sample) => THRESHOLD_USER_SAMPLE_NAMES.has(sample.name));
  const assistantSamples = ASSISTANT_CORPUS.filter((sample) => THRESHOLD_ASSISTANT_SAMPLE_NAMES.has(sample.name));

  const markdownSummary: MarkdownSummaryEntry[] = [];
  const rowSummary: RowSummaryEntry[] = [];

  for (const width of THRESHOLD_WIDTHS) {
    const markdownMeasurements = await measureMarkdownParity(page, markdownSamples, width);
    markdownSummary.push(...markdownMeasurements.map((measurement) => ({ width, ...measurement })));

    for (const sample of userSamples) {
      const measurement = await measureMessageParity(page, {
        ...sample.params,
        viewportWidth: width,
      });
      rowSummary.push({ kind: "message", width, name: sample.name, ...measurement });
    }

    for (const sample of assistantSamples) {
      const measurement = await measureAssistantParity(page, {
        ...sample.params,
        viewportWidth: width,
      });
      rowSummary.push({ kind: "assistant", width, name: sample.name, ...measurement });
    }
  }

  if (!ENFORCE) return;

  const markdownFailures = markdownSummary.filter((entry) => Math.abs(entry.delta) > MARKDOWN_THRESHOLD_PX);
  expect(
    markdownFailures,
    formatFailures(
      "threshold markdown",
      markdownFailures.map((failure) => ({
        name: failure.name,
        width: failure.width,
        delta: failure.delta,
        planned: failure.planned,
        actual: failure.actual,
      })),
    ),
  ).toEqual([]);

  const rowFailures = rowSummary.filter((entry) => Math.abs(entry.delta) > ROW_THRESHOLD_PX);
  expect(
    rowFailures,
    formatFailures(
      "threshold rows",
      rowFailures.map((failure) => ({
        name: `${failure.kind}:${failure.name}`,
        width: failure.width,
        delta: failure.delta,
        planned: failure.planned,
        actual: failure.actual,
      })),
    ),
  ).toEqual([]);
});

test("workbench: pretext soft-break markdown regressions", async ({ page, browserName }) => {
  test.setTimeout(180000);
  test.slow();
  await openWorkbenchShell(page);

  const summary: MarkdownSummaryEntry[] = [];
  for (const sample of SOFT_BREAK_MARKDOWN_REGRESSIONS.filter((candidate) => {
    return candidate.browsers == null || candidate.browsers.includes(browserName as "chromium" | "webkit");
  })) {
    const [measurement] = await measureMarkdownParity(page, [sample], sample.width);
    summary.push({ width: sample.width, ...measurement });
  }

  if (!ENFORCE) return;

  const failures = summary.filter((entry) => Math.abs(entry.delta) > MARKDOWN_THRESHOLD_PX);
  expect(
    failures,
    formatFailures(
      "soft-break markdown",
      failures.map((failure) => ({
        name: failure.name,
        width: failure.width,
        delta: failure.delta,
        planned: failure.planned,
        actual: failure.actual,
      })),
    ),
  ).toEqual([]);
});

test("workbench: pretext markdown quote-link-code seam parity", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const [measurement] = await measureMarkdownParity(
    page,
    [
      {
        name: "blockquote-link-code-tail",
        markdown:
          "> [summary message summary](https://example.com/inline-code/parity/webkit/parity?ref=781) `cargo test -p ctx-store`:",
      },
    ],
    382.48,
  );

  expect(
    Math.abs(measurement?.delta ?? Number.POSITIVE_INFINITY),
    `markdown drifted by ${measurement?.delta ?? "unknown"}px (planned ${measurement?.planned ?? "?"}, actual ${measurement?.actual ?? "?"})`,
  ).toBeLessThanOrEqual(MARKDOWN_THRESHOLD_PX);
});

test("workbench: pretext sealed inline path threshold parity", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurements = await measureMarkdownParity(
    page,
    [
      {
        name: "sealed-inline-path-threshold",
        markdown: "`apps/e2e/e2e/core/web/pretextVirtualizerRowLayout.ts`",
      },
    ],
    150,
  );

  if (!ENFORCE) return;

  const failures = measurements.filter((entry) => Math.abs(entry.delta) > MARKDOWN_THRESHOLD_PX);
  expect(
    failures,
    formatFailures(
      "sealed inline path threshold markdown",
      failures.map((failure) => ({
        name: failure.name,
        width: 150,
        delta: failure.delta,
        planned: failure.planned,
        actual: failure.actual,
      })),
    ),
  ).toEqual([]);
});

test("workbench: pretext mixed inline path continuation parity", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const cases = [
    {
      width: 440,
      samples: [
        {
          name: "path-after-prose-first-slice",
          markdown:
            "Agent header entry marker *header* stream summary layout entry summary 🙂 測試 佈局 `core/e2e/pretextVirtualizerRowLayout.ts/sessionThreadDomMeasurement.tsx/apps/turn-header` cargo test -p ctx-http.",
        },
      ],
    },
    {
      width: 416,
      samples: [
        {
          name: "path-tail-final-fragment-after-decorated-prose",
          markdown:
            "Probe layout browser `turn-header/sessionMarkdownMeasurement.ts/pages/inline-code/turn-header` context command virtualizer fragment 🧪 測試 佈局 ~~virtualizer~~. `blockquote/fixtures/sessionThread/e2e`",
        },
      ],
    },
    {
      width: 382.48,
      samples: [
        {
          name: "punctuation-seam-command-continuation",
          markdown:
            "Buffer session summary ~~turn~~ ⚙️ 你好 世界 `core/blockquote/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx` ~~command~~ ~~command command~~ **context**: `workbenchShell/fixtures/turn-header/table` Stream browser agent ⚙️ 測試 佈局 header deterministic thread summary buffer entry context ⚙️ 你好 世界 `web/fixtures/core/fixtures`. `pnpm -C core/apps/web test:e2e:pretext:corpus:webkit`",
        },
        {
          name: "path-after-short-prose",
          markdown:
            "Entry command command `turn-header/workbenchShell/blockquote/workbenchShell/src/pages/core`",
        },
        {
          name: "path-tail-continuation",
          markdown: "composer `sessionThread/src/apps/web/inline-code/pretextVirtualizerRowLayout.ts`",
        },
      ],
    },
    {
      width: 121.3333333333,
      samples: [
        {
          name: "narrow-continuation-full-chrome",
          markdown: "`table/src/blockquote/workbenchShell`",
        },
      ],
    },
    {
      width: 181.3333333333,
      samples: [
        {
          name: "table-cell-terminal-path-tail-wrap",
          markdown: "`table/e2e/workbenchShell/apps/web/sessionThread`",
        },
        {
          name: "webkit-sealed-path-cell-width",
          markdown: "`apps/e2e/e2e/core/web/pretextVirtualizerRowLayout.ts`",
        },
      ],
    },
  ] as const;

  const failures: Array<{
    actual: number;
    delta: number;
    name: string;
    planned: number;
    width: number;
  }> = [];

  for (const entry of cases) {
    const measurements = await measureMarkdownParity(page, entry.samples, entry.width);
    failures.push(
      ...measurements
        .filter((sample) => Math.abs(sample.delta) > MARKDOWN_THRESHOLD_PX)
        .map((sample) => ({
          name: sample.name,
          width: entry.width,
          delta: sample.delta,
          planned: sample.planned,
          actual: sample.actual,
        })),
    );
  }

  if (!ENFORCE) return;

  expect(
    failures,
    formatFailures("mixed inline path continuation markdown", failures),
  ).toEqual([]);
});

test("workbench: pretext transcript row corpus parity sweep", async ({ page }, testInfo) => {
  test.setTimeout(180000);
  test.slow();
  await openWorkbenchShell(page);

  const summary: RowSummaryEntry[] = [];

  for (const width of WIDTHS) {
    for (const sample of USER_MESSAGE_CORPUS) {
      const measurement = await measureMessageParity(page, {
        ...sample.params,
        viewportWidth: width,
      });
      summary.push({ kind: "message", width, name: sample.name, ...measurement });
    }

    for (const sample of ASSISTANT_CORPUS) {
      const measurement = await measureAssistantParity(page, {
        ...sample.params,
        viewportWidth: width,
      });
      summary.push({ kind: "assistant", width, name: sample.name, ...measurement });
    }

    for (const sample of TURN_HEADER_CORPUS) {
      const measurement = await measureTurnHeaderParity(page, {
        ...sample.params,
        viewportWidth: width,
      });
      summary.push({ kind: "turn_header", width, name: sample.name, ...measurement });
    }
  }

  const report = {
    enforce: ENFORCE,
    thresholdPx: ROW_THRESHOLD_PX,
    widths: WIDTHS,
    samples: summary,
  };
  const reportPath = testInfo.outputPath("pretext-transcript-row-corpus-parity.json");
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), "utf8");
  await testInfo.attach("pretext-transcript-row-corpus-parity.json", {
    path: reportPath,
    contentType: "application/json",
  });

  if (!ENFORCE) return;

  const failures = summary.filter((entry) => Math.abs(entry.delta) > ROW_THRESHOLD_PX);
  expect(
    failures,
    formatFailures(
      "transcript rows",
      failures.map((failure) => ({
        name: `${failure.kind}:${failure.name}`,
        width: failure.width,
        delta: failure.delta,
        planned: failure.planned,
        actual: failure.actual,
      })),
    ),
  ).toEqual([]);
});
