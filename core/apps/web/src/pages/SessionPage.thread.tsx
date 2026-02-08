import { memo, useCallback, type CSSProperties, type MutableRefObject, type ReactNode } from "react";
import {
  VirtuosoMessageList,
  VirtuosoMessageListLicense,
  type ItemLocation,
  type ItemContent as MessageItemContent,
  type ListScrollLocation,
  type ShortSizeAlign,
  type VirtuosoMessageListMethods,
  type VirtuosoMessageListProps,
  useVirtuosoLocation,
  useVirtuosoMethods,
} from "@virtuoso.dev/message-list";
import type { WorkbenchListItem } from "./SessionPage.types";

type WorkbenchMessageListStackProps = {
  virtuosoStyle: CSSProperties;
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  itemIdentity: (item: WorkbenchListItem) => unknown;
  initialLocation?: ItemLocation;
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
};

type WorkbenchMessageListProps = VirtuosoMessageListProps<WorkbenchListItem, WorkbenchMessageListContext>;

export const WorkbenchMessageListEmptyPlaceholder: WorkbenchMessageListProps["EmptyPlaceholder"] = ({ context }) => {
  // Avoid visible "Loading..." placeholders; the thread should feel continuous.
  return context.loaded ? <div className="wb-muted">Empty</div> : null;
};

export const WorkbenchMessageListHeader: WorkbenchMessageListProps["Header"] = ({ context }) => {
  // No visible loading header; preserve layout by rendering nothing.
  // (Adding/removing header height can also introduce scroll jitter.)
  void context;
  return null;
};

export const WorkbenchMessageListStickyFooter: WorkbenchMessageListProps["StickyFooter"] = () => {
  const location = useVirtuosoLocation();
  const methods = useVirtuosoMethods<WorkbenchListItem, WorkbenchMessageListContext>();
  return (
    <div style={{ position: "relative", width: "100%", height: 0 }}>
      {location.bottomOffset > 200 ? (
        <button
          type="button"
          className="new-activity-overlay"
          aria-label="Jump to latest"
          title="Jump to latest"
          style={{ position: "absolute", right: 16, bottom: 16 }}
          onClick={() => methods.scrollToItem({ index: "LAST" as const, align: "end", behavior: "auto" })}
        >
          ↓
        </button>
      ) : null}
    </div>
  );
};

export const WorkbenchMessageListStack = memo(function WorkbenchMessageListStack({
  virtuosoStyle,
  itemContent,
  itemIdentity,
  initialLocation,
  context,
  onScroll,
  onRenderedDataChange,
  listRef,
  licenseKey,
  shortSizeAlign,
}: WorkbenchMessageListStackProps) {
  const ItemContent = useCallback<MessageItemContent<WorkbenchListItem, WorkbenchMessageListContext>>(
    ({ index, data }) => {
      if (!data) return <div style={{ height: 1 }} />;
      return (
        <div role="listitem" data-thread-item-id={data.id}>
          {itemContent(index, data)}
        </div>
      );
    },
    [itemContent],
  );

  return (
    <div className="thread-stack wb-thread-stack wb-thread-scroller--message-list">
      <VirtuosoMessageListLicense licenseKey={licenseKey}>
        <VirtuosoMessageList<WorkbenchListItem, WorkbenchMessageListContext>
          ref={listRef}
          style={virtuosoStyle}
          role="list"
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
