import React, { type ReactNode } from "react";
import ReactDOMClient from "react-dom/client";
import { flushSync } from "react-dom";
import type { MessageAttachment } from "../../api/client";
import type { WorkbenchListItem, WorkbenchTurnHeader } from "../sessionView";
import { MemoMarkdown } from "../sessionView";
import {
  AssistantEntry,
  ThreadItemView,
  WorkbenchTurnHeaderView,
} from "./SessionThreadItemViews";
import {
  SESSION_THREAD_LAYOUT_STYLE,
  resolveSessionThreadContentWidth,
} from "./sessionThreadLayoutTokens";

const MEASUREMENT_CACHE_LIMIT = 4000;

type MeasurementSurfaceRecord = {
  container: HTMLDivElement;
  root: ReactDOMClient.Root;
};

let measurementSurface: MeasurementSurfaceRecord | null = null;

const markdownHeightCache = new Map<string, number>();
const rowHeightCache = new Map<string, number>();

const normalizeHeight = (value: number): number =>
  Number.isFinite(value) && value > 0 ? Math.max(1, Math.round(value * 16) / 16) : 0;

function pruneCache<T>(cache: Map<string, T>, limit: number): void {
  while (cache.size > limit) {
    const oldestKey = cache.keys().next().value;
    if (typeof oldestKey !== "string") break;
    cache.delete(oldestKey);
  }
}

