import { memo, type CSSProperties, type MutableRefObject, type ReactNode } from "react";
import type { ItemLocation, ListScrollLocation, VirtuosoMessageListMethods } from "@virtuoso.dev/message-list";
import { WorkbenchMessageListStack, type WorkbenchMessageListContext } from "./SessionPage.thread";
import type { WorkbenchListItem } from "./SessionPage.types";

export const SessionThreadMessageList = memo(function SessionThreadMessageList({
  style,
  itemContent,
  itemIdentity,
  initialLocation,
  context,
  onScroll,
  onRenderedDataChange,
  increaseViewportBy,
  methodsRef,
  licenseKey,
}: {
  style: CSSProperties;
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  itemIdentity: (item: WorkbenchListItem) => unknown;
  initialLocation: ItemLocation;
  context: WorkbenchMessageListContext;
  onScroll: (location: ListScrollLocation) => void;
  onRenderedDataChange: (range: WorkbenchListItem[]) => void;
  increaseViewportBy: number;
  methodsRef: MutableRefObject<VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>;
  licenseKey: string;
}) {
  return (
    <WorkbenchMessageListStack
      virtuosoStyle={style}
      itemContent={itemContent}
      itemIdentity={itemIdentity}
      initialLocation={initialLocation}
      context={context}
      onScroll={onScroll}
      onRenderedDataChange={onRenderedDataChange}
      increaseViewportBy={increaseViewportBy}
      listRef={methodsRef}
      licenseKey={licenseKey}
    />
  );
});
