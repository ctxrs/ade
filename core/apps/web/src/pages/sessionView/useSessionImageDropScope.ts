import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { MessageAttachment } from "../../api/client";
import { registerDropScope } from "../../utils/dragDropScopes";
import { imageAttachmentsFromPaths, imageAttachmentsFromTransfer } from "../../utils/droppedImageAttachments";

type UseSessionImageDropScopeArgs = {
  setDraftAttachments: Dispatch<SetStateAction<MessageAttachment[]>>;
};

export function useSessionImageDropScope({
  setDraftAttachments,
}: UseSessionImageDropScopeArgs) {
  const dropScopeRef = useRef<HTMLDivElement | null>(null);
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
    if (dropHideTimerRef.current) {
      window.clearTimeout(dropHideTimerRef.current);
    }
    dropHideTimerRef.current = window.setTimeout(() => setDropActive(false), 140);
  }, []);

  const hideDropOverlay = useCallback(() => {
    if (dropHideTimerRef.current) {
      window.clearTimeout(dropHideTimerRef.current);
    }
    dropHideTimerRef.current = null;
    setDropActive(false);
  }, []);

  useEffect(() => {
    const element = dropScopeRef.current;
    if (!element) return;
    return registerDropScope({
      element,
      onDragOver: () => showDropOverlay(),
      onDragLeave: () => hideDropOverlay(),
      onDrop: (dt) => {
        hideDropOverlay();
        void (async () => {
          appendAttachments(await imageAttachmentsFromTransfer(dt));
        })();
      },
      onDropPaths: (paths) => {
        hideDropOverlay();
        void (async () => {
          appendAttachments(await imageAttachmentsFromPaths(paths));
        })();
      },
    });
  }, [appendAttachments, hideDropOverlay, showDropOverlay]);

  useEffect(() => {
    return () => {
      if (dropHideTimerRef.current) {
        window.clearTimeout(dropHideTimerRef.current);
      }
    };
  }, []);

  return {
    dropScopeRef,
    dropActive,
    hideDropOverlay,
  };
}
