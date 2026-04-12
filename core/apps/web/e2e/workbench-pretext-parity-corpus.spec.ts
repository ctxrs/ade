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
const MARKDOWN_THRESHOLD_PX = 1;
const ROW_THRESHOLD_PX = 1;

const IMAGE_DATA_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO2VzJ8AAAAASUVORK5CYII=";

const MARKDOWN_CORPUS: MarkdownSample[] = [
  {
    name: "adjacent-inline-code",
    markdown:
      "Paragraph with `alpha-beta-gamma-delta/ctx/path/one` `second-inline-token/with/path/two` beside prose and punctuation.",
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

const TURN_HEADER_CORPUS = [
  {
    name: "url-heavy",
    params: {
      plainText: [
        "- https://example.com/a/really/long/path/that/keeps/wrapping?token=12345 should not drift when wrapped inside the turn header bubble.",
        "- Follow-up line with /Users/example-user/.ctx/worktrees/00000000-0000-4000-8000-000000000001 path pressure.",
      ].join("\n"),
    },
  },
  {
    name: "multiline-wrap",
    params: {
      plainText: [
        "Need follow-up on the long transcript planner path and the completion row interaction.",
        "Also verify that narrow widths keep the header height in sync after wrap pressure.",
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
