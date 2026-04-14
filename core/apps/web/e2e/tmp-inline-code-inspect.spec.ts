import { test } from "./fixtures";
import { openWorkbenchShell } from "./utils/pretextParity";
import { generatePretextParityFuzzCorpus } from "./utils/pretextParityFuzz";
import {
  resolveSessionThreadAssistantTextWidth,
  resolveSessionThreadMessageTextWidth,
  resolveSessionThreadTurnHeaderTextWidth,
} from "../src/pages/sessionThread/sessionThreadLayoutTokens";

const TARGETS = [
  {
    name: "generated-md-4-nested-list-nested-list-table-nested-list",
    widths: [472, 540, 620, 788],
  },
  {
    name: "generated-md-8-blockquote-list-heading-table",
    widths: [540, 620],
  },
  {
    name: "generated-md-10-blockquote-table-hard-break-paragraph",
    widths: [788],
  },
  {
    name: "generated-md-15-list-blockquote",
    widths: [540, 620, 788],
  },
  {
    name: "generated-md-14-table-heading-blockquote",
    widths: [472, 788],
  },
  {
    name: "generated-md-18-blockquote-hard-break",
    widths: [472, 540, 620],
  },
  {
    name: "generated-md-11-fence-list-list",
    widths: [472, 620, 788],
  },
] as const;

const MD18_TOKEN =
  "pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures";
const MD11_TOKEN =
  "pretextVirtualizerRowLayout.ts/pages/turn-header/sessionMarkdownMeasurement.ts/core/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx";
const MD14_PATH = "apps/e2e/e2e/core/web/pretextVirtualizerRowLayout.ts";
const MD14_COMMAND = "pnpm -C core/apps/web test:e2e:pretext:parity:webkit";

test("tmp inspect fuzz markdown prefixes", async ({ page }) => {
  await openWorkbenchShell(page);

  const corpus = generatePretextParityFuzzCorpus();
  for (const target of TARGETS) {
    const sample = corpus.markdownSamples.find((entry) => entry.name === target.name);
    if (!sample) {
      throw new Error(`missing sample: ${target.name}`);
    }
    const blocks = sample.markdown.split("\n\n");
    const variants = blocks.flatMap((block, index) => {
      const prefix = blocks.slice(0, index + 1).join("\n\n");
      return [
        { name: `${sample.name}:block:${index}`, markdown: block },
        { name: `${sample.name}:prefix:${index}`, markdown: prefix },
      ];
    });
    const measureSamples = [{ name: sample.name, markdown: sample.markdown }, ...variants];
    const markdownByName = new Map(measureSamples.map((entry) => [entry.name, entry.markdown]));
    for (const width of target.widths) {
      const results = await page.evaluate(
        async ({ probeSamples, probeWidth }) => {
          const e2e = (window as Window & {
            __ctxE2E?: {
              measureMarkdownParity?: (
                samples: ReadonlyArray<{ name: string; markdown: string }>,
                width: number,
              ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            };
          }).__ctxE2E;
          if (typeof e2e?.measureMarkdownParity !== "function") {
            throw new Error("markdown parity bridge unavailable");
          }
          return e2e.measureMarkdownParity(probeSamples, probeWidth);
        },
        { probeSamples: measureSamples, probeWidth: width },
      );
      console.log(
        JSON.stringify(
          {
            target: target.name,
            width,
            results: results
              .filter((entry) => Math.abs(entry.delta) > 0)
              .map((entry) => ({
                ...entry,
                markdown: markdownByName.get(entry.name) ?? null,
              })),
          },
          null,
          2,
        ),
      );
    }
  }
});

test("tmp inspect surviving inline-code contexts", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      width: 472,
      samples: [
        { name: "blockquote-path-bare", markdown: `> \`${MD18_TOKEN}\`` },
        {
          name: "blockquote-path-prose",
          markdown:
            `> Parity probe parity probe delta agent browser virtualizer command \`${MD18_TOKEN}\` marker buffer stream header message command.`,
        },
        { name: "list-path-bare", markdown: `- \`${MD11_TOKEN}\`` },
        {
          name: "list-path-prose",
          markdown:
            `- Padding message header turn render agent thread buffer padding \`${MD11_TOKEN}\`.`,
        },
        {
          name: "table-path-row",
          markdown: `| Kind | Token |\n|---|---|\n| agent | \`${MD14_PATH}\` |`,
        },
        {
          name: "table-command-row",
          markdown: `| Kind | Token |\n|---|---|\n| fragment session | \`${MD14_COMMAND}\` |`,
        },
      ],
    },
    {
      width: 540,
      samples: [
        { name: "blockquote-path-bare", markdown: `> \`${MD18_TOKEN}\`` },
        {
          name: "blockquote-path-prose",
          markdown:
            `> Parity probe parity probe delta agent browser virtualizer command \`${MD18_TOKEN}\` marker buffer stream header message command.`,
        },
      ],
    },
    {
      width: 620,
      samples: [
        { name: "list-path-bare", markdown: `- \`${MD11_TOKEN}\`` },
        {
          name: "list-path-prose",
          markdown:
            `- Padding message header turn render agent thread buffer padding \`${MD11_TOKEN}\`.`,
        },
      ],
    },
    {
      width: 788,
      samples: [
        {
          name: "table-path-row",
          markdown: `| Kind | Token |\n|---|---|\n| agent | \`${MD14_PATH}\` |`,
        },
        {
          name: "table-command-row",
          markdown: `| Kind | Token |\n|---|---|\n| fragment session | \`${MD14_COMMAND}\` |`,
        },
      ],
    },
  ] as const;

  for (const probe of probes) {
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: probe.samples, probeWidth: probe.width },
    );
    console.log(JSON.stringify({ kind: "inline-context", width: probe.width, results }, null, 2));
  }
});

test("tmp inspect planner vs DOM line packing for surviving path tokens", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      name: "list-path-bare",
      width: 620,
      markdown: `- \`${MD11_TOKEN}\``,
      token: MD11_TOKEN,
    },
    {
      name: "table-path-row",
      width: 472,
      markdown: `| Kind | Token |\n|---|---|\n| agent | \`${MD14_PATH}\` |`,
      token: MD14_PATH,
    },
  ] as const;

  for (const probe of probes) {
    const details = await page.evaluate(
      async ({ name, markdown, width, token }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = token;
        win.__ctxInlineCodeDebugWidth = width;
        await win.__ctxE2E?.measureMarkdownParity?.([{ name, markdown }], width);
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(markdown, width);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const fragments = Array.from(
          document.querySelectorAll("#markdown-scroll-probe .code-token-fragment"),
        ).map((node) => {
          const element = node as HTMLElement;
          const rect = element.getBoundingClientRect();
          return {
            text: element.textContent ?? "",
            top: rect.top,
            left: rect.left,
            width: rect.width,
          };
        });
        const codeRects = Array.from(document.querySelectorAll("#markdown-scroll-probe code")).map((node) => {
          const rects = Array.from((node as HTMLElement).getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          }));
          return {
            text: node.textContent ?? "",
            rects,
          };
        });
        const listBodyWidth =
          document.querySelector<HTMLElement>("#markdown-scroll-probe .wb-md-list-item-body")?.getBoundingClientRect().width ?? null;
        const tableCellWidths = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .wb-md-table-cell"),
        ).map((element) => ({
          text: element.textContent ?? "",
          width: element.getBoundingClientRect().width,
        }));

        win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return { name, width, planner, fragments, codeRects, listBodyWidth, tableCellWidths };
      },
      probe,
    );

    console.log(JSON.stringify({ kind: "line-pack", ...details }, null, 2));
  }
});

test("tmp inspect surviving mixed inline-code paragraphs", async ({ page }) => {
  await openWorkbenchShell(page);

  const md5Item1 =
    "- Layout parity context `table/fixtures/turn-header/table/inline-code` `sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx/apps/sessionThreadDomMeasurement.tsx/workbenchShell` `cargo test -p ctx-store` *thread layout shell*.";
  const md5Item2 =
    "- Context stream layout 🧪 你好 世界 command stream stream probe stream `ctx task list` 📏 測試 佈局;";
  const md11Item1 =
    "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding `pretextVirtualizerRowLayout.ts/pages/turn-header/sessionMarkdownMeasurement.ts/core/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx`.";
  const md18Quote1 =
    `> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command \`${MD18_TOKEN}\` *marker buffer stream* **header message command** 🧪 你好 世界;`;
  const md11BeforeSecondCode =
    "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding.";
  const md18BeforeCode =
    "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command.";
  const md5SecondCode =
    "sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx/apps/sessionThreadDomMeasurement.tsx/workbenchShell";
  const md11FirstCode =
    "sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header";
  const md11SecondCode =
    "pretextVirtualizerRowLayout.ts/pages/turn-header/sessionMarkdownMeasurement.ts/core/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx";

  const probes = [
    {
      width: 472,
      samples: [
        { name: "md5-item1", markdown: md5Item1 },
        { name: "md5-item1-first-code", markdown: "- Layout parity context `table/fixtures/turn-header/table/inline-code`." },
        { name: "md5-item1-second-code-bare", markdown: `- \`${md5SecondCode}\`` },
        {
          name: "md5-item1-two-codes",
          markdown:
            "- Layout parity context `table/fixtures/turn-header/table/inline-code` `sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx/apps/sessionThreadDomMeasurement.tsx/workbenchShell`.",
        },
        { name: "md5-item2", markdown: md5Item2 },
        { name: "md11-item1-first-code-bare", markdown: `- \`${md11FirstCode}\`` },
        { name: "md11-item1-second-code-bare", markdown: `- \`${md11SecondCode}\`` },
        { name: "md11-before-second-code", markdown: md11BeforeSecondCode },
        { name: "md11-item1", markdown: md11Item1 },
        { name: "md18-quote1", markdown: md18Quote1 },
        { name: "md18-before-code", markdown: md18BeforeCode },
        {
          name: "md18-quote1-no-tail",
          markdown: `> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command \`${MD18_TOKEN}\`.`,
        },
      ],
    },
    {
      width: 620,
      samples: [
        { name: "md11-before-second-code", markdown: md11BeforeSecondCode },
        { name: "md11-item1", markdown: md11Item1 },
      ],
    },
  ] as const;

  for (const probe of probes) {
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: probe.samples, probeWidth: probe.width },
    );
    console.log(JSON.stringify({ kind: "mixed-inline", width: probe.width, results }, null, 2));
  }
});

