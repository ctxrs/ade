import { memo, type CSSProperties, type MutableRefObject, type ReactNode } from "react";
import type {
  DataWithScrollModifier,
  ItemLocation,
  ListScrollLocation,
  ShortSizeAlign,
  VirtuosoMessageListMethods,
} from "@virtuoso.dev/message-list";
import { WorkbenchMessageListStack, type WorkbenchMessageListContext } from "./SessionPage.thread";
import type { WorkbenchListItem } from "./SessionPage.types";

export const SessionThreadMessageList = memo(function SessionThreadMessageList({
  sessionId,
  style,
  initialData,
  itemContent,
  itemIdentity,
  initialLocation,
  dataState,
  context,
  onScroll,
  onRenderedDataChange,
  methodsRef,
  licenseKey,
  shortSizeAlign,
}: {
  sessionId: string;
  style: CSSProperties;
  initialData: WorkbenchListItem[];
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  itemIdentity: (item: WorkbenchListItem) => unknown;
  initialLocation: ItemLocation;
  dataState?: DataWithScrollModifier<WorkbenchListItem>;
  context: WorkbenchMessageListContext;
  onScroll: (location: ListScrollLocation) => void;
  onRenderedDataChange: (range: WorkbenchListItem[]) => void;
  methodsRef: MutableRefObject<VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>;
  licenseKey: string;
  shortSizeAlign: ShortSizeAlign;
}) {
  return (
    <WorkbenchMessageListStack
      key={sessionId}
      virtuosoStyle={style}
      initialData={initialData}
      itemContent={itemContent}
      itemIdentity={itemIdentity}
      initialLocation={initialLocation}
      dataState={dataState}
      context={context}
      onScroll={onScroll}
      onRenderedDataChange={onRenderedDataChange}
      listRef={methodsRef}
      licenseKey={licenseKey}
      shortSizeAlign={shortSizeAlign}
    />
  );
});
