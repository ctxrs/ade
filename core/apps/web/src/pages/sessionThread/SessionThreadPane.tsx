import type { CSSProperties, MutableRefObject, ReactNode } from "react";
import type {
  DataWithScrollModifier,
  ItemLocation,
  ListScrollLocation,
  ShortSizeAlign,
  VirtuosoMessageListMethods,
} from "@virtuoso.dev/message-list";
import type { WorkbenchMessageListContext } from "../SessionPage.thread";
import type { WorkbenchListItem } from "../SessionPage.types";
import { SessionThreadMessageList } from "../SessionThreadMessageList";

export function SessionThreadPane({
  sessionId,
  style,
  initialData,
  itemContent,
  itemIdentity,
  itemKey,
  increaseViewportBy,
  initialLocation,
  dataState,
  context,
  onScroll,
  onRenderedDataChange,
  methodsRef,
  licenseKey,
  shortSizeAlign,
  children,
}: {
  sessionId: string;
  style: CSSProperties;
  initialData: WorkbenchListItem[];
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  itemIdentity: (item: WorkbenchListItem) => unknown;
  itemKey: (item: WorkbenchListItem) => string;
  increaseViewportBy: number;
  initialLocation: ItemLocation;
  dataState?: DataWithScrollModifier<WorkbenchListItem>;
  context: WorkbenchMessageListContext;
  onScroll: (location: ListScrollLocation) => void;
  onRenderedDataChange: (range: WorkbenchListItem[]) => void;
  methodsRef: MutableRefObject<VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>;
  licenseKey: string;
  shortSizeAlign: ShortSizeAlign;
  children: ReactNode;
}) {
  return (
    <>
      <SessionThreadMessageList
        sessionId={sessionId}
        style={style}
        initialData={initialData}
        itemContent={itemContent}
        itemIdentity={itemIdentity}
        itemKey={itemKey}
        increaseViewportBy={increaseViewportBy}
        initialLocation={initialLocation}
        dataState={dataState}
        context={context}
        onScroll={onScroll}
        onRenderedDataChange={onRenderedDataChange}
        methodsRef={methodsRef}
        licenseKey={licenseKey}
        shortSizeAlign={shortSizeAlign}
      />
      <div className="wb-session-bottom">{children}</div>
    </>
  );
}