test("tmp inspect current row frontier by block and line prefixes", async ({ page }) => {
  await openWorkbenchShell(page);

  const corpus = generatePretextParityFuzzCorpus();
  const markdownCases = [
    {
      name: "generated-message-2-generated-md-2-hard-break-paragraph-hard-break-nested-list",
      width: 472,
      innerWidth: resolveSessionThreadMessageTextWidth(472),
      markdown:
        corpus.messageSamples.find(
          (sample) => sample.name === "generated-message-2-generated-md-2-hard-break-paragraph-hard-break-nested-list",
        )?.params.content ?? "",
    },
    {
      name: "generated-message-3-generated-md-3-fence-hard-break-nested-list-fence",
      width: 472,
      innerWidth: resolveSessionThreadMessageTextWidth(472),
      markdown:
        corpus.messageSamples.find(
          (sample) => sample.name === "generated-message-3-generated-md-3-fence-hard-break-nested-list-fence",
        )?.params.content ?? "",
    },
    {
      name: "generated-message-4-generated-md-4-table-paragraph-blockquote-heading",
      width: 472,
      innerWidth: resolveSessionThreadMessageTextWidth(472),
      markdown:
        corpus.messageSamples.find(
          (sample) => sample.name === "generated-message-4-generated-md-4-table-paragraph-blockquote-heading",
        )?.params.content ?? "",
    },
    {
      name: "generated-message-8-generated-md-8-table-nested-list-fence-paragraph",
      width: 472,
      innerWidth: resolveSessionThreadMessageTextWidth(472),
      markdown:
        corpus.messageSamples.find(
          (sample) => sample.name === "generated-message-8-generated-md-8-table-nested-list-fence-paragraph",
        )?.params.content ?? "",
    },
    {
      name: "generated-message-8-generated-md-8-table-nested-list-fence-paragraph",
      width: 788,
      innerWidth: resolveSessionThreadMessageTextWidth(788),
      markdown:
        corpus.messageSamples.find(
          (sample) => sample.name === "generated-message-8-generated-md-8-table-nested-list-fence-paragraph",
        )?.params.content ?? "",
    },
    {
      name: "generated-assistant-1-generated-md-101-table-table-hard-break",
      width: 788,
      innerWidth: resolveSessionThreadAssistantTextWidth(788),
      markdown:
        corpus.assistantSamples.find(
          (sample) => sample.name === "generated-assistant-1-generated-md-101-table-table-hard-break",
        )?.params.content ?? "",
    },
    {
      name: "generated-assistant-2-generated-md-102-heading-table",
      width: 472,
      innerWidth: resolveSessionThreadAssistantTextWidth(472),
      markdown:
        corpus.assistantSamples.find(
          (sample) => sample.name === "generated-assistant-2-generated-md-102-heading-table",
        )?.params.content ?? "",
    },
    {
      name: "generated-assistant-3-generated-md-103-heading-list",
      width: 788,
      innerWidth: resolveSessionThreadAssistantTextWidth(788),
      markdown:
        corpus.assistantSamples.find(
          (sample) => sample.name === "generated-assistant-3-generated-md-103-heading-list",
        )?.params.content ?? "",
    },
    {
      name: "generated-assistant-5-generated-md-105-paragraph-list-fence-fence",
      width: 472,
      innerWidth: resolveSessionThreadAssistantTextWidth(472),
      markdown:
        corpus.assistantSamples.find(
          (sample) => sample.name === "generated-assistant-5-generated-md-105-paragraph-list-fence-fence",
        )?.params.content ?? "",
    },
    {
      name: "generated-assistant-11-generated-md-111-table-table-fence",
      width: 472,
      innerWidth: resolveSessionThreadAssistantTextWidth(472),
      markdown:
        corpus.assistantSamples.find(
          (sample) => sample.name === "generated-assistant-11-generated-md-111-table-table-fence",
        )?.params.content ?? "",
    },
  ] as const;

  for (const probe of markdownCases) {
    const blocks = probe.markdown.split("\n\n");
    const samples = [
      { name: `${probe.name}:full`, markdown: probe.markdown },
      ...blocks.map((block, index) => ({
        name: `${probe.name}:block:${index + 1}`,
        markdown: block,
      })),
      ...blocks.map((_, index) => ({
        name: `${probe.name}:prefix:${index + 1}`,
        markdown: blocks.slice(0, index + 1).join("\n\n"),
      })),
    ];
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const api = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof api?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return api.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: samples, probeWidth: probe.innerWidth },
    );
    console.log(
      JSON.stringify(
        {
          kind: "markdown-frontier",
          name: probe.name,
          width: probe.width,
          innerWidth: probe.innerWidth,
          blocks: blocks.map((block, index) => ({
            index: index + 1,
            markdown: block,
          })),
          failures: results.filter((entry) => Math.abs(entry.delta) > 0),
        },
        null,
        2,
      ),
    );
  }

  const collapsedCases = [
    {
      name: "generated-message-5-collapsed",
      width: 620,
      innerWidth: resolveSessionThreadMessageTextWidth(620),
      preview:
        (corpus.messageSamples.find((sample) => sample.name === "generated-message-5-collapsed")?.params.content ?? "")
          .split("\n")
          .slice(0, 20)
          .join("\n"),
    },
    {
      name: "generated-message-10-collapsed",
      width: 472,
      innerWidth: resolveSessionThreadMessageTextWidth(472),
      preview:
        (corpus.messageSamples.find((sample) => sample.name === "generated-message-10-collapsed")?.params.content ?? "")
          .split("\n")
          .slice(0, 20)
          .join("\n"),
    },
    {
      name: "generated-message-10-collapsed",
      width: 540,
      innerWidth: resolveSessionThreadMessageTextWidth(540),
      preview:
        (corpus.messageSamples.find((sample) => sample.name === "generated-message-10-collapsed")?.params.content ?? "")
          .split("\n")
          .slice(0, 20)
          .join("\n"),
    },
    {
      name: "generated-message-10-collapsed",
      width: 620,
      innerWidth: resolveSessionThreadMessageTextWidth(620),
      preview:
        (corpus.messageSamples.find((sample) => sample.name === "generated-message-10-collapsed")?.params.content ?? "")
          .split("\n")
          .slice(0, 20)
          .join("\n"),
    },
  ] as const;

  for (const probe of collapsedCases) {
    const lines = probe.preview.split("\n");
    const samples = [
      { name: `${probe.name}:full`, markdown: probe.preview },
      ...lines.map((_, index) => ({
        name: `${probe.name}:prefix:${index + 1}`,
        markdown: lines.slice(0, index + 1).join("\n"),
      })),
    ];
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const api = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof api?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return api.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: samples, probeWidth: probe.innerWidth },
    );
    console.log(
      JSON.stringify(
        {
          kind: "collapsed-frontier",
          name: probe.name,
          width: probe.width,
          innerWidth: probe.innerWidth,
          failures: results.filter((entry) => Math.abs(entry.delta) > 0),
        },
        null,
        2,
      ),
    );
  }

  const turnHeaderCases = [
    {
      name: "generated-turn-header-1",
      width: 540,
    },
    {
      name: "generated-turn-header-2",
      width: 472,
    },
    {
      name: "generated-turn-header-3",
      width: 540,
    },
    {
      name: "generated-turn-header-4",
      width: 620,
    },
    {
      name: "generated-turn-header-5",
      width: 620,
    },
    {
      name: "generated-turn-header-6",
      width: 472,
    },
    {
      name: "generated-turn-header-8",
      width: 540,
    },
  ] as const;

  for (const probe of turnHeaderCases) {
    const plainText =
      corpus.turnHeaderSamples.find((sample) => sample.name === probe.name)?.params.plainText ?? "";
    const lines = plainText.split("\n");
    const variants = [
      { plainText, name: `${probe.name}:full` },
      ...lines.map((_, index) => ({
        plainText: lines.slice(0, index + 1).join("\n"),
        name: `${probe.name}:prefix:${index + 1}`,
      })),
    ];
    const results = [];
    for (const variant of variants) {
      const measurement = await page.evaluate(
        async ({ plainText: nextPlainText, width }) => {
          const api = (window as Window & {
            __ctxE2E?: {
              measureTurnHeaderParity?: (params: {
                plainText: string;
                viewportWidth?: number;
              }) => Promise<{ planned: number; actual: number; delta: number }>;
            };
          }).__ctxE2E;
          if (typeof api?.measureTurnHeaderParity !== "function") {
            throw new Error("turn header parity bridge unavailable");
          }
          return api.measureTurnHeaderParity({ plainText: nextPlainText, viewportWidth: width });
        },
        { plainText: variant.plainText, width: probe.width },
      );
      results.push({ name: variant.name, ...measurement });
    }
    console.log(
      JSON.stringify(
        {
          kind: "turn-header-frontier",
          name: probe.name,
          width: probe.width,
          plainText,
          failures: results.filter((entry) => Math.abs(entry.delta) > 0),
        },
        null,
        2,
      ),
    );
  }
});

