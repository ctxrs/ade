import { expect, test } from "./fixtures";
import {
  measureAssistantParity,
  measureAssistantStreamingParity,
  measureMessageParity,
  measureTurnHeaderParity,
  openWorkbenchShell,
} from "./utils/pretextParity";

const EXACT_USER_MESSAGE = [
  "here is another neutral layout idea for a deterministic fixture",
  "",
  "we could ask the model to summarize a sample article and then write ten synthetic review notes.",
  "",
  "then we can compare whether the notes use the same broad style as the reference examples",
  "",
  "for example, a short positive note or a careful critical note can exercise different wrapping without carrying real conversation text.",
  "",
  "what other neutral variations should this layout test include",
].join("\n");

const COLLAPSED_LONG_MESSAGE = Array.from(
  { length: 24 },
  (_, index) => `line ${index + 1} with enough words to wrap a little bit`,
).join("\n");

const ASSISTANT_MARKDOWN = [
  "# Title",
  "",
  "- bullet one",
  "- bullet two",
  "",
  "```ts",
  "const x = 1;",
  "```",
].join("\n");
const ASSISTANT_INLINE_CODE_WRAP_MARKDOWN =
  "begin agent message here with plain text, and now some inline code block: `inline-thing-that-actually-gets-really-long-so-much-so-that-it-actually-wraps-to-2-lines-and-keeps-going-with-extra-path-segments/core/apps/web/src/pages/sessionThread/sessionMarkdownMeasurement.ts`";
const TURN_HEADER_TEXT = "and what about the CI smoke failure?";

const IMAGE_DATA_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO2VzJ8AAAAASUVORK5CYII=";

test("workbench: exact multi-paragraph user message planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureMessageParity(page, {
    content: EXACT_USER_MESSAGE,
    expanded: true,
  });

  expect(
    Math.abs(measurement.delta),
    `message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: collapsed toggleable user message planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureMessageParity(page, {
    content: COLLAPSED_LONG_MESSAGE,
    expanded: false,
  });

  expect(
    Math.abs(measurement.delta),
    `collapsed message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: image attachments stay in parity for message rows", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureMessageParity(page, {
    content: "two inline screenshots",
    expanded: true,
    attachments: [
      { kind: "image", mime_type: "image/png", data_base64: IMAGE_DATA_BASE64, name: "one.png" },
      { kind: "image", mime_type: "image/png", data_base64: IMAGE_DATA_BASE64, name: "two.png" },
    ],
  });

  expect(
    Math.abs(measurement.delta),
    `attachment message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: assistant markdown planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureAssistantParity(page, { content: ASSISTANT_MARKDOWN });

  expect(
    Math.abs(measurement.delta),
    `assistant drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: assistant prose with wrapped inline code matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureAssistantParity(page, { content: ASSISTANT_INLINE_CODE_WRAP_MARKDOWN });

  expect(
    Math.abs(measurement.delta),
    `assistant inline-code drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: partial assistant streaming stays identical to completed rendering for the same cumulative content", async ({
  page,
}) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureAssistantStreamingParity(page, {
    fragments: [
      "Before",
      "\n\n- partial item with `inline-tail-token/with/path`",
      "\n- second bullet with more prose to wrap near the edge",
    ],
  });

  expect(measurement.steps.length).toBe(3);
  for (const [index, step] of measurement.steps.entries()) {
    expect(
      Math.abs(step.partial.delta),
      `streaming partial step ${index} drifted by ${step.partial.delta}px (planned ${step.partial.planned}, actual ${step.partial.actual})`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(step.complete.delta),
      `streaming complete step ${index} drifted by ${step.complete.delta}px (planned ${step.complete.planned}, actual ${step.complete.actual})`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(step.actualDelta),
      `streaming actual mismatch at step ${index}: partial=${step.partial.actual} complete=${step.complete.actual}`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(step.plannedDelta),
      `streaming planned mismatch at step ${index}: partial=${step.partial.planned} complete=${step.complete.planned}`,
    ).toBeLessThanOrEqual(1);
    expect(step.structureEquivalent, `streaming structure diverged at step ${index} for content:\n${step.content}`).toBe(true);
  }
});

test("workbench: turn header planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureTurnHeaderParity(page, { plainText: TURN_HEADER_TEXT });

  expect(
    Math.abs(measurement.delta),
    `turn header drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});
