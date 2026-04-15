import { expect, test } from "./fixtures";
import {
  measureAssistantParity,
  openWorkbenchShell,
} from "./utils/pretextParity";

const REPORTED_ASSISTANT_CANARY_MESSAGE = `The layout canary uses a stable sample document with enough ordinary prose to make a real browser measurement useful. Each paragraph is invented for this fixture and describes a repeatable rendering check, so the sample can remain long without preserving a planning exchange or an operational decision.

The opening paragraph establishes the width pressure. It places a short sentence before a longer sentence, then keeps a second long sentence after the first inline marker. The renderer must carry the text through the same paragraph, retain the whitespace around the marker, and wrap the plain tail at the available width. The tail deliberately contains several clauses with different word lengths, because a single short sentence can pass while a realistic continuation exposes a line-height or inline-seam error.

The document has several ordinary paragraphs and a nested review list:
- measure the prepared text at the requested width
  - compare the result with a browser-rendered copy
    - retain the sample when both line counts agree
    - record the measured height after the final continuation
  - keep the list indentation visible when the outer item wraps
- compare the prepared block with a second width
  - preserve the same paragraph and marker order

The outer list is followed by prose so the next block starts after the list rather than being absorbed into its final item. The nested levels are intentional: the first child has a second child, the second child has its own continuation, and the outer item has a sibling after the nested group. That shape exercises indentation, paragraph boundaries, marker alignment, and the transition back to ordinary text in one actual Markdown value.

The check runs in two stages:
1. render the paragraph with \`inline-code-token\`, \`nested/list/path\`, and \`width=472\` markers
   - keep the first marker on the ordered item
     - keep the path marker together when it reaches the width edge
     - allow the plain explanation to continue on the next line
   - keep the width marker beside the sentence that follows it
2. compare the measured height with the displayed height
   - compare the first width with the second width
   - retain the larger result when the paragraph wraps

The inline markers are real code spans in the sample. The first marker is short enough to sit beside ordinary prose, the second contains slash-separated segments that create a useful unbroken run, and the width marker adds punctuation after the chip. The expected result is driven by the rendered Markdown structure, not by a sentence that merely names inline code or nested lists.

The sample also includes a fenced block between two long paragraphs. Its contents are neutral fixture data, but the fence is real Markdown so the parser must create a code block and the layout must account for its block spacing:

\`\`\`text
sample width=472
sample path=inline/code/path
sample mode=wrapped-tail
\`\`\`

Text after the fenced block continues with an ordinary paragraph. This paragraph is intentionally long enough to wrap at the narrow width, and it includes a second code chip \`sample-state=ready\` near the middle rather than at the end. The following clauses keep pressure after the chip so the test covers the line that begins after an inline element as well as the line that precedes it.

The final paragraph keeps enough plain prose after the code chips to exercise a wrapped tail. This deliberately long invented message repeats the ordinary sentence shape across several clauses so the canary covers block spacing, list indentation, inline-code chrome, fenced-code spacing, and a fresh-line continuation without recording an operational conversation. The browser should measure the complete document as one assistant message, preserving every blank line, list level, code span, fence, and trailing sentence while comparing planned and actual height.`;

test("workbench: reported assistant canary update matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureAssistantParity(page, {
    content: REPORTED_ASSISTANT_CANARY_MESSAGE,
  });

  expect(
    Math.abs(measurement.delta),
    `reported assistant drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});