test("tmp inspect representative frontier line details", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      kind: "markdown",
      name: "assistant2-paragraph-440",
      width: 440,
      debugTarget: "core/e2e/pretextVirtualizerRowLayout.ts/sessionThreadDomMeasurement.tsx/apps/turn-header",
      markdown:
        "Agent header entry marker *header* stream summary layout entry summary 🙂 測試 佈局 `core/e2e/pretextVirtualizerRowLayout.ts/sessionThreadDomMeasurement.tsx/apps/turn-header`",
    },
    {
      kind: "markdown",
      name: "message4-paragraph-382",
      width: 382.48,
      debugTarget: "turn-header/workbenchShell/blockquote/workbenchShell/src/pages/core",
      markdown:
        "Entry command command `turn-header/workbenchShell/blockquote/workbenchShell/src/pages/core`",
    },
    {
      kind: "markdown",
      name: "assistant3-paragraph-756",
      width: 756,
      debugTarget: "inline-code/turn-header/sessionThreadDomMeasurement.tsx/sessionThread/web/e2e/core",
      markdown:
        "Turn fragment composer `sessionMarkdownMeasurement.ts/pretextVirtualizerRowLayout.ts/sessionMarkdownMeasurement.ts/sessionMarkdownMeasurement.ts` `ctx task list` probe command render buffer delta `inline-code/turn-header/sessionThreadDomMeasurement.tsx/sessionThread/web/e2e/core` marker header turn deterministic render virtualizer;",
    },
    {
      kind: "markdown",
      name: "message3-list-382",
      width: 382.48,
      debugTarget:
        "sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx/fixtures/workbenchShell/workbenchShell/core/workbenchShell",
      markdown:
        "- Layout browser fragment turn stream shell stream delta summary command session shell shell **command**.\n  - Stream inline `sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx/fixtures/workbenchShell/workbenchShell/core/workbenchShell` 📏 測試 佈局 shell virtualizer inline browser `sessionThread/src/pretextVirtualizerRowLayout.ts/fixtures/core` *padding probe session*;\n  - Entry composer virtualizer delta ~~browser probe~~ ~~composer~~ token inline token context context.",
    },
    {
      kind: "markdown",
      name: "assistant11-table-440",
      width: 440,
      debugTarget:
        "pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/blockquote/turn-header/workbenchShell/fixtures",
      markdown: [
        "| Kind | Token | Note |",
        "|---|---|---|",
        "| pretext stream | `pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/blockquote/turn-header/workbenchShell/fixtures` | thread shell entry token delta |",
        "| entry probe | `pretextVirtualizerRowLayout.ts/pages/inline-code/table` | buffer marker inline turn |",
        "| browser | `turn-header/blockquote/blockquote/src/web/e2e/workbenchShell` | composer stream deterministic fragment token marker delta |",
      ].join("\n"),
    },
    {
      kind: "markdown",
      name: "message2-list-382",
      width: 382.48,
      debugTarget: "sessionThreadDomMeasurement.tsx/core/pretextVirtualizerRowLayout.ts/pages/pages/apps",
      markdown: [
        "- Agent thread token ~~message~~ `sessionThreadDomMeasurement.tsx/core/pretextVirtualizerRowLayout.ts/pages/pages/apps` **buffer delta** **composer** browser fragment composer composer shell:",
        "  - Probe probe context command composer agent parity parity padding stream padding shell *thread* ~~entry buffer~~ ⚙️ 測試 佈局.",
        "  - Entry browser **inline stream pretext** 🙂 你好 世界 *render session render* **delta render layout** 📏 你好 世界.",
      ].join("\n"),
    },
    {
      kind: "markdown",
      name: "message8-list-382",
      width: 382.48,
      debugTarget: "sessionThreadDomMeasurement.tsx/inline-code/turn-header/web/turn-header/src",
      markdown: [
        "- Thread stream parity fragment `sessionThreadDomMeasurement.tsx/inline-code/turn-header/web/turn-header/src` 🧪 測試 佈局 🙂 你好 世界:",
        "  - Buffer buffer ~~composer~~ `core/pages/pages/pages` ~~buffer~~;",
        "  - Deterministic parity message stream `git rev-parse HEAD` [padding entry](https://example.com/inline-code/transcript?ref=676) *message agent* *session entry header* *summary summary*:",
      ].join("\n"),
    },
    {
      kind: "turn-header",
      name: "turn-header-5-620",
      width: 620,
      plainText:
        "Composer turn header pretext session https://example.com/transcript/transcript/docs/streaming-tail?ref=972 fixtures/web/fixtures/web.\nCommand buffer summary ctx serve core/e2e/sessionThread/workbenchShell/inline-code.",
    },
  ] as const;

  for (const probe of probes) {
    if (probe.kind === "markdown") {
      const details = await page.evaluate(
        async ({ probeMarkdown, probeWidth, debugTarget, name }) => {
          const win = window as Window & {
            __ctxForceInlineCodeDebug?: boolean;
            __ctxInlineCodeDebugTarget?: string;
            __ctxInlineCodeDebugWidth?: number;
            __ctxInlineCodeDebug?: unknown;
            __ctxE2E?: {
              measureMarkdownParity?: (
                samples: ReadonlyArray<{ name: string; markdown: string }>,
                width: number,
              ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
              installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
              removeMarkdownScrollProbe?: () => void;
            };
          };

          await win.__ctxE2E?.installMarkdownScrollProbe?.(probeMarkdown, probeWidth);
          await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

          const root = document.querySelector<HTMLElement>("#markdown-scroll-probe .wb-markdown-root");
          if (!(root instanceof HTMLElement)) {
            throw new Error("missing markdown root");
          }
          const matchingCode = Array.from(root.querySelectorAll<HTMLElement>("code")).find(
            (element) => (element.textContent ?? "").includes(debugTarget),
          );
          const debugContainer =
            matchingCode?.closest<HTMLElement>(".wb-md-table-cell, .wb-md-list-item-body, .wb-md-blockquote-body") ??
            root;
          const debugWidth = debugContainer.getBoundingClientRect().width;
          await win.__ctxE2E?.removeMarkdownScrollProbe?.();

          win.__ctxForceInlineCodeDebug = true;
          win.__ctxInlineCodeDebugTarget = debugTarget;
          win.__ctxInlineCodeDebugWidth = debugWidth;
          win.__ctxInlineCodeDebug = null;
          const parity = await win.__ctxE2E?.measureMarkdownParity?.([{ name, markdown: probeMarkdown }], probeWidth);
          const planner = win.__ctxInlineCodeDebug ?? null;

          await win.__ctxE2E?.installMarkdownScrollProbe?.(probeMarkdown, probeWidth);
          await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

          const refreshedRoot = document.querySelector<HTMLElement>("#markdown-scroll-probe .wb-markdown-root");
          if (!(refreshedRoot instanceof HTMLElement)) {
            throw new Error("missing refreshed markdown root");
          }
          const range = document.createRange();
          range.selectNodeContents(refreshedRoot);
          const lineRects = Array.from(range.getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          }));
          const codeRects = Array.from(refreshedRoot.querySelectorAll<HTMLElement>("code")).map((node) => ({
            text: node.textContent ?? "",
            rects: Array.from(node.getClientRects()).map((rect) => ({
              top: rect.top,
              left: rect.left,
              width: rect.width,
              height: rect.height,
            })),
          }));

          await win.__ctxE2E?.removeMarkdownScrollProbe?.();
          return { parity, planner, lineRects, codeRects, debugWidth };
        },
        {
          probeMarkdown: probe.markdown,
          probeWidth: probe.width,
          debugTarget: probe.debugTarget,
          name: probe.name,
        },
      );

      console.log(JSON.stringify({ kind: "representative-frontier", probe, ...details }, null, 2));
      continue;
    }

    const details = await page.evaluate(
      async ({ plainText, width }) => {
        const api = (window as Window & {
          __ctxE2E?: {
            measureTurnHeaderParity?: (params: {
              plainText: string;
              viewportWidth?: number;
            }) => Promise<{ planned: number; actual: number; delta: number }>;
          };
        }).__ctxE2E;
        if (typeof api?.measureTurnHeaderParity !== "function") {
          throw new Error("turn header parity bridge unavailable");
        }
        const result = await api.measureTurnHeaderParity({ plainText, viewportWidth: width });
        const host = document.createElement("div");
        host.style.position = "fixed";
        host.style.left = "-10000px";
        host.style.top = "0";
        host.style.width = `${width}px`;
        host.style.setProperty("--wb-markdown-body-font-family", '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif');
        host.style.setProperty("--wb-markdown-body-font-size", "13px");
        host.style.setProperty("--wb-markdown-body-line-height", "20px");
        host.style.setProperty("--wb-turn-header-bubble-padding-block", "8px");
        host.style.setProperty("--wb-turn-header-bubble-padding-inline", "10px");
        host.style.setProperty("--wb-turn-header-bubble-border-width", "1px");
        host.style.setProperty("--wb-turn-header-copy-gutter", "24px");
        document.body.appendChild(host);

        const root = document.createElement("div");
        root.className = "wb-turn-header wb-turn-header-expanded";
        const bubble = document.createElement("div");
        bubble.className = "wb-turn-header-bubble";
        const copy = document.createElement("button");
        copy.type = "button";
        copy.className = "wb-turn-header-copy";
        copy.textContent = "copy";
        const content = document.createElement("div");
        content.className = "wb-turn-header-content";
        for (const [index, line] of plainText.split("\n").entries()) {
          const span = document.createElement("span");
          span.textContent = line;
          content.appendChild(span);
          if (index < plainText.split("\n").length - 1) {
            content.appendChild(document.createElement("br"));
          }
        }
        bubble.append(copy, content);
        root.appendChild(bubble);
        host.appendChild(root);

        const range = document.createRange();
        range.selectNodeContents(content);
        const lineRects = Array.from(range.getClientRects()).map((rect) => ({
          top: rect.top,
          left: rect.left,
          width: rect.width,
          height: rect.height,
        }));
        const contentRect = content.getBoundingClientRect();
        host.remove();
        return { ...result, lineRects, contentRectWidth: contentRect.width };
      },
      { plainText: probe.plainText, width: probe.width },
    );

    console.log(JSON.stringify({ kind: "representative-turn-header", probe, details }, null, 2));
  }
});

test("tmp inspect surviving multi-code DOM layout", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      name: "md5-item1-two-codes",
      width: 472,
      markdown:
        "- Layout parity context `table/fixtures/turn-header/table/inline-code` `sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx/apps/sessionThreadDomMeasurement.tsx/workbenchShell`.",
    },
    {
      name: "md11-item1",
      width: 472,
      markdown:
        "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding `pretextVirtualizerRowLayout.ts/pages/turn-header/sessionMarkdownMeasurement.ts/core/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx`.",
    },
    {
      name: "md18-quote1-no-tail",
      width: 472,
      markdown:
        `> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command \`${MD18_TOKEN}\`.`,
    },
  ] as const;

  for (const probe of probes) {
    const details = await page.evaluate(
      async ({ markdown, width, name }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        }).__ctxE2E;
        await e2e?.installMarkdownScrollProbe?.(markdown, width);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const paragraphs = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe p, #markdown-scroll-probe li, #markdown-scroll-probe blockquote"),
        ).map((node) => ({
          tag: node.tagName,
          text: node.textContent ?? "",
          rects: Array.from(node.getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          })),
        }));
        const codes = Array.from(document.querySelectorAll<HTMLElement>("#markdown-scroll-probe code")).map((node, index) => ({
          index,
          text: node.textContent ?? "",
          rects: Array.from(node.getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          })),
        }));
        e2e?.removeMarkdownScrollProbe?.();
        return { name, width, paragraphs, codes };
      },
      probe,
    );

    console.log(JSON.stringify({ kind: "multi-code-dom", ...details }, null, 2));
  }
});

test("tmp inspect planner debug lines for targeted inline-code transitions", async ({ page }) => {
  await openWorkbenchShell(page);

  const wrappedCode =
    "inline-thing-that-actually-gets-really-long-so-much-so-that-it-wraps-to-multiple-lines/core/apps/web/src/pages/sessionThread/sessionMarkdownMeasurement.ts";
  const probes = [
    {
      name: "md18-quote1-no-tail",
      width: 472,
      token: "*",
      markdown: `> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command \`${MD18_TOKEN}\`.`,
    },
    {
      name: "md11-before-second-code-472",
      width: 472,
      token: "*",
      markdown:
        "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding.",
    },
    {
      name: "md11-before-second-code-620",
      width: 620,
      token: "*",
      markdown:
        "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding.",
    },
    {
      name: "trailing-prose",
      width: 620,
      token: wrappedCode,
      markdown: `begin agent message with plain text and now \`${wrappedCode}\` after the prose`,
    },
    {
      name: "md8-list-item-540-part0",
      width: 540,
      token: "*",
      markdown:
        "- Deterministic agent **parity session** virtualizer browser message virtualizer inline *pretext virtualizer* [turn thread layout](https://example.com/transcript/transcript?ref=450) `fixtures/workbenchShell/fixtures/core/pretextVirtualizerRowLayout.ts/table/src`:",
    },
    {
      name: "md15-list-item-540-part0",
      width: 540,
      token: "*",
      markdown:
        "- Parity summary delta `pretextVirtualizerRowLayout.ts/workbenchShell/core/workbenchShell/sessionThread/turn-header/pages` 🧪 段落 換行 *parity* **token header** `ctx task list` [composer](https://example.com/measurement/chromium?ref=656);",
    },
    {
      name: "md18-block1-540",
      width: 540,
      token: "*",
      markdown:
        "Probe deterministic summary entry [token fragment](https://example.com/parity/virtualizer?ref=329) command pretext virtualizer buffer layout marker *parity* `sessionMarkdownMeasurement.ts/inline-code/sessionMarkdownMeasurement.ts/core` deterministic command command parity:\nInline shell turn ⚙️ 你好 世界 📏 測試 佈局 🙂 測試 佈局.",
    },
    {
      name: "md10-block3-788",
      width: 788,
      token: "*",
      markdown:
        "Session thread turn delta 📏 你好 世界 thread stream virtualizer composer [token agent](https://example.com/inline-code/docs/chromium?ref=901) context context shell fragment thread `inline-code/e2e/workbenchShell/sessionThread` ~~summary inline~~:",
    },
  ] as const;

  for (const probe of probes) {
    await page.reload();
    await openWorkbenchShell(page);

    const details = await page.evaluate(
      async ({ markdown, width, name, token }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = token;
        win.__ctxInlineCodeDebugWidth = width;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.([{ name, markdown }], width);
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(markdown, width);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const paragraphs = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe p, #markdown-scroll-probe li, #markdown-scroll-probe blockquote"),
        ).map((node) => ({
          tag: node.tagName,
          text: node.textContent ?? "",
          height: node.getBoundingClientRect().height,
        }));
        const fragments = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .code-token-fragment"),
        ).map((node) => {
          const rect = node.getBoundingClientRect();
          return {
            text: node.textContent ?? "",
            top: rect.top,
            left: rect.left,
            width: rect.width,
          };
        });
        const codes = Array.from(document.querySelectorAll<HTMLElement>("#markdown-scroll-probe code")).map((node) => ({
          text: node.textContent ?? "",
          rects: Array.from(node.getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          })),
        }));

        win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return { name, width, parity, planner, paragraphs, fragments, codes };
      },
      probe,
    );

    console.log(JSON.stringify({ kind: "planner-debug-lines", ...details }, null, 2));
  }
});

test("tmp inspect isolated md11 and md18 continuation primitives", async ({ page }) => {
  await openWorkbenchShell(page);

  const md11Code =
    "sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header";
  const md18Prefix = "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command";
  const probes = [
    {
      width: 472,
      samples: [
        { name: "md18-prefix-code", markdown: `${md18Prefix} \`${MD18_TOKEN}\`` },
        { name: "md18-prefix-code-period", markdown: `${md18Prefix} \`${MD18_TOKEN}\`.` },
        { name: "md11-code-plain-tail", markdown: `- Padding message header turn \`${md11Code}\` render agent thread buffer padding.` },
        {
          name: "md11-code-link-tail",
          markdown: `- Padding message header turn \`${md11Code}\` [session](https://example.com/inline-code/webkit/parity?ref=556).`,
        },
        {
          name: "md11-code-two-links-tail",
          markdown:
            `- Padding message header turn \`${md11Code}\` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703).`,
        },
        {
          name: "md11-code-links-prose-tail",
          markdown:
            `- Padding message header turn \`${md11Code}\` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding.`,
        },
      ],
    },
    {
      width: 620,
      samples: [
        {
          name: "md11-code-links-prose-tail",
          markdown:
            `- Padding message header turn \`${md11Code}\` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding.`,
        },
      ],
    },
  ] as const;

  for (const probe of probes) {
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: probe.samples, probeWidth: probe.width },
    );

    console.log(JSON.stringify({ kind: "continuation-primitives", width: probe.width, results }, null, 2));
  }
});

