import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { MessageAttachment } from "../../api/client";
import { registerDropScope } from "../../utils/dragDropScopes";
import { imageAttachmentsFromPaths, imageAttachmentsFromTransfer } from "../../utils/droppedImageAttachments";

type UseWorkbenchDragDropAttachmentsArgs = {
  scopeElement: HTMLElement | null;
  activeTaskId: string | null;
  setDraftAttachments: Dispatch<SetStateAction<MessageAttachment[]>>;
};

export function useWorkbenchDragDropAttachments({
  scopeElement,
  activeTaskId,
  setDraftAttachments,
}: UseWorkbenchDragDropAttachmentsArgs) {
  const [dropActive, setDropActive] = useState(false);
  const dropHideTimerRef = useRef<number | null>(null);

  const appendAttachments = useCallback(
    (next: MessageAttachment[]) => {
      if (next.length === 0) return;
      setDraftAttachments((prev) => [...prev, ...next]);
    },
    [setDraftAttachments],
  );

  const showDropOverlay = useCallback(() => {
    setDropActive(true);
    if (dropHideTimerRef.current) window.clearTimeout(dropHideTimerRef.current);
    dropHideTimerRef.current = window.setTimeout(() => setDropActive(false), 140);
  }, []);

  const hideDropOverlay = useCallback(() => {
    if (dropHideTimerRef.current) window.clearTimeout(dropHideTimerRef.current);
    dropHideTimerRef.current = null;
    setDropActive(false);
  }, []);

  useEffect(() => {
    if (!scopeElement) return;

    return registerDropScope({
      element: scopeElement,
      onDragOver: () => showDropOverlay(),
      onDragLeave: () => hideDropOverlay(),
      onDrop: (transfer) => {
        hideDropOverlay();
        void (async () => {
          appendAttachments(await imageAttachmentsFromTransfer(transfer));
        })();
      },
      onDropPaths: (paths) => {
        hideDropOverlay();
        void (async () => {
          appendAttachments(await imageAttachmentsFromPaths(paths));
        })();
      },
    });
  }, [activeTaskId, appendAttachments, hideDropOverlay, scopeElement, showDropOverlay]);

  useEffect(
    () => () => {
      if (dropHideTimerRef.current) {
        window.clearTimeout(dropHideTimerRef.current);
        dropHideTimerRef.current = null;
      }
    },
    [],
  );

  return {
    dropActive,
  };
}
