import { useCallback, useEffect, useRef, useState, type Dispatch, type RefObject, type SetStateAction } from "react";
import type { MessageAttachment } from "../../api/client";
import { imageFilesToInlineAttachments } from "../../utils/messageAttachments";
import { registerDropScope } from "../../utils/dragDropScopes";

type UseWorkbenchDragDropAttachmentsArgs = {
  scopeRef: RefObject<HTMLElement | null>;
  activeTaskId: string | null;
  setDraftAttachments: Dispatch<SetStateAction<MessageAttachment[]>>;
};

const extractFilesFromTransfer = (transfer: DataTransfer | null): File[] => {
  if (!transfer) return [];
  const out: File[] = [];
  const files = transfer.files ? Array.from(transfer.files) : [];
  out.push(...files);
  const items = transfer.items;
  if (out.length === 0 && items && items.length > 0) {
    for (const item of Array.from(items)) {
      if (item.kind !== "file") continue;
      const file = item.getAsFile?.();
      if (file) out.push(file);
    }
  }
  return out;
};

const extractFirstUrlFromTransfer = (transfer: DataTransfer | null): string | null => {
  if (!transfer) return null;
  const uriRaw = (transfer.getData?.("text/uri-list") ?? "").trim();
  if (uriRaw) {
    for (const line of uriRaw.split("\n")) {
      const value = line.trim();
      if (!value || value.startsWith("#")) continue;
      return value;
    }
  }
  const html = (transfer.getData?.("text/html") ?? "").trim();
  if (html) {
    const match = html.match(/<img[^>]*\ssrc=("([^"]+)"|'([^']+)'|([^\s>]+))/i);
    const src = (match?.[2] ?? match?.[3] ?? match?.[4] ?? "").trim();
    if (src) return src;
  }
  const text = (transfer.getData?.("text/plain") ?? "").trim();
  if (text && /^(https?:|data:image\/|blob:)/i.test(text)) return text;
  return null;
};

const urlToImageFile = async (url: string): Promise<File | null> => {
  try {
    const response = await fetch(url);
    if (!response.ok) return null;
    const blob = await response.blob();
    const type = blob.type || "";
    if (!type.startsWith("image/")) return null;

    const baseName = (() => {
      try {
        const parsed = new URL(url, window.location.href);
        const last = parsed.pathname.split("/").filter(Boolean).pop() || "image";
        return last.replace(/[?#].*$/, "") || "image";
      } catch {
        return "image";
      }
    })();

    const ext = type.split("/")[1] || "";
    const name = ext && !baseName.toLowerCase().endsWith(`.${ext.toLowerCase()}`) ? `${baseName}.${ext}` : baseName;
    return new File([blob], name, { type });
  } catch {
    return null;
  }
};

export function useWorkbenchDragDropAttachments({
  scopeRef,
  activeTaskId,
  setDraftAttachments,
}: UseWorkbenchDragDropAttachmentsArgs) {
  const [dropActive, setDropActive] = useState(false);
  const dropHideTimerRef = useRef<number | null>(null);

  const onDropFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;
      const next = await imageFilesToInlineAttachments(files);
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
    const element = scopeRef.current;
    if (!element) return;

    return registerDropScope({
      element,
      onDragOver: () => showDropOverlay(),
      onDrop: (transfer) => {
        hideDropOverlay();
        void (async () => {
          const files = extractFilesFromTransfer(transfer);
          if (files.length > 0) {
            await onDropFiles(files);
            return;
          }
          const url = extractFirstUrlFromTransfer(transfer);
          if (!url) return;
          const asFile = await urlToImageFile(url);
          if (!asFile) return;
          await onDropFiles([asFile]);
        })();
      },
    });
  }, [activeTaskId, hideDropOverlay, onDropFiles, scopeRef, showDropOverlay]);

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