test("tmp inspect path-start thresholds", async ({ page }) => {
  const probes = [
    {
      name: "md18-prefix-code-472",
      width: 472,
      markdown:
        "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures`.",
    },
    {
      name: "md11-before-second-code-620",
      width: 620,
      markdown:
        "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding.",
    },
    {
      name: "md18-prefix-code-540",
      width: 540,
      markdown:
        "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures`.",
    },
    {
      name: "md15-list-block-472",
      width: 472,
      markdown:
        "- Parity summary delta `pretextVirtualizerRowLayout.ts/workbenchShell/core/workbenchShell/sessionThread/turn-header/pages` 🧪 段落 換行 *parity* **token header** `ctx task list` [composer](https://example.com/measurement/chromium?ref=656);\n- Composer command browser header pretext buffer render composer thread shell session 🙂 你好 世界 [stream](https://example.com/parity/webkit/webkit/webkit?ref=125) 🧪 段落 換行.\n- Agent shell deterministic command [agent](https://example.com/chromium/docs?ref=933) probe parity entry summary header session entry turn *fragment stream* `fixtures/turn-header/pages/fixtures` pretext agent session delta.\n- Layout render turn inline ~~stream render~~ 📏 測試 佈局 `cargo test -p codex-crp` ~~summary~~ 📏 你好 世界 `git status`.",
    },
    {
      name: "md9-list-fence-620",
      width: 620,
      markdown:
        "- Pretext browser **browser delta** `web/inline-code/src/apps/e2e/blockquote/inline-code` 📏 你好 世界 summary fragment browser entry [message layout parity](https://example.com/assistant/assistant/transcript?ref=833) ~~marker~~:",
    },
    {
      name: "nested-list-wrap-620",
      width: 620,
      markdown:
        "- outer item with `inline-code-token`\n  - nested item with long prose and `ctx/pretext/pathlike/token`\n  - nested sibling with [docs](https://example.com) and punctuation",
    },
    {
      name: "user-with-images-620",
      width: 620,
      markdown: "two inline screenshots and a path `ctx/pretext/harness/attachment-check`",
    },
  ] as const;

  for (const probe of probes) {
    await page.reload();
    await openWorkbenchShell(page);
    const details = await page.evaluate(
      async ({ markdown, width, name }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        };
        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = "*";
        win.__ctxInlineCodeDebugWidth = undefined;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.([{ name, markdown }], width);
        return { name, width, parity, planner: win.__ctxInlineCodeDebug ?? null };
      },
      probe,
    );
    console.log(JSON.stringify({ kind: "path-start-thresholds", ...details }, null, 2));
  }
});

test("tmp inspect full paragraph line boxes for inline-code repros", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      name: "md11-before-second-code-620",
      width: 620,
      markdown:
        "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding.",
    },
    {
      name: "md18-prefix-code-472",
      width: 472,
      markdown:
        "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures`.",
    },
    {
      name: "md15-list-block-472",
      width: 472,
      markdown:
        "- Layout render turn inline ~~stream render~~ 📏 測試 佈局 `cargo test -p codex-crp` ~~summary~~ 📏 你好 世界 `git status`.",
    },
    {
      name: "md9-list-fence-620",
      width: 620,
      markdown:
        "- Pretext browser **browser delta** `web/inline-code/src/apps/e2e/blockquote/inline-code` 📏 你好 世界 summary fragment browser entry [message layout parity](https://example.com/assistant/assistant/transcript?ref=833) ~~marker~~:",
    },
  ] as const;

  for (const probe of probes) {
    const details = await page.evaluate(
      async ({ markdown, width, name }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        }).__ctxE2E;
        await e2e?.installMarkdownScrollProbe?.(markdown, width);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const paragraph = document.querySelector<HTMLElement>(
          "#markdown-scroll-probe p, #markdown-scroll-probe li, #markdown-scroll-probe blockquote",
        );
        if (!paragraph) {
          throw new Error(`missing paragraph for ${name}`);
        }
        const range = document.createRange();
        range.selectNodeContents(paragraph);
        const rects = Array.from(range.getClientRects()).map((rect) => ({
          top: rect.top,
          left: rect.left,
          width: rect.width,
          height: rect.height,
        }));
        const paragraphRect = paragraph.getBoundingClientRect();
        const codeRects = Array.from(paragraph.querySelectorAll("code")).map((node) => ({
          text: node.textContent ?? "",
          rects: Array.from((node as HTMLElement).getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          })),
        }));

        e2e?.removeMarkdownScrollProbe?.();
        return { name, width, paragraphRect, rects, codeRects };
      },
      probe,
    );

    console.log(JSON.stringify({ kind: "paragraph-line-boxes", ...details }, null, 2));
  }
});

test("tmp dump surviving fuzz markdown samples", async () => {
  const corpus = generatePretextParityFuzzCorpus();
  const names = [
    "generated-md-4-nested-list-nested-list-table-nested-list",
    "generated-md-8-blockquote-list-heading-table",
    "generated-md-9-list-fence",
    "generated-md-11-fence-list-list",
    "generated-md-15-list-blockquote",
    "generated-md-16-table-fence-fence",
    "generated-md-18-blockquote-hard-break",
  ] as const;

  for (const name of names) {
    const sample = corpus.markdownSamples.find((entry) => entry.name === name);
    if (!sample) {
      throw new Error(`missing sample: ${name}`);
    }
    console.log(
      JSON.stringify(
        {
          kind: "fuzz-sample",
          name,
          markdown: sample.markdown,
        },
        null,
        2,
      ),
    );
  }
});

test("tmp inspect surviving fuzz markdown block prefixes", async ({ page }) => {
  await openWorkbenchShell(page);

  const corpus = generatePretextParityFuzzCorpus();
  const targets = [
    { name: "generated-md-4-nested-list-nested-list-table-nested-list", widths: [540, 788] },
    { name: "generated-md-8-blockquote-list-heading-table", widths: [472, 540] },
    { name: "generated-md-9-list-fence", widths: [620] },
    { name: "generated-md-11-fence-list-list", widths: [472, 620, 788] },
    { name: "generated-md-15-list-blockquote", widths: [472] },
    { name: "generated-md-16-table-fence-fence", widths: [472, 620, 788] },
    { name: "generated-md-18-blockquote-hard-break", widths: [472] },
  ] as const;

  for (const target of targets) {
    const sample = corpus.markdownSamples.find((entry) => entry.name === target.name);
    if (!sample) {
      throw new Error(`missing sample: ${target.name}`);
    }
    const blocks = sample.markdown.split("\n\n");
    const variants = blocks.flatMap((block, index) => {
      const prefix = blocks.slice(0, index + 1).join("\n\n");
      return [
        { name: `${sample.name}:block:${index}`, markdown: block },
        { name: `${sample.name}:prefix:${index}`, markdown: prefix },
      ];
    });

    for (const width of target.widths) {
      const results = await page.evaluate(
        async ({ probeSamples, probeWidth }) => {
          const e2e = (window as Window & {
            __ctxE2E?: {
              measureMarkdownParity?: (
                samples: ReadonlyArray<{ name: string; markdown: string }>,
                width: number,
              ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            };
          }).__ctxE2E;
          if (typeof e2e?.measureMarkdownParity !== "function") {
            throw new Error("markdown parity bridge unavailable");
          }
          return e2e.measureMarkdownParity(probeSamples, probeWidth);
        },
        { probeSamples: variants, probeWidth: width },
      );
      console.log(
        JSON.stringify(
          {
            kind: "fuzz-block-prefixes",
            target: target.name,
            width,
            results: results.filter((entry) => Math.abs(entry.delta) > 0),
          },
          null,
          2,
        ),
      );
    }
  }
});

function splitTopLevelListItems(markdown: string): string[] {
  const lines = markdown.split("\n");
  const items: string[] = [];
  let current: string[] = [];

  const flush = () => {
    if (current.length > 0) {
      items.push(current.join("\n"));
      current = [];
    }
  };

  for (const line of lines) {
    if (/^- /.test(line)) {
      flush();
      current.push(line);
      continue;
    }
    if (current.length > 0) {
      current.push(line);
    }
  }
  flush();
  return items;
}

function splitBlockquoteParagraphs(markdown: string): string[] {
  return markdown
    .split("\n\n")
    .map((paragraph) => paragraph.trim())
    .filter(Boolean);
}

test("tmp inspect surviving fuzz list items and blockquote paragraphs", async ({ page }) => {
  await openWorkbenchShell(page);

  const corpus = generatePretextParityFuzzCorpus();
  const targets = [
    {
      name: "generated-md-4-nested-list-nested-list-table-nested-list",
      width: 540,
      blockIndex: 0,
      mode: "list" as const,
    },
    {
      name: "generated-md-8-blockquote-list-heading-table",
      width: 540,
      blockIndex: 1,
      mode: "list" as const,
    },
    {
      name: "generated-md-9-list-fence",
      width: 620,
      blockIndex: 0,
      mode: "list" as const,
    },
    {
      name: "generated-md-11-fence-list-list",
      width: 788,
      blockIndex: 2,
      mode: "list" as const,
    },
    {
      name: "generated-md-15-list-blockquote",
      width: 540,
      blockIndex: 0,
      mode: "list" as const,
    },
    {
      name: "generated-md-15-list-blockquote",
      width: 620,
      blockIndex: 0,
      mode: "list" as const,
    },
    {
      name: "generated-md-15-list-blockquote",
      width: 788,
      blockIndex: 0,
      mode: "list" as const,
    },
    {
      name: "generated-md-18-blockquote-hard-break",
      width: 472,
      blockIndex: 0,
      mode: "blockquote" as const,
    },
    {
      name: "generated-md-18-blockquote-hard-break",
      width: 540,
      blockIndex: 0,
      mode: "blockquote" as const,
    },
  ] as const;

  for (const target of targets) {
    const sample = corpus.markdownSamples.find((entry) => entry.name === target.name);
    if (!sample) {
      throw new Error(`missing sample: ${target.name}`);
    }
    const block = sample.markdown.split("\n\n")[target.blockIndex];
    if (!block) {
      throw new Error(`missing block ${target.blockIndex} for ${target.name}`);
    }

    const parts =
      target.mode === "list" ? splitTopLevelListItems(block) : splitBlockquoteParagraphs(block);
    const variants = parts.flatMap((part, index) => {
      const prefix = parts.slice(0, index + 1).join("\n");
      return [
        { name: `${target.name}:${target.mode}:part:${index}`, markdown: part },
        { name: `${target.name}:${target.mode}:prefix:${index}`, markdown: prefix },
      ];
    });

    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: variants, probeWidth: target.width },
    );

    console.log(
      JSON.stringify(
        {
          kind: "fuzz-list-blockquote-parts",
          target: target.name,
          mode: target.mode,
          width: target.width,
          results: results.filter((entry) => Math.abs(entry.delta) > 0),
        },
        null,
        2,
      ),
    );
  }
});

test("tmp inspect table cell code-only vs note-only parity", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      name: "md16-code-only",
      markdown:
        "`turn-header/fixtures/sessionMarkdownMeasurement.ts/src/blockquote/pretextVirtualizerRowLayout.ts/core`",
    },
    {
      name: "md16-note-only",
      markdown: "render deterministic delta virtualizer thread command marker thread",
    },
    {
      name: "md4-table-code-only",
      markdown: "`src/pages/src/pretextVirtualizerRowLayout.ts/sessionThreadDomMeasurement.tsx`",
    },
    {
      name: "table-command-only",
      markdown: "`pnpm -C core/apps/web test:e2e:pretext:parity:chromium`",
    },
  ] as const;
  const widths = [132, 181, 236, 237] as const;

  for (const width of widths) {
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: probes, probeWidth: width },
    );

    console.log(
      JSON.stringify(
        {
          kind: "table-cell-probes",
          width,
          results: results.filter((entry) => Math.abs(entry.delta) > 0),
        },
        null,
        2,
      ),
    );
  }
});

