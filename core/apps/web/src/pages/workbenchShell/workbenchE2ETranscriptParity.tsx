import React from "react";
import ReactDOMClient from "react-dom/client";
import { flushSync } from "react-dom";
import type { MessageAttachment } from "../../api/client";
import {
  AssistantEntry,
  ThreadItemView,
  WorkbenchTurnHeaderView,
} from "../sessionThread/SessionThreadItemViews";
import { getPretextVirtualizerRowLayout } from "../sessionThread/pretextVirtualizerRowLayout";
import {
  SESSION_THREAD_LAYOUT_STYLE,
  resolveSessionThreadContentWidth,
} from "../sessionThread/sessionThreadLayoutTokens";
import type { WorkbenchListItem, WorkbenchTurnHeader } from "../sessionView";

export type WorkbenchRowParityMeasurement = {
  planned: number;
  actual: number;
  delta: number;
};

export type WorkbenchMessageParityParams = {
  content: string;
  expanded: boolean;
  attachments?: MessageAttachment[];
  viewportWidth?: number;
};

export type WorkbenchAssistantParityParams = {
  content: string;
  isComplete?: boolean;
  viewportWidth?: number;
};

export type WorkbenchTurnHeaderParityParams = {
  plainText: string;
  viewportWidth?: number;
};

function applyTranscriptLayoutStyle(host: HTMLElement, viewportWidth: number): void {
  host.style.position = "fixed";
  host.style.left = "-10000px";
  host.style.top = "0";
  host.style.width = `${resolveSessionThreadContentWidth(viewportWidth)}px`;
  host.style.margin = "0";
  host.style.padding = "0";
  host.style.border = "0";
  host.style.boxSizing = "border-box";
  for (const [key, value] of Object.entries(SESSION_THREAD_LAYOUT_STYLE)) {
    host.style.setProperty(key, String(value));
  }
}

function makeParityMeasurement(planned: number, actual: number): WorkbenchRowParityMeasurement {
  return {
    planned,
    actual,
    delta: planned - actual,
  };
}

export async function measureWorkbenchMessageParity(
  params: WorkbenchMessageParityParams,
): Promise<WorkbenchRowParityMeasurement> {
  const viewportWidth = params.viewportWidth ?? 820;
  const host = document.createElement("div");
  applyTranscriptLayoutStyle(host, viewportWidth);
  document.body.appendChild(host);
  const root = ReactDOMClient.createRoot(host);
  const item: Extract<WorkbenchListItem, { kind: "message" }> = {
    kind: "message",
    id: "message-parity",
    role: "user",
    content: params.content,
    attachments: params.attachments ?? [],
    created_at: "2026-04-09T00:00:00Z",
  };

  try {
    flushSync(() => {
      root.render(
        React.createElement(
          "div",
          { "data-thread-item-id": item.id },
          React.createElement(
            "div",
            { className: "wb-thread-indent" },
            React.createElement(ThreadItemView, {
              item,
              worktreeId: null,
              onFileOpenError: () => {},
              messageExpanded: params.expanded,
              onToggleMessageExpanded: () => {},
            }),
          ),
        ),
      );
    });

    const actual = host.querySelector<HTMLElement>(`[data-thread-item-id="${item.id}"]`)?.getBoundingClientRect().height ?? 0;
    const planned = getPretextVirtualizerRowLayout(item, viewportWidth, {
      expandedMessageById: { [item.id]: params.expanded },
    }).height;
    return makeParityMeasurement(planned, actual);
  } finally {
    root.unmount();
    host.remove();
  }
}

export async function measureWorkbenchAssistantParity(
  params: WorkbenchAssistantParityParams,
): Promise<WorkbenchRowParityMeasurement> {
  const viewportWidth = params.viewportWidth ?? 820;
  const host = document.createElement("div");
  applyTranscriptLayoutStyle(host, viewportWidth);
  document.body.appendChild(host);
  const root = ReactDOMClient.createRoot(host);
  const item: Extract<WorkbenchListItem, { kind: "assistant" }> = {
    kind: "assistant",
    id: "assistant-parity",
    turn_id: "turn-1",
    created_at: "2026-04-09T00:00:00Z",
    content: params.content,
    thought: "",
    is_complete: params.isComplete ?? true,
  };

  try {
    flushSync(() => {
      root.render(
        React.createElement(
          "div",
          { "data-thread-item-id": item.id },
          React.createElement(
            "div",
            { className: "wb-thread-indent" },
            React.createElement(AssistantEntry, {
              content: item.content,
              isComplete: item.is_complete,
              worktreeId: null,
              onFileOpenError: () => {},
            }),
          ),
        ),
      );
    });

    const actual = host.querySelector<HTMLElement>(`[data-thread-item-id="${item.id}"]`)?.getBoundingClientRect().height ?? 0;
    const planned = getPretextVirtualizerRowLayout(item, viewportWidth, {}).height;
    return makeParityMeasurement(planned, actual);
  } finally {
    root.unmount();
    host.remove();
  }
}

export async function measureWorkbenchTurnHeaderParity(
  params: WorkbenchTurnHeaderParityParams,
): Promise<WorkbenchRowParityMeasurement> {
  const viewportWidth = params.viewportWidth ?? 820;
  const host = document.createElement("div");
  applyTranscriptLayoutStyle(host, viewportWidth);
  document.body.appendChild(host);
  const root = ReactDOMClient.createRoot(host);
  const header: WorkbenchTurnHeader = {
    id: "turn-header-parity",
    content: params.plainText,
    plain_text: params.plainText,
    attachments: [],
    created_at: "2026-04-10T00:00:00Z",
  };
  const item: Extract<WorkbenchListItem, { kind: "turn_header" }> = {
    kind: "turn_header",
    id: "turn-header-parity-row",
    header,
  };

  try {
    flushSync(() => {
      root.render(
        React.createElement(
          "div",
          { style: { display: "contents" } },
          React.createElement(WorkbenchTurnHeaderView, {
            header,
            plainText: params.plainText,
            expanded: true,
            onToggle: () => {},
          }),
        ),
      );
    });

    const actual = host.querySelector<HTMLElement>(".wb-turn-header")?.getBoundingClientRect().height ?? 0;
    const planned = getPretextVirtualizerRowLayout(item, viewportWidth, {
      expandedTurnHeaders: { [header.id]: true },
    }).height;
    return makeParityMeasurement(planned, actual);
  } finally {
    root.unmount();
    host.remove();
  }
}
