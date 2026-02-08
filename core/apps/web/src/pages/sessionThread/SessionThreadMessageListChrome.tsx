import { type VirtuosoMessageListProps, useVirtuosoLocation, useVirtuosoMethods } from "@virtuoso.dev/message-list";
import type { WorkbenchListItem } from "../SessionPage.types";
import type { WorkbenchMessageListContext } from "../SessionPage.thread";

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