test("tmp inspect table cell code-only line boxes", async ({ page }) => {
  await openWorkbenchShell(page);

  const markdown =
    "`turn-header/fixtures/sessionMarkdownMeasurement.ts/src/blockquote/pretextVirtualizerRowLayout.ts/core`";
  const widths = [132, 181, 236] as const;

  for (const width of widths) {
    const details = await page.evaluate(
      async ({ probeMarkdown, probeWidth }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = "*";
        win.__ctxInlineCodeDebugWidth = undefined;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.(
          [{ name: "md16-code-only-lines", markdown: probeMarkdown }],
          probeWidth,
        );
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(probeMarkdown, probeWidth);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const codeRects = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .code-token-fragment"),
        ).map((node) => {
          const rect = node.getBoundingClientRect();
          return {
            text: node.textContent ?? "",
            top: rect.top,
            left: rect.left,
            width: rect.width,
          };
        });

        await win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return { width: probeWidth, parity, planner, codeRects };
      },
      { probeMarkdown: markdown, probeWidth: width },
    );

    console.log(JSON.stringify({ kind: "table-cell-code-line-boxes", ...details }, null, 2));
  }
});

test("tmp inspect table-inline-code corpus row details", async ({ page }) => {
  await openWorkbenchShell(page);

  const markdown = [
    "| Env | Command | Note |",
    "|---|---|---|",
    "| dev | `pnpm dev` | wraps with prose and punctuation |",
    "| test | `pnpm -C core/apps/web test:e2e:pretext:parity:chromium` | very long note that should keep wrapping |",
  ].join("\n");
  const widths = [540, 620, 788] as const;

  for (const width of widths) {
    const details = await page.evaluate(
      async ({ probeMarkdown, probeWidth }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = "pnpm -C core/apps/web test:e2e:pretext:parity:chromium";
        win.__ctxInlineCodeDebugWidth = probeWidth;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.(
          [{ name: "table-inline-code-corpus", markdown: probeMarkdown }],
          probeWidth,
        );
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(probeMarkdown, probeWidth);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const rowHeights = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .wb-md-table-row"),
        ).map((row, rowIndex) => {
          const rowRect = row.getBoundingClientRect();
          const cellHeights = Array.from(row.querySelectorAll<HTMLElement>(".wb-md-table-cell")).map((cell) => {
            const rect = cell.getBoundingClientRect();
            return {
              text: cell.textContent ?? "",
              width: rect.width,
              height: rect.height,
            };
          });
          return {
            rowIndex,
            height: rowRect.height,
            cells: cellHeights,
          };
        });
        const codeRects = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .wb-md-table-row code"),
        ).map((node) => {
          const rects = Array.from(node.getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          }));
          return {
            text: node.textContent ?? "",
            rects,
          };
        });

        await win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return { width: probeWidth, parity, planner, rowHeights, codeRects };
      },
      { probeMarkdown: markdown, probeWidth: width },
    );

    console.log(JSON.stringify({ kind: "table-inline-code-corpus", ...details }, null, 2));
  }
});

test("tmp inspect table command-only line boxes", async ({ page }) => {
  await openWorkbenchShell(page);

  const markdown = "`pnpm -C core/apps/web test:e2e:pretext:parity:chromium`";
  const widths = [132, 181, 236, 237] as const;

  for (const width of widths) {
    const details = await page.evaluate(
      async ({ probeMarkdown, probeWidth }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = "*";
        win.__ctxInlineCodeDebugWidth = undefined;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.(
          [{ name: "table-command-only-lines", markdown: probeMarkdown }],
          probeWidth,
        );
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(probeMarkdown, probeWidth);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const codeRects = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .code-token-fragment"),
        ).map((node) => {
          const rect = node.getBoundingClientRect();
          return {
            text: node.textContent ?? "",
            top: rect.top,
            left: rect.left,
            width: rect.width,
          };
        });

        await win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return { width: probeWidth, parity, planner, codeRects };
      },
      { probeMarkdown: markdown, probeWidth: width },
    );

    console.log(JSON.stringify({ kind: "table-command-line-boxes", ...details }, null, 2));
  }
});

test("tmp inspect md4 table row details", async ({ page }) => {
  await openWorkbenchShell(page);

  const rows = [
    "| buffer | `table/e2e/workbenchShell/apps/web/sessionThread` | message shell command stream session |",
    "| composer | `src/pages/src/pretextVirtualizerRowLayout.ts/sessionThreadDomMeasurement.tsx` | virtualizer virtualizer pretext fragment inline parity browser |",
    "| turn virtualizer | `sessionThread/sessionThreadDomMeasurement.tsx/inline-code/pages/pretextVirtualizerRowLayout.ts/web` | shell turn render entry parity token buffer deterministic |",
  ];
  const markdown = ["| Kind | Token | Note |", "|---|---|---|", ...rows].join("\n");
  const widths = [472, 788];

  for (const width of widths) {
    const details = await page.evaluate(
      async ({ probeMarkdown, probeWidth }) => {
      const win = window as Window & {
        __ctxForceInlineCodeDebug?: boolean;
        __ctxInlineCodeDebugTarget?: string;
        __ctxInlineCodeDebugWidth?: number;
        __ctxInlineCodeDebug?: unknown;
        __ctxE2E?: {
          measureMarkdownParity?: (
            samples: ReadonlyArray<{ name: string; markdown: string }>,
            width: number,
          ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
          removeMarkdownScrollProbe?: () => void;
        };
      };

      win.__ctxForceInlineCodeDebug = true;
      win.__ctxInlineCodeDebugTarget = "*";
      win.__ctxInlineCodeDebugWidth = undefined;
      const parity = await win.__ctxE2E?.measureMarkdownParity?.(
        [{ name: "md4-table-block", markdown: probeMarkdown }],
        probeWidth,
      );

      await win.__ctxE2E?.installMarkdownScrollProbe?.(probeMarkdown, probeWidth);
      await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

      const rowHeights = Array.from(
        document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .wb-md-table-row"),
      ).map((row, rowIndex) => {
        const rowRect = row.getBoundingClientRect();
        const cells = Array.from(row.querySelectorAll<HTMLElement>(".wb-md-table-cell")).map((cell) => {
          const rect = cell.getBoundingClientRect();
          const codeRects = Array.from(cell.querySelectorAll<HTMLElement>("code")).map((code) => ({
            text: code.textContent ?? "",
            rects: Array.from(code.getClientRects()).map((clientRect) => ({
              top: clientRect.top,
              left: clientRect.left,
              width: clientRect.width,
              height: clientRect.height,
            })),
          }));
          return {
            text: cell.textContent ?? "",
            width: rect.width,
            height: rect.height,
            codeRects,
          };
        });
        return { rowIndex, height: rowRect.height, cells };
      });

      await win.__ctxE2E?.removeMarkdownScrollProbe?.();
        const planner = win.__ctxInlineCodeDebug ?? null;
        return { width: probeWidth, parity, planner, rowHeights };
      },
      { probeMarkdown: markdown, probeWidth: width },
    );

    console.log(JSON.stringify({ kind: "md4-table-row-details", ...details }, null, 2));
  }

  const variants = rows.map((row, index) => ({
    name: `md4-row-${index + 1}`,
    markdown: ["| Kind | Token | Note |", "|---|---|---|", row].join("\n"),
  }));
  for (const width of widths) {
    const parity = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: variants, probeWidth: width },
    );
    console.log(JSON.stringify({ kind: "md4-table-row-variants", width, parity }, null, 2));
  }

  const rowOneDetails = await page.evaluate(
    async ({ probeMarkdown, probeWidth, token }) => {
      const win = window as Window & {
        __ctxForceInlineCodeDebug?: boolean;
        __ctxInlineCodeDebugTarget?: string;
        __ctxInlineCodeDebugWidth?: number;
        __ctxInlineCodeDebug?: unknown;
        __ctxE2E?: {
          measureMarkdownParity?: (
            samples: ReadonlyArray<{ name: string; markdown: string }>,
            width: number,
          ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
          removeMarkdownScrollProbe?: () => void;
        };
      };

      win.__ctxForceInlineCodeDebug = true;
      win.__ctxInlineCodeDebugTarget = token;
      win.__ctxInlineCodeDebugWidth = probeWidth;
      win.__ctxInlineCodeDebug = null;
      const parity = await win.__ctxE2E?.measureMarkdownParity?.([{ name: "md4-row-1", markdown: probeMarkdown }], probeWidth);
      const planner = win.__ctxInlineCodeDebug ?? null;

      await win.__ctxE2E?.installMarkdownScrollProbe?.(probeMarkdown, probeWidth);
      await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

      const code = document.querySelector<HTMLElement>("#markdown-scroll-probe code");
      const codeRects = Array.from(code?.getClientRects() ?? []).map((rect) => ({
        top: rect.top,
        left: rect.left,
        width: rect.width,
        height: rect.height,
      }));

      await win.__ctxE2E?.removeMarkdownScrollProbe?.();
      return { parity, planner, codeRects };
    },
    {
      probeMarkdown: ["| Kind | Token | Note |", "|---|---|---|", rows[0]!].join("\n"),
      probeWidth: 472,
      token: "*",
    },
  );
  console.log(JSON.stringify({ kind: "md4-row-1-details", width: 472, ...rowOneDetails }, null, 2));
});

