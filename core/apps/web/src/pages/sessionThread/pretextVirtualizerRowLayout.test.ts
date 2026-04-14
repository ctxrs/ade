import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkbenchListItem } from "../sessionView";

const { prepareMock, prepareWithSegmentsMock, layoutMock, layoutNextLineMock } = vi.hoisted(() => ({
  prepareMock: vi.fn((text: string, font: string, options?: { whiteSpace?: "normal" | "pre-wrap" }) => ({
    text,
    font,
    options,
  })),
  prepareWithSegmentsMock: vi.fn((text: string, font: string, options?: { whiteSpace?: "normal" | "pre-wrap" }) => {
    const segments = text.match(/\s+|\S+/g) ?? [];
    const mono = font.toLowerCase().includes("mono");
    return {
      text,
      font,
      options,
      segments,
      widths: segments.map((segment) => (mono ? 8 : 6) * Math.max(1, segment.length)),
      kinds: segments.map((segment) => (/^\s+$/.test(segment) ? "space" : "text")),
      breakableWidths: segments.map((segment) =>
        /^\s+$/.test(segment)
          ? null
          : Array.from(segment).map(() => (mono ? 8 : 6)),
      ),
      breakablePrefixWidths: segments.map(() => null),
      lineEndFitAdvances: segments.map((segment) => (/^\s+$/.test(segment) ? 0 : (mono ? 8 : 6) * Math.max(1, segment.length))),
      lineEndPaintAdvances: segments.map((segment) => (/^\s+$/.test(segment) ? 0 : (mono ? 8 : 6) * Math.max(1, segment.length))),
      simpleLineWalkFastPath: true,
      segLevels: null,
      discretionaryHyphenWidth: 0,
      tabStopAdvance: 0,
      chunks: [],
    };
  }),
  layoutNextLineMock: vi.fn(
    (
      prepared: { text: string },
      start: { segmentIndex: number; graphemeIndex: number },
      maxWidth: number,
    ) => {
      const text = prepared.text ?? "";
      const startIndex = Math.max(0, start.segmentIndex);
      if (startIndex >= text.length) {
        return null;
      }
      const maxChars = Math.max(1, Math.floor(maxWidth / 10));
      let endIndex = Math.min(text.length, startIndex + maxChars);
      if (endIndex < text.length) {
        const lastSpace = text.lastIndexOf(" ", endIndex - 1);
        if (lastSpace >= startIndex) {
          endIndex = lastSpace + 1;
        }
      }
      if (endIndex <= startIndex) {
        endIndex = Math.min(text.length, startIndex + 1);
      }
      const lineText = text.slice(startIndex, endIndex);
      return {
        text: lineText,
        width: Math.max(1, lineText.length) * 6,
        start,
        end: { segmentIndex: endIndex, graphemeIndex: 0 },
      };
    },
  ),
  layoutMock: vi.fn((prepared: { text: string }, maxWidth: number, lineHeight: number) => ({
    height: Math.max(
      lineHeight,
      Math.ceil(Math.max(1, prepared.text.length) / Math.max(1, Math.floor(maxWidth / 10))) * lineHeight,
    ),
    lineCount: 1,
  })),
}));

vi.mock("@chenglou/pretext", () => ({
  prepare: prepareMock,
  prepareWithSegments: prepareWithSegmentsMock,
  layoutNextLine: layoutNextLineMock,
  layout: layoutMock,
}));

import {
  clearPretextVirtualizerRowLayoutCache,
  getPretextVirtualizerRowLayout,
} from "./pretextVirtualizerRowLayout";
import {
  clearSessionMarkdownMeasurementCaches,
  measureSessionMarkdownDocument,
  measureSessionPlainTextBlockHeight,
} from "./sessionMarkdownMeasurement";
import {
  SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY,
  SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX,
  SESSION_THREAD_ASK_USER_MARGIN_VERTICAL_PX,
  SESSION_THREAD_ASK_USER_SHELL_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_BORDER_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_LINE_HEIGHT_PREMIUM_PX,
  SESSION_THREAD_TURN_HEADER_BUBBLE_BORDER_WIDTH_PX,
  SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_INLINE_PX,
  SESSION_THREAD_TURN_HEADER_COPY_GUTTER_PX,
  resolveSessionThreadContentWidth,
  resolveSessionThreadTurnHeaderTextWidth,
} from "./sessionThreadLayoutTokens";

