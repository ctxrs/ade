import React from "react";
import { act, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageAttachment } from "../api/client";
import { WorkbenchSessionSlot } from "./WorkbenchPage.sessionSlot";

const sessionViewSpy = vi.hoisted(() => vi.fn());
const draftState = vi.hoisted(() => ({
  value: {
    text: "draft text",
    modeId: "default",
    attachments: [] as MessageAttachment[],
  },
}));
const setValueSpy = vi.hoisted(() =>
  vi.fn(
    (
      next:
        | { text: string; modeId: string; attachments?: MessageAttachment[] }
        | ((prev: { text: string; modeId: string; attachments: MessageAttachment[] }) => {
            text: string;
            modeId: string;
            attachments?: MessageAttachment[];
          }),
    ) => {
      const current = draftState.value;
      const resolved = typeof next === "function" ? next(current) : next;
      draftState.value = {
        text: resolved.text,
        modeId: resolved.modeId,
        attachments: resolved.attachments ?? current.attachments,
      };
    },
  ),
);
const flushDraftSpy = vi.hoisted(() => vi.fn(async () => {}));

const initialAttachment: MessageAttachment = {
  kind: "image_ref",
  blob_id: "blob-1",
  mime_type: "image/png",
  name: "blob-1.png",
};

vi.mock("./SessionPage", () => ({
  SessionView: (props: unknown) => {
    sessionViewSpy(props);
    return <div data-testid="session-view" />;
  },
}));

vi.mock("../workbench/store", () => ({
  sessionDraftKey: (sessionId: string) => `session:${sessionId}`,
  useWorkbenchDraft: () => ({
    value: draftState.value,
    updatedAtMs: 0,
    setValue: (
      next:
        | { text: string; modeId: string; attachments?: MessageAttachment[] }
        | ((current: { text: string; modeId: string; attachments: MessageAttachment[] }) => {
            text: string;
            modeId: string;
            attachments?: MessageAttachment[];
          }),
    ) => {
      const resolved = typeof next === "function" ? next(draftState.value) : next;
      draftState.value = {
        text: resolved.text,
        modeId: resolved.modeId,
        attachments: resolved.attachments ?? draftState.value.attachments,
      };
      setValueSpy(resolved);
    },
  }),
  useWorkbenchStore: () => ({
    flushDraft: flushDraftSpy,
  }),
}));

vi.mock("../utils/clipboard", () => ({
  copyTextToClipboard: vi.fn(async () => true),
}));

describe("WorkbenchSessionSlot", () => {
  beforeEach(() => {
    draftState.value = {
      text: "draft text",
      modeId: "default",
      attachments: [initialAttachment],
    };
    sessionViewSpy.mockClear();
    setValueSpy.mockClear();
    flushDraftSpy.mockClear();
    draftState.value = { text: "draft text", modeId: "default", attachments: [initialAttachment] };
  });

  it("passes attachment drafts through to SessionView and persists attachment edits", async () => {
    render(<WorkbenchSessionSlot sessionId="session-1" />);

    const props = sessionViewSpy.mock.calls.at(-1)?.[0] as
      | {
          draft: { text: string; modeId: string; attachments: MessageAttachment[] };
          onDraftAttachmentsChange: (attachments: MessageAttachment[]) => void;
        }
      | undefined;

    expect(props?.draft.attachments).toEqual([initialAttachment]);

    const nextAttachment: MessageAttachment = {
      kind: "image_ref",
      blob_id: "blob-2",
      mime_type: "image/png",
      name: "blob-2.png",
    };

    await act(async () => {
      props?.onDraftAttachmentsChange([nextAttachment]);
    });

    expect(draftState.value).toEqual({
      text: "draft text",
      modeId: "default",
      attachments: [nextAttachment],
    });
  });

  it("composes sequential draft updates without restoring stale text", async () => {
    render(<WorkbenchSessionSlot sessionId="session-1" />);

    const props = sessionViewSpy.mock.calls.at(-1)?.[0] as
      | {
          onDraftChange: (text: string) => void;
          onDraftAttachmentsChange: (attachments: MessageAttachment[]) => void;
        }
      | undefined;

    await act(async () => {
      props?.onDraftChange("");
      props?.onDraftAttachmentsChange([]);
    });

    expect(draftState.value).toEqual({
      text: "",
      modeId: "default",
      attachments: [],
    });
  });
});
