import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { MessageAttachment } from "../../api/client";
import { imageFilesToInlineAttachments } from "../../utils/messageAttachments";
import { registerDropScope } from "../../utils/dragDropScopes";

type UseSessionImageDropScopeArgs = {
  setDraftAttachments: Dispatch<SetStateAction<MessageAttachment[]>>;
};

export function useSessionImageDropScope({
  setDraftAttachments,
}: UseSessionImageDropScopeArgs) {
  const dropScopeRef = useRef<HTMLDivElement | null>(null);
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

  const extractFilesFromTransfer = useCallback((dt: DataTransfer | null): File[] => {
    if (!dt) return [];
    const out: File[] = [];
    const files = dt.files ? Array.from(dt.files) : [];
    out.push(...files);
    const items = dt.items;
    if (out.length === 0 && items && items.length > 0) {
      for (const item of Array.from(items)) {
        if (item.kind !== "file") continue;
        const file = item.getAsFile?.();
        if (file) out.push(file);
      }
    }
    return out;
  }, []);

  const extractFirstUrlFromTransfer = useCallback((dt: DataTransfer | null): string | null => {
    if (!dt) return null;
    const uriRaw = (dt.getData?.("text/uri-list") ?? "").trim();
    if (uriRaw) {
      for (const line of uriRaw.split("\n")) {
        const value = line.trim();
        if (!value || value.startsWith("#")) continue;
        return value;
      }
    }

    const html = (dt.getData?.("text/html") ?? "").trim();
    if (html) {
      const match = html.match(/<img[^>]*\ssrc=("([^"]+)"|'([^']+)'|([^\s>]+))/i);
      const src = (match?.[2] ?? match?.[3] ?? match?.[4] ?? "").trim();
      if (src) return src;
    }

    const text = (dt.getData?.("text/plain") ?? "").trim();
    if (text && /^(https?:|data:image\/|blob:)/i.test(text)) return text;
    return null;
  }, []);

  const urlToImageFile = useCallback(async (url: string): Promise<File | null> => {
    try {
      const response = await fetch(url);
      if (!response.ok) return null;
      const blob = await response.blob();
      const type = blob.type || "";
      if (!type.startsWith("image/")) return null;

      const baseName = (() => {
        try {
          const resolvedUrl = new URL(url, window.location.href);
          const last = resolvedUrl.pathname.split("/").filter(Boolean).pop() || "image";
          return last.replace(/[?#].*$/, "") || "image";
        } catch {
          return "image";
        }
      })();

      const extension = type.split("/")[1] || "";
      const name =
        extension && !baseName.toLowerCase().endsWith(`.${extension.toLowerCase()}`)
          ? `${baseName}.${extension}`
          : baseName;

      return new File([blob], name, { type });
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    const element = dropScopeRef.current;
    if (!element) return;
    return registerDropScope({
      element,
      onDragOver: () => showDropOverlay(),
      onDrop: (dt) => {
        hideDropOverlay();
        void (async () => {
          const files = extractFilesFromTransfer(dt);
          if (files.length > 0) {
            await onDropFiles(files);
            return;
          }
          const url = extractFirstUrlFromTransfer(dt);
          if (!url) return;
          const file = await urlToImageFile(url);
          if (!file) return;
          await onDropFiles([file]);
        })();
      },
    });
  }, [
    extractFilesFromTransfer,
    extractFirstUrlFromTransfer,
    hideDropOverlay,
    onDropFiles,
    showDropOverlay,
    urlToImageFile,
  ]);

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