describe("getPretextVirtualizerRowLayout", () => {
  beforeEach(() => {
    clearPretextVirtualizerRowLayoutCache();
    clearSessionMarkdownMeasurementCaches();
    prepareMock.mockClear();
    prepareWithSegmentsMock.mockClear();
    layoutNextLineMock.mockClear();
    layoutMock.mockClear();
  });

  it("reuses prepared text across width changes for thought rows", () => {
    const item: WorkbenchListItem = {
      kind: "thought",
      id: "thought-1",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      content: "thinking aloud",
    };

    const first = getPretextVirtualizerRowLayout(item, 640, {});
    const second = getPretextVirtualizerRowLayout(item, 720, {});

    expect(first.height).toBeGreaterThan(0);
    expect(second.height).toBeGreaterThan(0);
    expect(prepareMock).toHaveBeenCalledTimes(1);
    expect(layoutMock).toHaveBeenCalledTimes(2);
  });

  it("measures markdown-heavy assistant rows deterministically", () => {
    const item: WorkbenchListItem = {
      kind: "assistant",
      id: "assistant-1",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      content: "# Heading\n\n- bullet",
      thought: "",
      is_complete: true,
    };

    const result = getPretextVirtualizerRowLayout(item, 640, {});

    expect(result.height).toBeGreaterThan(40);
    expect(prepareMock.mock.calls.length + prepareWithSegmentsMock.mock.calls.length).toBeGreaterThan(0);
  });

  it("measures incomplete assistant markdown exactly like the completed row for the same content", () => {
    const partialItem: WorkbenchListItem = {
      kind: "assistant",
      id: "assistant-streaming-partial",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      content: "Before\n\n- partial item",
      thought: "",
      is_complete: false,
    };
    const closedItem: WorkbenchListItem = {
      ...partialItem,
      id: "assistant-streaming-closed",
      is_complete: true,
    };

    const partial = getPretextVirtualizerRowLayout(partialItem, 640, {});
    const closed = getPretextVirtualizerRowLayout(closedItem, 640, {});

    expect(partial.height).toBeGreaterThan(0);
    expect(closed.height).toBe(partial.height);
    expect(prepareMock).toHaveBeenCalled();
  });

  it("measures markdown tables with deterministic width-sensitive heights", () => {
    const item: WorkbenchListItem = {
      kind: "assistant",
      id: "assistant-table-1",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      content: [
        "| Fixture day | Active rows | Return rows |",
        "|---|---:|---:|",
        "| fixture-a | 12 | 3 |",
        "| fixture-b | 9 | `4` |",
      ].join("\n"),
      thought: "",
      is_complete: true,
    };

    const wide = getPretextVirtualizerRowLayout(item, 640, {});
    const narrow = getPretextVirtualizerRowLayout(item, 240, {});

    expect(wide.height).toBeGreaterThan(40);
    expect(narrow.height).toBeGreaterThan(wide.height);
    expect(prepareWithSegmentsMock).toHaveBeenCalled();
  });

  it("uses segmented monospace measurement for inline-code chips in assistant rows", () => {
    const item: WorkbenchListItem = {
      kind: "assistant",
      id: "assistant-inline-code",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      content: "prefix `1180` suffix",
      thought: "",
      is_complete: true,
    };

    const result = getPretextVirtualizerRowLayout(item, 180, {});

    expect(result.height).toBeGreaterThan(0);
    expect(prepareWithSegmentsMock).toHaveBeenCalled();
    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text, font]) => text === "1180" && typeof font === "string" && font.toLowerCase().includes("mono"),
      ),
    ).toBe(true);
  });

  it("splits long hyphenated inline-code tokens into deterministic wrap fragments", () => {
    const item: WorkbenchListItem = {
      kind: "assistant",
      id: "assistant-inline-code-wrap-fragments",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      content:
        "begin agent message with plain text and now `inline-thing-that-actually-gets-really-long-so-much-so-that-it-wraps-to-multiple-lines/core/apps/web/src/pages/sessionThread/sessionMarkdownMeasurement.ts` after the prose",
      thought: "",
      is_complete: true,
    };

    const result = getPretextVirtualizerRowLayout(item, 620, {});

    expect(result.height).toBeGreaterThan(0);
    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text, font]) =>
          text === "really-" && typeof font === "string" && font.toLowerCase().includes("mono"),
      ),
    ).toBe(true);
    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text, font]) =>
          text === "multiple-lines/core/" &&
          typeof font === "string" &&
          font.toLowerCase().includes("mono"),
      ),
    ).toBe(true);
  });

  it("splits pure path-like inline-code tokens at filename boundaries", () => {
    const item: WorkbenchListItem = {
      kind: "assistant",
      id: "assistant-inline-code-path-wrap-fragments",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      content:
        "prefix prose `sessionThreadDomMeasurement.tsx/sessionThreadDomMeasurement.tsx/apps/sessionThreadDomMeasurement.tsx/workbenchShell` suffix prose",
      thought: "",
      is_complete: true,
    };

    const result = getPretextVirtualizerRowLayout(item, 472, {});

    expect(result.height).toBeGreaterThan(0);
    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text, font]) =>
          text === "sessionThreadDomMeasurement." &&
          typeof font === "string" &&
          font.toLowerCase().includes("mono"),
      ),
    ).toBe(true);
    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text, font]) =>
          text === "tsx/sessionThreadDomMeasurement." &&
          typeof font === "string" &&
          font.toLowerCase().includes("mono"),
      ),
    ).toBe(true);
  });

  it("budgets the deterministic inline-code line-height premium for wrapped markdown lines", () => {
    const markdown = "`abcd` `efgh` `ijkl`";

    const height = measureSessionMarkdownDocument(markdown, 50);

    expect(height).toBe(
      Math.round(
        3 *
          (Math.max(
            SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
            SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX,
          ) +
            SESSION_THREAD_MARKDOWN_INLINE_CODE_LINE_HEIGHT_PREMIUM_PX) *
          16,
      ) / 16,
    );
  });

  it("treats markdown hard breaks as forced line breaks in wide paragraphs", () => {
    const markdown = [
      "Short version:  ",
      "the next system should answer not just which title wins.  ",
      "It should answer which article shape wins.",
    ].join("\n");

    const height = measureSessionMarkdownDocument(markdown, 1600);

    expect(height).toBe(Math.round(SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX * 3 * 16) / 16);
  });

  it("uses styled inline runs when bold markdown changes line breaking", () => {
    const markdown =
      "**Most predictive overall, but not usable directly pre-submit** These are still the strongest overall signal family.";

    const height = measureSessionMarkdownDocument(markdown, 340);

    expect(prepareWithSegmentsMock).toHaveBeenCalled();
    expect(height).toBeGreaterThan(SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX * 2);
  });

  it("measures strong inline markdown with the browser-matching heavier font weight", () => {
    measureSessionMarkdownDocument("prefix **strong fragment** suffix", 260);

    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text, font]) =>
          typeof text === "string" &&
          text.includes("strong") &&
          typeof font === "string" &&
          font.includes("700"),
      ),
    ).toBe(true);
  });

  it("does not reuse prepared mixed-inline segments across different markdown documents", () => {
    const first = measureSessionMarkdownDocument("`supercalifragilisticexpialidocious` tail", 120);
    const second = measureSessionMarkdownDocument("`x` tail", 120);

    expect(first).toBeGreaterThan(second);
    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text, font]) =>
          text === "supercalifragilisticexpialidocious" &&
          typeof font === "string" &&
          font.toLowerCase().includes("mono"),
      ),
    ).toBe(true);
    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text, font]) => text === "x" && typeof font === "string" && font.toLowerCase().includes("mono"),
      ),
    ).toBe(true);
  });

  it("keeps trailing prose on the continued inline-code line when the path leaves room", () => {
    const wrappedCode =
      "inline-thing-that-actually-gets-really-long-so-much-so-that-it-wraps-to-multiple-lines/core/apps/web/src/pages/sessionThread/sessionMarkdownMeasurement.ts";
    const withTrailingProse = measureSessionMarkdownDocument(
      `begin agent message with plain text and now \`${wrappedCode}\` after the prose`,
      620,
    );
    const withoutTrailingProse = measureSessionMarkdownDocument(
      `begin agent message with plain text and now \`${wrappedCode}\``,
      620,
    );

    expect(withTrailingProse).toBe(withoutTrailingProse);
  });

  it("keeps sealed inline-code fragments atomic when they exceed the available line width", () => {
    const height = measureSessionMarkdownDocument("`web/pretextVirtualizerRowLayout.ts`", 80);

    expect(height).toBe(
      Math.round(
        2 *
          (Math.max(
            SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
            SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX,
          ) +
            SESSION_THREAD_MARKDOWN_INLINE_CODE_LINE_HEIGHT_PREMIUM_PX) *
          16,
      ) / 16,
    );
  });

  it("treats soft newlines inside mixed inline paragraphs like collapsed spaces", () => {
    const withSoftNewline = measureSessionMarkdownDocument(
      "before mixed prose\nand `inline-code-token/with/path` after the wrap",
      260,
    );
    const withSpace = measureSessionMarkdownDocument(
      "before mixed prose and `inline-code-token/with/path` after the wrap",
      260,
    );

    expect(withSoftNewline).toBe(withSpace);
  });

  it("packs URL-heavy plain text lines as whitespace-separated turn-header tokens", () => {
    const line =
      "https://example.com/transcript/inline-code/transcript/inline-code?ref=293 pretextVirtualizerRowLayout.ts/fixtures/fixtures/core/workbenchShell/core/pages.";
    measureSessionPlainTextBlockHeight({
      cacheKey: "turn-header-plain-url",
      text: line,
      font: `${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`,
      width: 220,
      lineHeight: SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
    });

    expect(
      prepareMock.mock.calls.some(([text]) => text === line),
    ).toBe(false);
    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text, font]) =>
          text === "https://example.com/transcript/inline-code/transcript/inline-code?ref=293" &&
          typeof font === "string" &&
          !font.toLowerCase().includes("mono"),
      ),
    ).toBe(true);
    expect(
      prepareWithSegmentsMock.mock.calls.some(
        ([text]) => typeof text === "string" && text.startsWith("https://"),
      ),
    ).toBe(true);
  });

  it("wraps whitespace-separated turn-header tokens before breaking inside a long token", () => {
    const height = measureSessionPlainTextBlockHeight({
      cacheKey: "turn-header-word-wrap-before-break",
      text: "alpha betagamma",
      font: `${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`,
      width: 70,
      lineHeight: SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
    });

    expect(height).toBe(SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX * 2);
  });

  it("breaks overlong turn-header tokens at grapheme boundaries on a fresh line", () => {
    const height = measureSessionPlainTextBlockHeight({
      cacheKey: "turn-header-plain-command",
      text: "supercalifragilistic",
      font: `${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`,
      width: 60,
      lineHeight: SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
    });

    expect(height).toBe(SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX * 2);
    expect(
      prepareWithSegmentsMock.mock.calls.some(([text]) => text === "supercalif"),
    ).toBe(true);
  });

  it("continues overlong turn-header tokens on the current line when break-word is required", () => {
    const height = measureSessionPlainTextBlockHeight({
      cacheKey: "turn-header-break-word-continuation",
      text: "alpha supercalifragilistic",
      font: `${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`,
      width: 100,
      lineHeight: SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
    });

    expect(height).toBe(SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX * 2);
  });

  it("includes fenced code block border chrome in deterministic markdown height", () => {
    const markdown = "```ts\nconst value = 1;\n```";

    const height = measureSessionMarkdownDocument(markdown, 400);

    const expected =
      10 +
      SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX +
      SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX +
      SESSION_THREAD_MARKDOWN_CODE_BLOCK_BORDER_WIDTH_PX * 2 +
      SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX;
    expect(height).toBe(Math.round(expected * 16) / 16);
  });

  it("accounts for expansion state and attachments in turn header height", () => {
    const item: WorkbenchListItem = {
      kind: "turn_header",
      id: "header-item-1",
      header: {
        id: "header-1",
        created_at: "2026-04-05T00:00:00Z",
        content: "Header",
        plain_text: "Header\nwith several lines\nof content\nthat can collapse",
        attachments: [
          { kind: "image_ref", blob_id: "blob-1", mime_type: "image/png", name: "a.png" },
          { kind: "image_ref", blob_id: "blob-2", mime_type: "image/png", name: "b.png" },
        ],
      },
    };

    const collapsed = getPretextVirtualizerRowLayout(item, 640, {
      expandedTurnHeaders: { "header-1": false },
    });
    const expanded = getPretextVirtualizerRowLayout(item, 640, {
      expandedTurnHeaders: { "header-1": true },
    });

    expect(expanded.height).toBeGreaterThan(collapsed.height);
  });

  it("measures URL-heavy turn headers with the whitespace-token packer", () => {
    const item: WorkbenchListItem = {
      kind: "turn_header",
      id: "header-item-url",
      header: {
        id: "header-url",
        created_at: "2026-04-05T00:00:00Z",
        content:
          "- https://example.com/a/really/long/path/that/keeps/wrapping?token=12345 should not drift when wrapped inside the turn header bubble.",
        plain_text:
          "- https://example.com/a/really/long/path/that/keeps/wrapping?token=12345 should not drift when wrapped inside the turn header bubble.",
        attachments: [],
      },
    };

    const result = getPretextVirtualizerRowLayout(item, 620, {
      expandedTurnHeaders: { "header-url": true },
    });

    expect(result.height).toBeGreaterThan(0);
    expect(
      prepareMock.mock.calls.some(([text]) => text === item.header.plain_text),
    ).toBe(false);
    expect(
      prepareWithSegmentsMock.mock.calls.some(([text]) => text === "https://example.com/a/really/long/path/that/keeps/wrapping?token=12345"),
    ).toBe(true);
    expect(prepareWithSegmentsMock.mock.calls.some(([text]) => text === "https://")).toBe(false);
  });

  it("budgets turn-header text width using bubble border, padding, and copy gutter", () => {
    const viewportWidth = 620;

    expect(resolveSessionThreadTurnHeaderTextWidth(viewportWidth)).toBe(
      resolveSessionThreadContentWidth(viewportWidth) -
        SESSION_THREAD_TURN_HEADER_BUBBLE_BORDER_WIDTH_PX * 2 -
        SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_INLINE_PX * 2 -
        SESSION_THREAD_TURN_HEADER_COPY_GUTTER_PX,
    );
  });

  it("keeps tool groups compact until expanded", () => {
    const item: WorkbenchListItem = {
      kind: "tool_group",
      id: "tool-group-1",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      updated_at: "2026-04-05T00:00:30Z",
      tool_total: 2,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 2,
      tool_failed: 0,
      tools: [
        {
          kind: "tool",
          id: "tool-1",
          created_at: "2026-04-05T00:00:00Z",
          updated_at: "2026-04-05T00:00:10Z",
          tool_call_id: "call-1",
          tool_kind: "execute",
          title: "Run pwd",
          status: "completed",
          locations: [],
          input: { command: "pwd" },
          output_text: "",
          raw: {},
          updates_seen: 1,
        },
      ],
      thought: "I should inspect the repository first.",
    };

    const collapsed = getPretextVirtualizerRowLayout(item, 640, {});
    const expanded = getPretextVirtualizerRowLayout(item, 640, {
      expandedTurnDetailsById: { "turn-1": true },
    });

    expect(expanded.height).toBeGreaterThan(collapsed.height);
  });

  it("reserves deterministic space for ask-user-question cards", () => {
    const item: WorkbenchListItem = {
      kind: "ask_user_question",
      id: "ask-1",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      tool_call_id: "tool-call-1",
      answered: false,
      input: {
        questions: [
          {
            header: "Priority",
            question: "Which option should I choose?",
            options: [
              { label: "Fast", description: "Get it done quickly." },
              { label: "Careful", description: "Spend more time validating." },
            ],
            allowOther: true,
          },
        ],
      },
    };

    const result = getPretextVirtualizerRowLayout(item, 640, {});

    expect(result.height).toBe(
      Math.round((SESSION_THREAD_ASK_USER_MARGIN_VERTICAL_PX + SESSION_THREAD_ASK_USER_SHELL_HEIGHT_PX) * 16) / 16,
    );
  });

  it("keeps ask-user-question height fixed after the row is answered", () => {
    const pendingItem: WorkbenchListItem = {
      kind: "ask_user_question",
      id: "ask-2",
      turn_id: "turn-1",
      created_at: "2026-04-05T00:00:00Z",
      tool_call_id: "tool-call-2",
      answered: false,
      input: {
        questions: [
          {
            header: "Priority",
            question: "Which option should I choose?",
            options: [{ label: "Fast", description: "Get it done quickly." }],
            allowOther: true,
          },
        ],
      },
    };
    const answeredItem: WorkbenchListItem = {
      ...pendingItem,
      answered: true,
      answers: { "Which option should I choose?": "Fast" },
      outcome: "submitted",
    };

    const pending = getPretextVirtualizerRowLayout(pendingItem, 640, {});
    const answered = getPretextVirtualizerRowLayout(answeredItem, 640, {});

    expect(answered.height).toBe(pending.height);
  });
});
