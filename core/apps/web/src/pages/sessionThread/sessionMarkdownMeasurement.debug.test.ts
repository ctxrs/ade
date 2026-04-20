import { describe, it } from "vitest";
import { measureSessionMarkdownDocument } from "./sessionMarkdownMeasurement";
import { splitInlineCodeFragments } from "../../utils/inlineCodeFragments";

type DebugWindow = Window & {
  __ctxForceInlineCodeDebug?: unknown;
  __ctxInlineCodeDebug?: unknown;
  __ctxInlineCodeDebugTarget?: string;
  __ctxInlineCodeDebugWidth?: number;
};

describe("sessionMarkdownMeasurement debug", () => {
  it("logs remaining inline-code planner lines", () => {
    const probes = [
      {
        name: "md18-472",
        width: 472,
        markdown:
          "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures` *marker buffer stream* **header message command** 🧪 你好 世界;",
      },
      {
        name: "md18-540",
        width: 540,
        markdown:
          "> Parity probe parity probe ⚙️ 測試 佈局 delta agent browser virtualizer command `pretextVirtualizerRowLayout.ts/fixtures/apps/sessionMarkdownMeasurement.ts/turn-header/pretextVirtualizerRowLayout.ts/fixtures` *marker buffer stream* **header message command** 🧪 你好 世界;",
      },
      {
        name: "md15-540",
        width: 540,
        markdown:
          "- Parity summary delta `pretextVirtualizerRowLayout.ts/workbenchShell/core/workbenchShell/sessionThread/turn-header/pages` 🧪 段落 換行 *parity* **token header** `ctx task list` [composer](https://example.com/measurement/chromium?ref=656);",
      },
      {
        name: "md11-620",
        width: 620,
        markdown:
          "- Padding message header turn `sessionThreadDomMeasurement.tsx/apps/blockquote/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/turn-header` [session](https://example.com/inline-code/webkit/parity?ref=556) [command shell turn](https://example.com/webkit/inline-code/assistant?ref=703) render agent thread buffer padding `pretextVirtualizerRowLayout.ts/pages/turn-header/sessionMarkdownMeasurement.ts/core/sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx`.",
      },
      {
        name: "assistant3-588",
        width: 588,
        markdown:
          "Turn fragment composer `sessionMarkdownMeasurement.ts/pretextVirtualizerRowLayout.ts/sessionMarkdownMeasurement.ts/sessionMarkdownMeasurement.ts` `ctx task list` probe command render buffer delta `inline-code/turn-header/sessionThreadDomMeasurement.tsx/sessionThread/web/e2e/core` marker header turn deterministic render virtualizer;",
      },
      {
        name: "row1-code-445",
        width: 445.04,
        markdown:
          "| Kind | Token | Note |\n|---|---|---|\n| entry buffer | `table/pages/inline-code/blockquote/sessionMarkdownMeasurement.ts` |  |",
      },
      {
        name: "row1-code-382",
        width: 382.48,
        markdown:
          "| Kind | Token | Note |\n|---|---|---|\n| entry buffer | `table/pages/inline-code/blockquote/sessionMarkdownMeasurement.ts` |  |",
      },
      {
        name: "row3-code-382",
        width: 382.48,
        markdown:
          "| Kind | Token | Note |\n|---|---|---|\n| entry | `pages/apps/inline-code/blockquote/turn-header/fixtures/web` |  |",
      },
      {
        name: "message2-list-382",
        width: 382.48,
        markdown: [
          "- Agent thread token ~~message~~ `sessionThreadDomMeasurement.tsx/core/pretextVirtualizerRowLayout.ts/pages/pages/apps` **buffer delta** **composer** browser fragment composer composer shell:",
          "  - Probe probe context command composer agent parity parity padding stream padding shell *thread* ~~entry buffer~~ ⚙️ 測試 佈局.",
          "  - Entry browser **inline stream pretext** 🙂 你好 世界 *render session render* **delta render layout** 📏 你好 世界.",
        ].join("\n"),
      },
      {
        name: "message8-list-382",
        width: 382.48,
        markdown: [
          "- Thread stream parity fragment `sessionThreadDomMeasurement.tsx/inline-code/turn-header/web/turn-header/src` 🧪 測試 佈局 🙂 你好 世界:",
          "  - Buffer buffer ~~composer~~ `core/pages/pages/pages` ~~buffer~~;",
          "  - Deterministic parity message stream `git rev-parse HEAD` [padding entry](https://example.com/inline-code/transcript?ref=676) *message agent* *session entry header* *summary summary*:",
        ].join("\n"),
      },
      {
        name: "assistant11-table-440",
        width: 440,
        markdown: [
          "| Kind | Token | Note |",
          "|---|---|---|",
          "| pretext stream | `pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/blockquote/turn-header/workbenchShell/fixtures` | thread shell entry token delta |",
          "| entry probe | `pretextVirtualizerRowLayout.ts/pages/inline-code/table` | buffer marker inline turn |",
          "| browser | `turn-header/blockquote/blockquote/src/web/e2e/workbenchShell` | composer stream deterministic fragment token marker delta |",
        ].join("\n"),
      },
      {
        name: "md14-table-heading-blockquote-540",
        width: 540,
        markdown: [
          "| Kind | Token | Note |",
          "|---|---|---|",
          "| agent | `apps/e2e/e2e/core/web/pretextVirtualizerRowLayout.ts` | entry agent virtualizer browser |",
          "| fragment session | `pnpm -C core/apps/web test:e2e:pretext:parity:webkit` | entry header fragment virtualizer summary fragment thread composer |",
          "",
          "## Context browser",
          "",
          "Entry probe browser ~~session~~ *marker layout fragment* 🙂 你好 世界.",
          "",
          "> Turn padding stream ~~summary~~ 🙂 段落 換行 **thread** [inline](https://example.com/assistant/transcript?ref=259);",
          ">",
          "> Padding composer render **context** ~~deterministic~~ `pnpm -C core/apps/web test:e2e:pretext:parity:webkit` summary delta *render agent inline*.",
        ].join("\n"),
      },
    ] as const;

    for (const probe of probes) {
      const debugWindow = window as DebugWindow;
      debugWindow.__ctxForceInlineCodeDebug = true;
      debugWindow.__ctxInlineCodeDebug = undefined;
      debugWindow.__ctxInlineCodeDebugTarget = "*";
      debugWindow.__ctxInlineCodeDebugWidth = probe.width;
      const height = measureSessionMarkdownDocument(probe.markdown, probe.width);
      // eslint-disable-next-line no-console
      console.log(
        JSON.stringify(
          {
            name: probe.name,
            width: probe.width,
            height,
            debug: debugWindow.__ctxInlineCodeDebug ?? null,
          },
          null,
          2,
        ),
      );
    }

    // eslint-disable-next-line no-console
    console.log(
      JSON.stringify(
        {
          fragments: {
            message2: splitInlineCodeFragments(
              "sessionThreadDomMeasurement.tsx/core/pretextVirtualizerRowLayout.ts/pages/pages/apps",
            ),
            message8: splitInlineCodeFragments(
              "sessionThreadDomMeasurement.tsx/inline-code/turn-header/web/turn-header/src",
            ),
            assistant11: splitInlineCodeFragments(
              "pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/pretextVirtualizerRowLayout.ts/blockquote/turn-header/workbenchShell/fixtures",
            ),
          },
        },
        null,
        2,
      ),
    );

  });
});
