import { memo, useCallback, useEffect, useRef, type CSSProperties, type MutableRefObject, type ReactNode } from "react";
import {
  VirtuosoMessageList,
  VirtuosoMessageListLicense,
  type DataWithScrollModifier,
  type ItemLocation,
  type ItemContent as MessageItemContent,
  type ListScrollLocation,
  type ShortSizeAlign,
  type VirtuosoMessageListMethods,
} from "@virtuoso.dev/message-list";
import type { WorkbenchListItem } from "./SessionPage.types";
import {
  WorkbenchMessageListEmptyPlaceholder,
  WorkbenchMessageListStickyFooter,
} from "./sessionThread/SessionThreadMessageListChrome";

type WorkbenchMessageListStackProps = {
  virtuosoStyle: CSSProperties;
  initialData: WorkbenchListItem[];
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  itemIdentity: (item: WorkbenchListItem) => unknown;
  initialLocation?: ItemLocation;
  dataState?: DataWithScrollModifier<WorkbenchListItem>;
  context: WorkbenchMessageListContext;
  onScroll: (location: ListScrollLocation) => void;
  onRenderedDataChange: (range: WorkbenchListItem[]) => void;
  listRef: MutableRefObject<VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>;
  licenseKey: string;
  shortSizeAlign: ShortSizeAlign;
};

export type WorkbenchMessageListContext = {
  loaded: boolean;
  loadingOlder: boolean;
  renderRevision?: string;
};

const DEBUG_ROW_SIZE_DELTA_PX = 8;

function MeasuredThreadRow({
  id,
  children,
}: {
  id: string;
  children: ReactNode;
}) {
  const rowRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let debugEnabled = false;
    try {
      debugEnabled = new URLSearchParams(window.location.search).get("debug") === "1";
    } catch {
      debugEnabled = false;
    }
    if (!debugEnabled) return;

    const rowEl = rowRef.current;
    if (!rowEl) return;
    const parentEl = rowEl.parentElement as HTMLElement | null;
    if (!parentEl) return;

    let lastSignature = "";
    const emitMismatch = (reason: string) => {
      const knownSizeRaw = parentEl.getAttribute("data-known-size");
      const actualHeight = rowEl.getBoundingClientRect().height;
      const parentHeight = parentEl.getBoundingClientRect().height;
      const knownSize = knownSizeRaw == null ? Number.NaN : Number(knownSizeRaw);
      if (!Number.isFinite(knownSize)) return;
      if (Math.abs(actualHeight - knownSize) <= DEBUG_ROW_SIZE_DELTA_PX) return;
      const signature = `${reason}:${knownSize}:${Math.round(actualHeight)}:${Math.round(parentHeight)}`;
      if (signature === lastSignature) return;
      lastSignature = signature;
      // eslint-disable-next-line no-console
      console.log("[MessageList][row-size-mismatch]", {
        id,
        reason,
        dataIndex: parentEl.getAttribute("data-index"),
        knownSize,
        actualHeight,
        parentHeight,
      });
    };

    emitMismatch("mount");
    const observer = new ResizeObserver(() => emitMismatch("resize"));
    observer.observe(rowEl);
    observer.observe(parentEl);
    const rafId = requestAnimationFrame(() => emitMismatch("raf"));
    return () => {
      cancelAnimationFrame(rafId);
      observer.disconnect();
    };
  }, [id]);

  return (
    <div ref={rowRef} role="listitem" data-thread-item-id={id}>
      {children}
    </div>
  );
}

export const WorkbenchMessageListStack = memo(function WorkbenchMessageListStack({
  virtuosoStyle,
  initialData,
  itemContent,
  itemIdentity,
  initialLocation,
  dataState,
  context,
  onScroll,
  onRenderedDataChange,
  listRef,
  licenseKey,
  shortSizeAlign,
}: WorkbenchMessageListStackProps) {
  // Keep ItemContent component identity stable so React doesn't remount visible rows
  // (which clears text selection / hover state) when SessionView rerenders.
  const itemContentRef = useRef(itemContent);
  itemContentRef.current = itemContent;
  const ItemContent = useCallback<MessageItemContent<WorkbenchListItem, WorkbenchMessageListContext>>(
    ({ index, data }) => {
      if (!data) return <div style={{ height: 1 }} />;
      return (
        <MeasuredThreadRow id={data.id}>
          {itemContentRef.current(index, data)}
        </MeasuredThreadRow>
      );
    },
    [],
  );

  return (
    <div className="thread-stack wb-thread-stack wb-thread-scroller--message-list">
      <VirtuosoMessageListLicense licenseKey={licenseKey}>
        <VirtuosoMessageList<WorkbenchListItem, WorkbenchMessageListContext>
          ref={listRef}
          style={virtuosoStyle}
          className="wb-thread-scroller"
          role="list"
          initialData={initialData}
          data={dataState}
          context={context}
          itemIdentity={itemIdentity}
          computeItemKey={({ data }) => data.id}
          ItemContent={ItemContent}
          initialLocation={initialLocation}
          onScroll={onScroll}
          onRenderedDataChange={onRenderedDataChange}
          EmptyPlaceholder={WorkbenchMessageListEmptyPlaceholder}
          StickyFooter={WorkbenchMessageListStickyFooter}
          shortSizeAlign={shortSizeAlign}
        />
      </VirtuosoMessageListLicense>
    </div>
  );
});