test("tmp inspect md4 row-1 cell primitives", async ({ page }) => {
  await openWorkbenchShell(page);

  const samples = [
    { name: "md4-row1-note-cell", markdown: "message shell command stream session" },
    { name: "md4-row1-code-cell", markdown: "`table/e2e/workbenchShell/apps/web/sessionThread`" },
  ];
  const width = 132;

  const parity = await page.evaluate(
    async ({ probeSamples, probeWidth }) => {
      const e2e = (window as Window & {
        __ctxE2E?: {
          measureMarkdownParity?: (
            samples: ReadonlyArray<{ name: string; markdown: string }>,
            width: number,
          ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
        };
      }).__ctxE2E;
      if (typeof e2e?.measureMarkdownParity !== "function") {
        throw new Error("markdown parity bridge unavailable");
      }
      return e2e.measureMarkdownParity(probeSamples, probeWidth);
    },
    { probeSamples: samples, probeWidth: width },
  );

  console.log(JSON.stringify({ kind: "md4-row1-cell-primitives", width, parity }, null, 2));
});

test("tmp inspect md4 row-1 code-only line boxes", async ({ page }) => {
  await openWorkbenchShell(page);

  const probe = {
    name: "md4-row1-code-only",
    width: 132,
    markdown: "`table/e2e/workbenchShell/apps/web/sessionThread`",
  };

  const details = await page.evaluate(
    async ({ markdown, width, name }) => {
      const win = window as Window & {
        __ctxForceInlineCodeDebug?: boolean;
        __ctxInlineCodeDebugTarget?: string;
        __ctxInlineCodeDebugWidth?: number;
        __ctxInlineCodeDebug?: unknown;
        __ctxE2E?: {
          measureMarkdownParity?: (
            samples: ReadonlyArray<{ name: string; markdown: string }>,
            width: number,
          ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
          removeMarkdownScrollProbe?: () => void;
        };
      };

      win.__ctxForceInlineCodeDebug = true;
      win.__ctxInlineCodeDebugTarget = "*";
      win.__ctxInlineCodeDebugWidth = width;
      win.__ctxInlineCodeDebug = null;
      const parity = await win.__ctxE2E?.measureMarkdownParity?.([{ name, markdown }], width);
      const planner = win.__ctxInlineCodeDebug ?? null;

      await win.__ctxE2E?.installMarkdownScrollProbe?.(markdown, width);
      await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

      const code = document.querySelector<HTMLElement>("#markdown-scroll-probe code");
      const codeRects = Array.from(code?.getClientRects() ?? []).map((rect) => ({
        top: rect.top,
        left: rect.left,
        width: rect.width,
        height: rect.height,
      }));

      await win.__ctxE2E?.removeMarkdownScrollProbe?.();
      return { parity, planner, codeRects };
    },
    probe,
  );

  console.log(JSON.stringify({ kind: "md4-row1-code-only-lines", ...probe, ...details }, null, 2));
});

test("tmp inspect remaining mixed inline item line boxes", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      name: "md8-list-item-540-part1-lines",
      width: 540,
      markdown:
        "- Agent token marker `src/src/pretextVirtualizerRowLayout.ts/core/blockquote` *padding entry* **marker stream**:",
    },
    {
      name: "md15-list-item-540-part0-lines",
      width: 540,
      markdown:
        "- Parity summary delta `pretextVirtualizerRowLayout.ts/workbenchShell/core/workbenchShell/sessionThread/turn-header/pages` 🧪 段落 換行 *parity* **token header** `ctx task list` [composer](https://example.com/measurement/chromium?ref=656);",
    },
    {
      name: "md11-list-item-788-part0-lines",
      width: 788,
      markdown:
        "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding `pretextVirtualizerRowLayout.ts/pages/turn-header/sessionMarkdownMeasurement.ts/core/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx`.",
    },
    {
      name: "md18-block1-540-lines",
      width: 540,
      markdown:
        "Probe deterministic summary entry [token fragment](https://example.com/parity/virtualizer?ref=329) command pretext virtualizer buffer layout marker *parity* `sessionMarkdownMeasurement.ts/inline-code/sessionMarkdownMeasurement.ts/core` deterministic command command parity:\nInline shell turn ⚙️ 你好 世界 📏 測試 佈局 🙂 測試 佈局.",
    },
    {
      name: "md18-block1-620-lines",
      width: 620,
      markdown:
        "Probe deterministic summary entry [token fragment](https://example.com/parity/virtualizer?ref=329) command pretext virtualizer buffer layout marker *parity* `sessionMarkdownMeasurement.ts/inline-code/sessionMarkdownMeasurement.ts/core` deterministic command command parity:\nInline shell turn ⚙️ 你好 世界 📏 測試 佈局 🙂 測試 佈局.",
    },
  ] as const;

  for (const probe of probes) {
    const details = await page.evaluate(
      async ({ markdown, width, name }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = "*";
        win.__ctxInlineCodeDebugWidth = undefined;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.([{ name, markdown }], width);
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(markdown, width);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const paragraph = document.querySelector<HTMLElement>("#markdown-scroll-probe p");
        if (!(paragraph instanceof HTMLElement)) {
          throw new Error("missing paragraph");
        }

        const paragraphRect = paragraph.getBoundingClientRect();
        const lineRects = Array.from(paragraph.getClientRects()).map((rect) => ({
          top: rect.top,
          left: rect.left,
          width: rect.width,
          height: rect.height,
        }));
        const codeRects = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .code-token-fragment"),
        ).map((node) => {
          const rect = node.getBoundingClientRect();
          return {
            text: node.textContent ?? "",
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          };
        });

        await win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return { name, width, parity, planner, paragraphRect, lineRects, codeRects };
      },
      probe,
    );

    console.log(JSON.stringify({ kind: "remaining-mixed-item-line-boxes", ...details }, null, 2));
  }
});

test("tmp inspect surviving isolated inline-code item packing", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      name: "md8-list-item-under",
      width: 472,
      markdown:
        "- Agent token marker `src/src/pretextVirtualizerRowLayout.ts/core/blockquote` *padding entry* **marker stream**:",
    },
    {
      name: "md8-list-item-540-part0",
      width: 540,
      markdown:
        "- Deterministic agent **parity session** virtualizer browser message virtualizer inline *pretext virtualizer* [turn thread layout](https://example.com/transcript/transcript?ref=450) `fixtures/workbenchShell/fixtures/core/pretextVirtualizerRowLayout.ts/table/src`:",
    },
    {
      name: "md8-list-item-540-part1",
      width: 540,
      markdown:
        "- Agent token marker `src/src/pretextVirtualizerRowLayout.ts/core/blockquote` *padding entry* **marker stream**:",
    },
    {
      name: "md9-list-item-over",
      width: 620,
      markdown:
        "- Pretext browser **browser delta** `web/inline-code/src/apps/e2e/blockquote/inline-code` 📏 你好 世界 summary fragment browser entry [message layout parity](https://example.com/assistant/assistant/transcript?ref=833) ~~marker~~:",
    },
    {
      name: "md9-list-item-620-part2",
      width: 620,
      markdown:
        "- Token summary 🙂 段落 換行 [session context header](https://example.com/streaming-tail/webkit/streaming-tail/measurement?ref=907) `blockquote/fixtures/pretextVirtualizerRowLayout.ts/blockquote/core/web` [token parity inline](https://example.com/parity/inline-code/webkit?ref=309):",
    },
    {
      name: "md11-list-item-under",
      width: 472,
      markdown:
        "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding `pretextVirtualizerRowLayout.ts/pages/turn-header/sessionMarkdownMeasurement.ts/core/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx`.",
    },
    {
      name: "md11-list-item-788-part0",
      width: 788,
      markdown:
        "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding `pretextVirtualizerRowLayout.ts/pages/turn-header/sessionMarkdownMeasurement.ts/core/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx`.",
    },
    {
      name: "md15-list-item-over",
      width: 472,
      markdown:
        "- Agent shell deterministic command [agent](https://example.com/chromium/docs?ref=933) probe parity entry summary header session entry turn *fragment stream* `fixtures/turn-header/pages/fixtures` pretext agent session delta.",
    },
    {
      name: "md15-list-item-540-part0",
      width: 540,
      markdown:
        "- Parity summary delta `pretextVirtualizerRowLayout.ts/workbenchShell/core/workbenchShell/sessionThread/turn-header/pages` 🧪 段落 換行 *parity* **token header** `ctx task list` [composer](https://example.com/measurement/chromium?ref=656);",
    },
    {
      name: "md15-list-item-620-part0",
      width: 620,
      markdown:
        "- Parity summary delta `pretextVirtualizerRowLayout.ts/workbenchShell/core/workbenchShell/sessionThread/turn-header/pages` 🧪 段落 換行 *parity* **token header** `ctx task list` [composer](https://example.com/measurement/chromium?ref=656);",
    },
    {
      name: "md18-blockquote-under",
      width: 472,
      markdown:
        "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures` *marker buffer stream* **header message command** 🧪 你好 世界;",
    },
    {
      name: "md18-blockquote-540-full",
      width: 540,
      markdown: [
        "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures` *marker buffer stream* **header message command** 🧪 你好 世界;",
        ">",
        "> Agent parity thread virtualizer [parity](https://example.com/virtualizer/docs/chromium?ref=521) 🙂 測試 佈局 agent marker shell deterministic *buffer command token* session virtualizer pretext marker [message](https://example.com/transcript/transcript?ref=991):",
      ].join("\n"),
    },
    {
      name: "message4-prefix4-code2",
      width: 382.48,
      markdown:
        "Entry command command `turn-header/workbenchShell/blockquote/workbenchShell/src/pages/core` *composer* `sessionThread/src/apps/web/inline-code/pretextVirtualizerRowLayout.ts`",
    },
    {
      name: "message10-line13-620",
      width: 518.64,
      markdown:
        "Composer layout context thread **inline virtualizer session** padding padding `apps/sessionMarkdownMeasurement.ts/pages/sessionThread/workbenchShell` *marker render stream* entry summary parity [message marker](https://example.com/parity/webkit?ref=290). `cargo test -p codex-crp`",
    },
    {
      name: "message10-line20-620",
      width: 518.64,
      markdown:
        "Stream browser agent ⚙️ 測試 佈局 header deterministic thread summary buffer entry context ⚙️ 你好 世界 `web/fixtures/core/fixtures`. `pnpm -C core/apps/web test:e2e:pretext:corpus:webkit`",
    },
  ] as const;

  for (const probe of probes) {
    const details = await page.evaluate(
      async ({ markdown, width, name }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = "*";
        win.__ctxInlineCodeDebugWidth = undefined;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.([{ name, markdown }], width);
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(markdown, width);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const fragments = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .code-token-fragment"),
        ).map((node) => {
          const rect = node.getBoundingClientRect();
          return {
            text: node.textContent ?? "",
            top: rect.top,
            left: rect.left,
            width: rect.width,
          };
        });
        const paragraph = document.querySelector<HTMLElement>(
          "#markdown-scroll-probe p, #markdown-scroll-probe li, #markdown-scroll-probe blockquote",
        );
        const paragraphHeight = paragraph?.getBoundingClientRect().height ?? null;

        win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return { name, width, parity, planner, paragraphHeight, fragments };
      },
      probe,
    );

    console.log(JSON.stringify({ kind: "isolated-inline-item-packing", ...details }, null, 2));
  }
});

test("tmp inspect remaining inline-code prefix decompositions", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      width: 620,
      samples: [
        {
          name: "md9-prefix",
          markdown: "- Pretext browser **browser delta**",
        },
        {
          name: "md9-prefix-code",
          markdown: "- Pretext browser **browser delta** `web/inline-code/src/apps/e2e/blockquote/inline-code`",
        },
        {
          name: "md9-full",
          markdown:
            "- Pretext browser **browser delta** `web/inline-code/src/apps/e2e/blockquote/inline-code` 📏 你好 世界 summary fragment browser entry [message layout parity](https://example.com/assistant/assistant/transcript?ref=833) ~~marker~~:",
        },
      ],
    },
    {
      width: 472,
      samples: [
        {
          name: "md15-prefix",
          markdown:
            "- Agent shell deterministic command [agent](https://example.com/chromium/docs?ref=933) probe parity entry summary header session entry turn *fragment stream*",
        },
        {
          name: "md15-prefix-code",
          markdown:
            "- Agent shell deterministic command [agent](https://example.com/chromium/docs?ref=933) probe parity entry summary header session entry turn *fragment stream* `fixtures/turn-header/pages/fixtures`",
        },
        {
          name: "md15-full",
          markdown:
            "- Agent shell deterministic command [agent](https://example.com/chromium/docs?ref=933) probe parity entry summary header session entry turn *fragment stream* `fixtures/turn-header/pages/fixtures` pretext agent session delta.",
        },
        {
          name: "md18-prefix",
          markdown:
            "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command",
        },
        {
          name: "md18-prefix-code",
          markdown:
            "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures`",
        },
        {
          name: "md18-full",
          markdown:
            "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures` *marker buffer stream* **header message command** 🧪 你好 世界;",
        },
      ],
    },
  ] as const;

  for (const probe of probes) {
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: probe.samples, probeWidth: probe.width },
    );

    console.log(JSON.stringify({ kind: "remaining-inline-prefixes", width: probe.width, results }, null, 2));
  }
});

