import type { CSSProperties, MutableRefObject, ReactNode } from "react";
import type {
  ItemLocation,
  ListScrollLocation,
  ShortSizeAlign,
  VirtuosoMessageListMethods,
} from "@virtuoso.dev/message-list";
import type { WorkbenchMessageListContext } from "../SessionPage.thread";
import type { WorkbenchListItem } from "../SessionPage.types";
import { SessionThreadMessageList } from "../SessionThreadMessageList";

export function SessionThreadPane({
  style,
  itemContent,
  itemIdentity,
  initialLocation,
  context,
  onScroll,
  onRenderedDataChange,
  methodsRef,
  licenseKey,
  shortSizeAlign,
  children,
}: {
  style: CSSProperties;
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  itemIdentity: (item: WorkbenchListItem) => unknown;
  initialLocation: ItemLocation;
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
        style={style}
        itemContent={itemContent}
        itemIdentity={itemIdentity}
        initialLocation={initialLocation}
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

