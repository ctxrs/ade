import { promises as fs } from "fs";
import { expect, test } from "./fixtures";
import {
  measureAssistantParity,
  measureMarkdownParity,
  measureMessageParity,
  measureTurnHeaderParity,
  openWorkbenchShell,
} from "./utils/pretextParity";
import { generatePretextParityFuzzCorpus } from "./utils/pretextParityFuzz";

const ENFORCE = process.env.CTX_PRETEXT_PARITY_ENFORCE === "1";
const MARKDOWN_THRESHOLD_PX = 1;
const ROW_THRESHOLD_PX = 1;

const DEFAULT_SEED = 20260412;
const DEFAULT_MARKDOWN_CASES = 18;
const DEFAULT_MESSAGE_CASES = 12;
const DEFAULT_ASSISTANT_CASES = 12;
const DEFAULT_TURN_HEADER_CASES = 8;

type MarkdownSummaryEntry = {
  width: number;
  name: string;
  markdown: string;
  planned: number;
  actual: number;
  delta: number;
};

type RowSummaryEntry = {
  kind: "message" | "assistant" | "turn_header";
  width: number;
  name: string;
  content: string;
  planned: number;
  actual: number;
  delta: number;
  expanded?: boolean;
  isComplete?: boolean;
  attachmentCount?: number;
};

function readEnvInt(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) return fallback;
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function formatFailures(
  kind: string,
  failures: Array<{ name: string; width: number; delta: number; planned: number; actual: number }>,
): string {
  return `${kind} fuzz parity failures:\n${failures
    .map(
      (failure) =>
        `- width ${failure.width} ${failure.name}: delta=${failure.delta}px planned=${failure.planned} actual=${failure.actual}`,
    )
    .join("\n")}`;
}

test("workbench: pretext generated markdown fuzz parity", async ({ page }, testInfo) => {
  test.setTimeout(240000);
  test.slow();
  await openWorkbenchShell(page);

  const corpus = generatePretextParityFuzzCorpus({
    seed: readEnvInt("CTX_PRETEXT_FUZZ_SEED", DEFAULT_SEED),
    markdownCount: readEnvInt("CTX_PRETEXT_FUZZ_MARKDOWN_CASES", DEFAULT_MARKDOWN_CASES),
    messageCount: readEnvInt("CTX_PRETEXT_FUZZ_MESSAGE_CASES", DEFAULT_MESSAGE_CASES),
    assistantCount: readEnvInt("CTX_PRETEXT_FUZZ_ASSISTANT_CASES", DEFAULT_ASSISTANT_CASES),
    turnHeaderCount: readEnvInt("CTX_PRETEXT_FUZZ_TURN_HEADER_CASES", DEFAULT_TURN_HEADER_CASES),
  });

  const summary: MarkdownSummaryEntry[] = [];
  for (const width of corpus.widths) {
    const measurements = await measureMarkdownParity(page, corpus.markdownSamples, width);
    summary.push(
      ...measurements.map((measurement, index) => ({
        width,
        markdown: corpus.markdownSamples[index]!.markdown,
        ...measurement,
      })),
    );
  }

  const report = {
    enforce: ENFORCE,
    seed: corpus.seed,
    thresholdPx: MARKDOWN_THRESHOLD_PX,
    widths: corpus.widths,
    counts: {
      markdown: corpus.markdownSamples.length,
    },
    samples: summary,
  };
  const reportPath = testInfo.outputPath("pretext-markdown-fuzz-parity.json");
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), "utf8");
  await testInfo.attach("pretext-markdown-fuzz-parity.json", {
    path: reportPath,
    contentType: "application/json",
  });

  if (!ENFORCE) return;

  const failures = summary.filter((entry) => Math.abs(entry.delta) > MARKDOWN_THRESHOLD_PX);
  expect(
    failures,
    formatFailures(
      "markdown",
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

test("workbench: pretext generated transcript row fuzz parity", async ({ page }, testInfo) => {
  test.setTimeout(240000);
  test.slow();
  await openWorkbenchShell(page);

  const corpus = generatePretextParityFuzzCorpus({
    seed: readEnvInt("CTX_PRETEXT_FUZZ_SEED", DEFAULT_SEED),
    markdownCount: readEnvInt("CTX_PRETEXT_FUZZ_MARKDOWN_CASES", DEFAULT_MARKDOWN_CASES),
    messageCount: readEnvInt("CTX_PRETEXT_FUZZ_MESSAGE_CASES", DEFAULT_MESSAGE_CASES),
    assistantCount: readEnvInt("CTX_PRETEXT_FUZZ_ASSISTANT_CASES", DEFAULT_ASSISTANT_CASES),
    turnHeaderCount: readEnvInt("CTX_PRETEXT_FUZZ_TURN_HEADER_CASES", DEFAULT_TURN_HEADER_CASES),
  });

  const summary: RowSummaryEntry[] = [];

  for (const width of corpus.widths) {
    for (const sample of corpus.messageSamples) {
      const measurement = await measureMessageParity(page, {
        ...sample.params,
        viewportWidth: width,
      });
      summary.push({
        kind: "message",
        width,
        name: sample.name,
        content: sample.params.content,
        expanded: sample.params.expanded,
        attachmentCount: sample.params.attachments?.length ?? 0,
        ...measurement,
      });
    }

    for (const sample of corpus.assistantSamples) {
      const measurement = await measureAssistantParity(page, {
        ...sample.params,
        viewportWidth: width,
      });
      summary.push({
        kind: "assistant",
        width,
        name: sample.name,
        content: sample.params.content,
        isComplete: sample.params.isComplete ?? true,
        ...measurement,
      });
    }

    for (const sample of corpus.turnHeaderSamples) {
      const measurement = await measureTurnHeaderParity(page, {
        ...sample.params,
        viewportWidth: width,
      });
      summary.push({
        kind: "turn_header",
        width,
        name: sample.name,
        content: sample.params.plainText,
        ...measurement,
      });
    }
  }

  const report = {
    enforce: ENFORCE,
    seed: corpus.seed,
    thresholdPx: ROW_THRESHOLD_PX,
    widths: corpus.widths,
    counts: {
      message: corpus.messageSamples.length,
      assistant: corpus.assistantSamples.length,
      turnHeader: corpus.turnHeaderSamples.length,
    },
    samples: summary,
  };
  const reportPath = testInfo.outputPath("pretext-row-fuzz-parity.json");
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), "utf8");
  await testInfo.attach("pretext-row-fuzz-parity.json", {
    path: reportPath,
    contentType: "application/json",
  });

  if (!ENFORCE) return;

  const failures = summary.filter((entry) => Math.abs(entry.delta) > ROW_THRESHOLD_PX);
  expect(
    failures,
    formatFailures(
      "row",
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