test("tmp inspect remaining inline-code full line boxes", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      name: "md9-full",
      width: 620,
      markdown:
        "- Pretext browser **browser delta** `web/inline-code/src/apps/e2e/blockquote/inline-code` 📏 你好 世界 summary fragment browser entry [message layout parity](https://example.com/assistant/assistant/transcript?ref=833) ~~marker~~:",
    },
    {
      name: "md8-list-item-540-part0",
      width: 540,
      markdown:
        "- Deterministic agent **parity session** virtualizer browser message virtualizer inline *pretext virtualizer* [turn thread layout](https://example.com/transcript/transcript?ref=450) `fixtures/workbenchShell/fixtures/core/pretextVirtualizerRowLayout.ts/table/src`:",
    },
    {
      name: "md15-list-item-540-part0",
      width: 540,
      markdown:
        "- Parity summary delta `pretextVirtualizerRowLayout.ts/workbenchShell/core/workbenchShell/sessionThread/turn-header/pages` 🧪 段落 換行 *parity* **token header** `ctx task list` [composer](https://example.com/measurement/chromium?ref=656);",
    },
    {
      name: "md15-full",
      width: 472,
      markdown:
        "- Agent shell deterministic command [agent](https://example.com/chromium/docs?ref=933) probe parity entry summary header session entry turn *fragment stream* `fixtures/turn-header/pages/fixtures` pretext agent session delta.",
    },
    {
      name: "md18-block1-540",
      width: 540,
      markdown:
        "Probe deterministic summary entry [token fragment](https://example.com/parity/virtualizer?ref=329) command pretext virtualizer buffer layout marker *parity* `sessionMarkdownMeasurement.ts/inline-code/sessionMarkdownMeasurement.ts/core` deterministic command command parity:\nInline shell turn ⚙️ 你好 世界 📏 測試 佈局 🙂 測試 佈局.",
    },
    {
      name: "md10-block3-788",
      width: 788,
      markdown:
        "Session thread turn delta 📏 你好 世界 thread stream virtualizer composer [token agent](https://example.com/inline-code/docs/chromium?ref=901) context context shell fragment thread `inline-code/e2e/workbenchShell/sessionThread` ~~summary inline~~:",
    },
    {
      name: "md18-prefix-code",
      width: 472,
      markdown:
        "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures`",
    },
  ] as const;

  for (const probe of probes) {
    const details = await page.evaluate(
      async ({ markdown, width, name }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        }).__ctxE2E;
        await e2e?.installMarkdownScrollProbe?.(markdown, width);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const paragraph = document.querySelector<HTMLElement>(
          "#markdown-scroll-probe p, #markdown-scroll-probe li, #markdown-scroll-probe blockquote",
        );
        if (!paragraph) {
          throw new Error(`missing paragraph for ${name}`);
        }
        const range = document.createRange();
        range.selectNodeContents(paragraph);
        const lineRects = Array.from(range.getClientRects()).map((rect) => ({
          top: rect.top,
          left: rect.left,
          width: rect.width,
          height: rect.height,
        }));
        const paragraphRect = paragraph.getBoundingClientRect();
        const codeRects = Array.from(paragraph.querySelectorAll("code")).map((node) => ({
          text: node.textContent ?? "",
          rects: Array.from((node as HTMLElement).getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          })),
        }));

        e2e?.removeMarkdownScrollProbe?.();
        return { name, width, paragraphRect, lineRects, codeRects };
      },
      probe,
    );

    console.log(JSON.stringify({ kind: "remaining-inline-line-boxes", ...details }, null, 2));
  }
});

test("tmp inspect standalone markdown parity at row text widths", async ({ page }) => {
  await openWorkbenchShell(page);

  const corpus = generatePretextParityFuzzCorpus();
  const probes = [
    {
      sampleName: "generated-assistant-3-generated-md-103-heading-list",
      width: resolveSessionThreadAssistantTextWidth(472),
    },
    {
      sampleName: "generated-assistant-3-generated-md-103-heading-list",
      width: resolveSessionThreadAssistantTextWidth(540),
    },
    {
      sampleName: "generated-assistant-3-generated-md-103-heading-list",
      width: resolveSessionThreadAssistantTextWidth(620),
    },
    {
      sampleName: "generated-message-1-generated-md-1-heading-list-table-list",
      width: resolveSessionThreadMessageTextWidth(472),
    },
    {
      sampleName: "generated-message-1-generated-md-1-heading-list-table-list",
      width: resolveSessionThreadMessageTextWidth(540),
    },
    {
      sampleName: "generated-message-1-generated-md-1-heading-list-table-list",
      width: resolveSessionThreadMessageTextWidth(620),
    },
    {
      sampleName: "generated-message-4-generated-md-4-table-paragraph-blockquote-heading",
      width: resolveSessionThreadMessageTextWidth(540),
    },
    {
      sampleName: "generated-message-8-generated-md-8-table-nested-list-fence-paragraph",
      width: resolveSessionThreadMessageTextWidth(540),
    },
    {
      sampleName: "generated-message-12-generated-md-12-nested-list-list-nested-list",
      width: resolveSessionThreadMessageTextWidth(620),
    },
    {
      sampleName: "generated-assistant-7-generated-md-107-blockquote-fence-list-hard-break",
      width: resolveSessionThreadAssistantTextWidth(788),
    },
  ] as const;

  const sampleByName = new Map(
    [...corpus.messageSamples, ...corpus.assistantSamples].map((sample) => [sample.name, sample.params.content]),
  );

  for (const probe of probes) {
    const markdown = sampleByName.get(probe.sampleName);
    if (!markdown) {
      throw new Error(`missing sample: ${probe.sampleName}`);
    }
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      {
        probeSamples: [{ name: `${probe.sampleName}:${probe.width}`, markdown }],
        probeWidth: probe.width,
      },
    );

    console.log(
      JSON.stringify(
        {
          kind: "row-width-markdown",
          sampleName: probe.sampleName,
          width: probe.width,
          results,
        },
        null,
        2,
      ),
    );
  }
});

test("tmp inspect assistant3 row-width paragraph details", async ({ page }) => {
  await openWorkbenchShell(page);

  const markdown =
    "Turn fragment composer `sessionMarkdownMeasurement.ts/pretextVirtualizerRowLayout.ts/sessionMarkdownMeasurement.ts/sessionMarkdownMeasurement.ts` `ctx task list` probe command render buffer delta `inline-code/turn-header/sessionThreadDomMeasurement.tsx/sessionThread/web/e2e/core` marker header turn deterministic render virtualizer;";
  const widths = [508, 588] as const;

  for (const width of widths) {
    const details = await page.evaluate(
      async ({ probeMarkdown, probeWidth }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget =
          "inline-code/turn-header/sessionThreadDomMeasurement.tsx/sessionThread/web/e2e/core";
        win.__ctxInlineCodeDebugWidth = probeWidth;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.(
          [{ name: `assistant3-block1-${probeWidth}`, markdown: probeMarkdown }],
          probeWidth,
        );
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(probeMarkdown, probeWidth);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const paragraph = document.querySelector<HTMLElement>("#markdown-scroll-probe p");
        if (!(paragraph instanceof HTMLElement)) {
          throw new Error("missing paragraph");
        }

        const range = document.createRange();
        range.selectNodeContents(paragraph);
        const lineRects = Array.from(range.getClientRects()).map((rect) => ({
          top: rect.top,
          left: rect.left,
          width: rect.width,
          height: rect.height,
        }));
        const codeFragments = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .code-token-fragment"),
        ).map((node) => {
          const rect = node.getBoundingClientRect();
          return {
            text: node.textContent ?? "",
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          };
        });
        const codeRects = Array.from(paragraph.querySelectorAll<HTMLElement>("code")).map((node) => ({
          text: node.textContent ?? "",
          rects: Array.from(node.getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          })),
        }));

        await win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return { width: probeWidth, parity, planner, lineRects, codeFragments, codeRects };
      },
      { probeMarkdown: markdown, probeWidth: width },
    );

    console.log(JSON.stringify({ kind: "assistant3-row-width-details", ...details }, null, 2));
  }
});

test("tmp inspect assistant3 row-width prefix decomposition", async ({ page }) => {
  await openWorkbenchShell(page);

  const prefix =
    "Turn fragment composer `sessionMarkdownMeasurement.ts/pretextVirtualizerRowLayout.ts/sessionMarkdownMeasurement.ts/sessionMarkdownMeasurement.ts` `ctx task list` probe command render buffer delta";
  const code = "`inline-code/turn-header/sessionThreadDomMeasurement.tsx/sessionThread/web/e2e/core`";
  const tail = "marker header turn deterministic render virtualizer;";

  const samples = [
    { name: "assistant3-prefix", markdown: prefix },
    { name: "assistant3-prefix-code", markdown: `${prefix} ${code}` },
    { name: "assistant3-full", markdown: `${prefix} ${code} ${tail}` },
  ] as const;

  for (const width of [588, 756] as const) {
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: samples, probeWidth: width },
    );

    console.log(JSON.stringify({ kind: "assistant3-prefix-decomposition", width, results }, null, 2));
  }
});

test("tmp inspect remaining paragraph decompositions", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      name: "assistant2-block2-440",
      width: 440,
      parts: [
        "Agent header entry marker *header* stream summary layout entry summary 🙂 測試 佈局",
        "`core/e2e/pretextVirtualizerRowLayout.ts/sessionThreadDomMeasurement.tsx/apps/turn-header`",
        "`cargo test -p ctx-http`.",
      ],
    },
    {
      name: "message4-block2-382",
      width: 382.48,
      parts: [
        "Entry command command",
        "`turn-header/workbenchShell/blockquote/workbenchShell/src/pages/core`",
        "*composer*",
        "`sessionThread/src/apps/web/inline-code/pretextVirtualizerRowLayout.ts`",
        "*parity message* **layout** composer layout context header header session pretext browser 📏 你好 世界.",
      ],
    },
  ] as const;

  for (const probe of probes) {
    const variants = probe.parts.map((_, index) => ({
      name: `${probe.name}:prefix:${index + 1}`,
      markdown: probe.parts.slice(0, index + 1).join(" "),
    }));
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: variants, probeWidth: probe.width },
    );

    console.log(JSON.stringify({ kind: "remaining-paragraph-decomposition", probe, results }, null, 2));
  }
});

test("tmp inspect message1 row-width table details", async ({ page }) => {
  await openWorkbenchShell(page);

  const markdown = [
    "| Kind | Token | Note |",
    "|---|---|---|",
    "| entry buffer | `table/pages/inline-code/blockquote/sessionMarkdownMeasurement.ts` | summary fragment composer fragment marker entry agent command |",
    "| turn virtualizer | `git status` | virtualizer shell probe message buffer deterministic virtualizer browser |",
    "| entry | `pages/apps/inline-code/blockquote/turn-header/fixtures/web` | turn message virtualizer inline thread composer command session |",
  ].join("\n");
  const widths = [382.48, 445.04, 518.64] as const;

  for (const width of widths) {
    const details = await page.evaluate(
      async ({ probeMarkdown, probeWidth }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = "pages/apps/inline-code/blockquote/turn-header/fixtures/web";
        win.__ctxInlineCodeDebugWidth = probeWidth;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.(
          [{ name: `message1-table-block3-${probeWidth}`, markdown: probeMarkdown }],
          probeWidth,
        );
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(probeMarkdown, probeWidth);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const table = document.querySelector<HTMLElement>("#markdown-scroll-probe .wb-md-table");
        const tableRect = table?.getBoundingClientRect() ?? null;
        const rowHeights = Array.from(
          document.querySelectorAll<HTMLElement>("#markdown-scroll-probe .wb-md-table-row"),
        ).map((row, rowIndex) => {
          const rowRect = row.getBoundingClientRect();
          const cells = Array.from(row.querySelectorAll<HTMLElement>(".wb-md-table-cell")).map((cell) => {
            const rect = cell.getBoundingClientRect();
            const range = document.createRange();
            range.selectNodeContents(cell);
            const lineRects = Array.from(range.getClientRects()).map((clientRect) => ({
              top: clientRect.top,
              left: clientRect.left,
              width: clientRect.width,
              height: clientRect.height,
            }));
            const codeRects = Array.from(cell.querySelectorAll<HTMLElement>("code")).map((code) => ({
              text: code.textContent ?? "",
              rects: Array.from(code.getClientRects()).map((clientRect) => ({
                top: clientRect.top,
                left: clientRect.left,
                width: clientRect.width,
                height: clientRect.height,
              })),
            }));
            return {
              text: cell.textContent ?? "",
              width: rect.width,
              height: rect.height,
              lineRects,
              codeRects,
            };
          });
          return {
            rowIndex,
            height: rowRect.height,
            cells,
          };
        });

        await win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return {
          width: probeWidth,
          parity,
          planner,
          tableRect:
            tableRect == null
              ? null
              : {
                  width: tableRect.width,
                  height: tableRect.height,
                },
          rowHeights,
        };
      },
      { probeMarkdown: markdown, probeWidth: width },
    );

    console.log(JSON.stringify({ kind: "message1-table-row-width-details", ...details }, null, 2));
  }
});

