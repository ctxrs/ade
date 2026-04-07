import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkbenchListItem } from "../SessionPage.types";

const { prepareMock, prepareWithSegmentsMock, layoutMock } = vi.hoisted(() => ({
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
  layout: layoutMock,
}));

import {
  clearPretextVirtualizerRowLayoutCache,
  getPretextVirtualizerRowLayout,
} from "./pretextVirtualizerRowLayout";
import { measureSessionMarkdownDocument } from "./sessionMarkdownMeasurement";
import { SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_HEIGHT_PX } from "./sessionThreadLayoutTokens";

describe("getPretextVirtualizerRowLayout", () => {
  beforeEach(() => {
    clearPretextVirtualizerRowLayoutCache();
    prepareMock.mockClear();
    prepareWithSegmentsMock.mockClear();
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

  it("budgets the full inline-code chip chrome for wrapped markdown lines", () => {
    const markdown = "`abcd` `efgh` `ijkl`";

    const height = measureSessionMarkdownDocument(markdown, 50);

    expect(height).toBe(Math.ceil(3 * (20.15 + SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_HEIGHT_PX)));
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

    expect(result.height).toBeGreaterThan(180);
  });
});