function fingerprintString(value: string): string {
  const normalized = String(value ?? "");
  let hash = 2166136261;
  for (let index = 0; index < normalized.length; index += 1) {
    hash ^= normalized.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return `${normalized.length}:${(hash >>> 0).toString(36)}`;
}

function fingerprintAttachments(attachments: readonly MessageAttachment[] | undefined): string {
  return fingerprintString(
    JSON.stringify(
      (attachments ?? []).map((attachment) => ({
        kind: attachment.kind ?? "",
        name: attachment.name ?? "",
        mimeType: attachment.mime_type ?? "",
        blobId: "blob_id" in attachment ? attachment.blob_id ?? "" : "",
        dataLength: "data_base64" in attachment ? attachment.data_base64?.length ?? 0 : 0,
      })),
    ) ?? "",
  );
}

function ensureMeasurementSurface(): MeasurementSurfaceRecord | null {
  if (typeof document === "undefined" || !document.body) {
    return null;
  }
  if (measurementSurface) {
    return measurementSurface;
  }
  const container = document.createElement("div");
  container.setAttribute("data-session-thread-measurement-surface", "1");
  container.style.position = "fixed";
  container.style.left = "-10000px";
  container.style.top = "0";
  container.style.margin = "0";
  container.style.padding = "0";
  container.style.border = "0";
  container.style.visibility = "hidden";
  container.style.pointerEvents = "none";
  container.style.boxSizing = "border-box";
  document.body.appendChild(container);
  measurementSurface = {
    container,
    root: ReactDOMClient.createRoot(container),
  };
  return measurementSurface;
}

function renderIntoMeasurementSurface(params: {
  width: number;
  className?: string;
  content: ReactNode;
  measureSelector?: string;
}): number | null {
  const surface = ensureMeasurementSurface();
  if (!surface) {
    return null;
  }

  const wrapperStyle: React.CSSProperties = {
    width: `${Math.max(1, Math.round(params.width))}px`,
    margin: 0,
    padding: 0,
    border: 0,
    boxSizing: "border-box",
  };

  flushSync(() => {
    surface.root.render(
      React.createElement(
        "div",
        {
          className: params.className,
          style: wrapperStyle,
          ref: (element: HTMLDivElement | null) => {
            if (!element) return;
            for (const [key, value] of Object.entries(SESSION_THREAD_LAYOUT_STYLE)) {
              element.style.setProperty(key, String(value));
            }
          },
        },
        params.content,
      ),
    );
  });

  const target =
    (params.measureSelector
      ? surface.container.querySelector<HTMLElement>(params.measureSelector)
      : surface.container.firstElementChild) ?? null;
  const measuredHeight = target instanceof HTMLElement ? normalizeHeight(target.getBoundingClientRect().height) : 0;

  flushSync(() => {
    surface.root.render(null);
  });

  return measuredHeight > 0 ? measuredHeight : null;
}

function readCachedMeasurement(
  cache: Map<string, number>,
  cacheKey: string,
  measure: () => number | null,
): number | null {
  const cached = cache.get(cacheKey);
  if (cached != null) {
    return cached;
  }
  const measured = measure();
  if (measured == null) {
    return null;
  }
  cache.set(cacheKey, measured);
  pruneCache(cache, MEASUREMENT_CACHE_LIMIT);
  return measured;
}

export function clearSessionThreadDomMeasurementCaches(): void {
  markdownHeightCache.clear();
  rowHeightCache.clear();
  if (measurementSurface) {
    flushSync(() => {
      measurementSurface?.root.render(null);
    });
    measurementSurface.container.remove();
    measurementSurface = null;
  }
}

export function measureRenderedSessionMarkdownHeight(markdown: string, width: number): number | null {
  const normalizedWidth = Math.max(1, Math.round(width));
  const cacheKey = `markdown:${normalizedWidth}:${fingerprintString(markdown)}`;
  return readCachedMeasurement(markdownHeightCache, cacheKey, () =>
    renderIntoMeasurementSurface({
      width: normalizedWidth,
      className: "wb-assistant-body",
      content: React.createElement(MemoMarkdown, { content: markdown }),
      measureSelector: ".wb-markdown-root",
    }),
  );
}

export function measureRenderedSessionMessageHeight(
  item: Extract<WorkbenchListItem, { kind: "message" }>,
  viewportWidth: number,
  expanded: boolean,
): number | null {
  const normalizedViewportWidth = Math.max(1, Math.round(viewportWidth));
  const cacheKey = [
    "message",
    normalizedViewportWidth,
    expanded ? "expanded" : "collapsed",
    item.role,
    fingerprintString(item.content),
    fingerprintAttachments(item.attachments),
  ].join(":");
  return readCachedMeasurement(rowHeightCache, cacheKey, () =>
    renderIntoMeasurementSurface({
      width: resolveSessionThreadContentWidth(normalizedViewportWidth),
      content: React.createElement(
        "div",
        { "data-thread-item-id": item.id },
        React.createElement(
          "div",
          { className: "wb-thread-indent" },
          React.createElement(ThreadItemView, {
            item,
            worktreeId: null,
            onFileOpenError: () => {},
            messageExpanded: expanded,
            onToggleMessageExpanded: () => {},
          }),
        ),
      ),
      measureSelector: `[data-thread-item-id="${item.id}"]`,
    }),
  );
}

export function measureRenderedSessionAssistantHeight(
  item: Extract<WorkbenchListItem, { kind: "assistant" }>,
  viewportWidth: number,
): number | null {
  const normalizedViewportWidth = Math.max(1, Math.round(viewportWidth));
  const cacheKey = ["assistant", normalizedViewportWidth, fingerprintString(item.content)].join(":");
  return readCachedMeasurement(rowHeightCache, cacheKey, () =>
    renderIntoMeasurementSurface({
      width: resolveSessionThreadContentWidth(normalizedViewportWidth),
      content: React.createElement(
        "div",
        { "data-thread-item-id": item.id },
        React.createElement(
          "div",
          { className: "wb-thread-indent" },
          React.createElement(AssistantEntry, {
            content: item.content,
            worktreeId: null,
            onFileOpenError: () => {},
          }),
        ),
      ),
      measureSelector: `[data-thread-item-id="${item.id}"]`,
    }),
  );
}

export function measureRenderedSessionTurnHeaderHeight(
  header: WorkbenchTurnHeader,
  plainText: string,
  expanded: boolean,
  viewportWidth: number,
): number | null {
  const normalizedViewportWidth = Math.max(1, Math.round(viewportWidth));
  const cacheKey = [
    "turn-header",
    normalizedViewportWidth,
    expanded ? "expanded" : "collapsed",
    fingerprintString(plainText),
    fingerprintAttachments(header.attachments),
  ].join(":");
  return readCachedMeasurement(rowHeightCache, cacheKey, () =>
    renderIntoMeasurementSurface({
      width: resolveSessionThreadContentWidth(normalizedViewportWidth),
      content: React.createElement(WorkbenchTurnHeaderView, {
        header,
        plainText,
        expanded,
        onToggle: () => {},
      }),
      measureSelector: ".wb-turn-header",
    }),
  );
}
