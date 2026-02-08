import { memo, useCallback, type CSSProperties, type MutableRefObject, type ReactNode } from "react";
import {
  VirtuosoMessageList,
  VirtuosoMessageListLicense,
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
