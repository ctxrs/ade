import React from "react";
import { act, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageAttachment } from "../api/client";
import { WorkbenchSessionSlot } from "./WorkbenchPage.sessionSlot";

const sessionViewSpy = vi.hoisted(() => vi.fn());
const setValueSpy = vi.hoisted(() => vi.fn());
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
    value: { text: "draft text", modeId: "default", attachments: [initialAttachment] },
    updatedAtMs: 0,
    setValue: setValueSpy,
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
    sessionViewSpy.mockClear();
    setValueSpy.mockClear();
    flushDraftSpy.mockClear();
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

    expect(setValueSpy).toHaveBeenCalledWith({
      text: "draft text",
      modeId: "default",
      attachments: [nextAttachment],
    });
  });
});
