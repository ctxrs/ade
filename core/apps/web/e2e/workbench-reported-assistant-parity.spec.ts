import { expect, test } from "./fixtures";
import {
  measureAssistantParity,
  openWorkbenchShell,
} from "./utils/pretextParity";

const REPORTED_ASSISTANT_CANARY_MESSAGE = `Yes. That is the right fixture-side move.

The markdown parity sample should stay small and reviewable. The correct execution is:
- keep the component path and measurement harness in place
- replace realistic operational prose with neutral synthetic examples
- preserve lists, emphasis, code spans, and long paragraphs
- leave unrelated rendering behavior untouched

That is what this sample is modeling now. I paused to separate:
- text that exercises wrapping behavior
- markdown structure that affects planned height
- labels that only exist to make the report readable

The short version is: **yes, the fixture should be synthetic**, while still looking like the kind of assistant update that users write in real sessions.

I’m moving from inspection into fixture editing now. I’m not changing the browser harness or the measurement utility; I’m swapping the canary message for public-safe prose, then I’ll run targeted scans for the removed phrases.

The work is happening in two buckets: content that can be replaced directly, and behavior that should remain exactly as it was. After that I’ll verify that the parity assertion still compares the same planned and rendered surfaces.

The content bucket is straightforward:
- headings should remain plain markdown
- bullets should keep enough length to wrap
- inline code should appear near prose boundaries
- numbered items should still cover multi-line list layout

The behavior bucket is also intentionally narrow:
- do not change the timeout
- do not change how the workbench opens
- do not change how the assistant row is measured
- do not alter the one-pixel drift threshold

I’ve got the sample into a coherent state. The remaining work is about confirming that the rendered row still exercises realistic markdown density, not about expanding the test surface.

Current state:
- the report uses synthetic task names
- the prose describes fixture maintenance only
- code spans and list structure are still present
- no unavailable service names or implementation plans are embedded

What is left before considering the fixture ready:

1. Recheck the canary message.
- keep enough prose to trigger line wrapping
- keep short inline code tokens beside ordinary text
- keep the paragraph cadence similar to the previous sample

2. Recheck the surrounding test.
- keep the same helper call
- keep the same assertion
- keep the same timeout and workbench setup

3. Recheck nearby corpora.
- replace repeated synthetic-run notes where needed
- keep Unicode and code-path samples that exist for layout pressure
- avoid turning fixture text into product documentation

4. Recheck source text searches.
- search for the removed phrases
- inspect any remaining hits in the allowed files
- report the validation commands that were run

So the remaining checks are plain:
- fixture text scan
- settings-copy scan
- documentation wording scan
- final git status review

I’ll finish this pass by recording exactly which files changed and which focused searches passed.`;

test("workbench: reported assistant update matches rendered height", async ({ page }) => {
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