test("tmp inspect message1 row-width table variants", async ({ page }) => {
  await openWorkbenchShell(page);

  const rows = [
    "| entry buffer | `table/pages/inline-code/blockquote/sessionMarkdownMeasurement.ts` | summary fragment composer fragment marker entry agent command |",
    "| turn virtualizer | `git status` | virtualizer shell probe message buffer deterministic virtualizer browser |",
    "| entry | `pages/apps/inline-code/blockquote/turn-header/fixtures/web` | turn message virtualizer inline thread composer command session |",
  ];
  const widths = [382.48, 445.04, 518.64] as const;
  const variants = rows.map((row, index) => ({
    name: `message1-table-row-${index + 1}`,
    markdown: ["| Kind | Token | Note |", "|---|---|---|", row].join("\n"),
  }));

  for (const width of widths) {
    const parity = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: variants, probeWidth: width },
    );

    console.log(JSON.stringify({ kind: "message1-table-row-variants", width, parity }, null, 2));
  }
});

test("tmp inspect message1 problematic row planners", async ({ page }) => {
  await openWorkbenchShell(page);

  const probes = [
    {
      name: "message1-row1-382.48",
      width: 382.48,
      token: "*",
      markdown: [
        "| Kind | Token | Note |",
        "|---|---|---|",
        "| entry buffer | `table/pages/inline-code/blockquote/sessionMarkdownMeasurement.ts` | summary fragment composer fragment marker entry agent command |",
      ].join("\n"),
    },
    {
      name: "message1-row1-445.04",
      width: 445.04,
      token: "*",
      markdown: [
        "| Kind | Token | Note |",
        "|---|---|---|",
        "| entry buffer | `table/pages/inline-code/blockquote/sessionMarkdownMeasurement.ts` | summary fragment composer fragment marker entry agent command |",
      ].join("\n"),
    },
    {
      name: "message1-row3-382.48",
      width: 382.48,
      token: "*",
      markdown: [
        "| Kind | Token | Note |",
        "|---|---|---|",
        "| entry | `pages/apps/inline-code/blockquote/turn-header/fixtures/web` | turn message virtualizer inline thread composer command session |",
      ].join("\n"),
    },
  ] as const;

  for (const probe of probes) {
    await page.reload();
    await openWorkbenchShell(page);

    const details = await page.evaluate(
      async ({ markdown, width, name, token }) => {
        const win = window as Window & {
          __ctxForceInlineCodeDebug?: boolean;
          __ctxInlineCodeDebugTarget?: string;
          __ctxInlineCodeDebugWidth?: number;
          __ctxInlineCodeDebug?: unknown;
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
            installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
            removeMarkdownScrollProbe?: () => void;
          };
        };

        win.__ctxForceInlineCodeDebug = true;
        win.__ctxInlineCodeDebugTarget = token;
        win.__ctxInlineCodeDebugWidth = width;
        win.__ctxInlineCodeDebug = null;
        const parity = await win.__ctxE2E?.measureMarkdownParity?.([{ name, markdown }], width);
        const planner = win.__ctxInlineCodeDebug ?? null;

        await win.__ctxE2E?.installMarkdownScrollProbe?.(markdown, width);
        await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));

        const codeRects = Array.from(document.querySelectorAll<HTMLElement>("#markdown-scroll-probe code")).map((node) => ({
          text: node.textContent ?? "",
          rects: Array.from(node.getClientRects()).map((rect) => ({
            top: rect.top,
            left: rect.left,
            width: rect.width,
            height: rect.height,
          })),
        }));

        await win.__ctxE2E?.removeMarkdownScrollProbe?.();
        return { name, width, parity, planner, codeRects };
      },
      probe,
    );

    console.log(JSON.stringify({ kind: "message1-problem-row-planners", ...details }, null, 2));
  }
});

test("tmp inspect message1 problematic row cell variants", async ({ page }) => {
  await openWorkbenchShell(page);

  const samples = [
    {
      name: "row1-code-only",
      markdown: ["| Kind | Token | Note |", "|---|---|---|", "| entry buffer | `table/pages/inline-code/blockquote/sessionMarkdownMeasurement.ts` |  |"].join("\n"),
    },
    {
      name: "row1-note-only",
      markdown: ["| Kind | Token | Note |", "|---|---|---|", "| entry buffer |  | summary fragment composer fragment marker entry agent command |"].join("\n"),
    },
    {
      name: "row3-code-only",
      markdown: ["| Kind | Token | Note |", "|---|---|---|", "| entry | `pages/apps/inline-code/blockquote/turn-header/fixtures/web` |  |"].join("\n"),
    },
    {
      name: "row3-note-only",
      markdown: ["| Kind | Token | Note |", "|---|---|---|", "| entry |  | turn message virtualizer inline thread composer command session |"].join("\n"),
    },
  ] as const;
  const widths = [382.48, 445.04] as const;

  for (const width of widths) {
    const parity = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: samples, probeWidth: width },
    );

    console.log(JSON.stringify({ kind: "message1-problem-row-cell-variants", width, parity }, null, 2));
  }
});

test("tmp inspect row-width markdown prefixes", async ({ page }) => {
  await openWorkbenchShell(page);

  const corpus = generatePretextParityFuzzCorpus();
  const probes = [
    {
      sampleName: "generated-assistant-3-generated-md-103-heading-list",
      width: resolveSessionThreadAssistantTextWidth(540),
    },
    {
      sampleName: "generated-assistant-3-generated-md-103-heading-list",
      width: resolveSessionThreadAssistantTextWidth(620),
    },
    {
      sampleName: "generated-message-1-generated-md-1-heading-list-table-list",
      width: resolveSessionThreadMessageTextWidth(472),
    },
    {
      sampleName: "generated-message-1-generated-md-1-heading-list-table-list",
      width: resolveSessionThreadMessageTextWidth(540),
    },
    {
      sampleName: "generated-message-1-generated-md-1-heading-list-table-list",
      width: resolveSessionThreadMessageTextWidth(620),
    },
  ] as const;

  const sampleByName = new Map(
    [...corpus.messageSamples, ...corpus.assistantSamples].map((sample) => [sample.name, sample.params.content]),
  );

  for (const probe of probes) {
    const markdown = sampleByName.get(probe.sampleName);
    if (!markdown) {
      throw new Error(`missing sample: ${probe.sampleName}`);
    }
    const blocks = markdown.split("\n\n");
    const variants = blocks.flatMap((block, index) => {
      const prefix = blocks.slice(0, index + 1).join("\n\n");
      return [
        { name: `${probe.sampleName}:block:${index}`, markdown: block },
        { name: `${probe.sampleName}:prefix:${index}`, markdown: prefix },
      ];
    });
    const results = await page.evaluate(
      async ({ probeSamples, probeWidth }) => {
        const e2e = (window as Window & {
          __ctxE2E?: {
            measureMarkdownParity?: (
              samples: ReadonlyArray<{ name: string; markdown: string }>,
              width: number,
            ) => Promise<Array<{ name: string; planned: number; actual: number; delta: number }>>;
          };
        }).__ctxE2E;
        if (typeof e2e?.measureMarkdownParity !== "function") {
          throw new Error("markdown parity bridge unavailable");
        }
        return e2e.measureMarkdownParity(probeSamples, probeWidth);
      },
      { probeSamples: variants, probeWidth: probe.width },
    );
    console.log(
      JSON.stringify(
        {
          kind: "row-width-prefixes",
          sampleName: probe.sampleName,
          width: probe.width,
          results: results.filter((entry) => Math.abs(entry.delta) > 0),
        },
        null,
        2,
      ),
    );
  }
});

test("tmp inspect turn-header planner vs browser lines", async ({ page }) => {
  await openWorkbenchShell(page);

  const corpus = generatePretextParityFuzzCorpus();
  const probes = [
    { name: "generated-turn-header-1", width: 540, prefix: 1 },
    { name: "generated-turn-header-1", width: 540, prefix: 2 },
    { name: "generated-turn-header-2", width: 472, prefix: 1 },
    { name: "generated-turn-header-3", width: 540, prefix: 1 },
    { name: "generated-turn-header-3", width: 540, prefix: 2 },
    { name: "generated-turn-header-3", width: 540, prefix: 3 },
    { name: "generated-turn-header-6", width: 472, prefix: 1 },
    { name: "generated-turn-header-6", width: 472, prefix: 2 },
    { name: "generated-turn-header-8", width: 540, prefix: 1 },
  ] as const;

  for (const probe of probes) {
    const plainText =
      corpus.turnHeaderSamples.find((sample) => sample.name === probe.name)?.params.plainText ?? "";
    const line = plainText.split("\n").slice(0, probe.prefix).join("\n");
    const details = await page.evaluate(
      async ({ plainText: targetLine, viewportWidth, textWidth }) => {
        const win = window as Window & {
          __ctxForcePlainTextDebug?: boolean;
          __ctxPlainTextDebugTarget?: string;
          __ctxPlainTextDebugWidth?: number;
          __ctxPlainTextDebug?: unknown;
          __ctxE2E?: {
            measureTurnHeaderParity?: (params: {
              plainText: string;
              viewportWidth?: number;
            }) => Promise<{ planned: number; actual: number; delta: number }>;
          };
        };
        const measureTurnHeaderParity = win.__ctxE2E?.measureTurnHeaderParity;
        if (typeof measureTurnHeaderParity !== "function") {
          throw new Error("turn header parity bridge unavailable");
        }

        win.__ctxForcePlainTextDebug = true;
        win.__ctxPlainTextDebugTarget = targetLine;
        win.__ctxPlainTextDebugWidth = textWidth;
        win.__ctxPlainTextDebug = null;
        const parity = await measureTurnHeaderParity({ plainText: targetLine, viewportWidth });
        const planner = win.__ctxPlainTextDebug ?? null;

        const host = document.createElement("div");
        host.style.position = "fixed";
        host.style.left = "-10000px";
        host.style.top = "0";
        host.style.width = `${textWidth + 46}px`;
        host.style.setProperty("--wb-markdown-body-font-family", '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif');
        host.style.setProperty("--wb-markdown-body-font-size", "13px");
        host.style.setProperty("--wb-markdown-body-line-height", "20px");
        host.style.setProperty("--wb-turn-header-bubble-padding-block", "8px");
        host.style.setProperty("--wb-turn-header-bubble-padding-inline", "10px");
        host.style.setProperty("--wb-turn-header-bubble-border-width", "1px");
        host.style.setProperty("--wb-turn-header-copy-gutter", "24px");
        document.body.appendChild(host);

        const root = document.createElement("div");
        root.className = "wb-turn-header wb-turn-header-expanded";
        root.style.width = `${textWidth + 46}px`;
        const bubble = document.createElement("div");
        bubble.className = "wb-turn-header-bubble";
        bubble.style.width = "100%";
        const copy = document.createElement("button");
        copy.type = "button";
        copy.className = "wb-turn-header-copy";
        copy.textContent = "copy";
        const content = document.createElement("div");
        content.className = "wb-turn-header-content";
        content.style.width = `${textWidth}px`;
        const chars = Array.from(targetLine);
        for (const char of chars) {
          const span = document.createElement("span");
          span.textContent = char;
          span.dataset.grapheme = char;
          content.appendChild(span);
        }
        bubble.append(copy, content);
        root.appendChild(bubble);
        host.appendChild(root);

        const lines = new Map<number, string>();
        for (const span of Array.from(content.querySelectorAll<HTMLElement>("span"))) {
          const rect = span.getBoundingClientRect();
          const key = Math.round(rect.top);
          lines.set(key, `${lines.get(key) ?? ""}${span.dataset.grapheme ?? ""}`);
        }
        const browserLines = Array.from(lines.entries())
          .sort((a, b) => a[0] - b[0])
          .map(([, text]) => text);
        host.remove();
        return { parity, planner, browserLines };
      },
      {
        plainText: line,
        viewportWidth: probe.width,
        textWidth: resolveSessionThreadTurnHeaderTextWidth(probe.width),
      },
    );

    console.log(
      JSON.stringify(
        {
          kind: "turn-header-line-debug",
          probe,
          line,
          details,
        },
        null,
        2,
      ),
    );
  }
});
