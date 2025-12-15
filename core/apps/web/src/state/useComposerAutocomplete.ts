import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type React from "react";
import {
  applyComposerAutocompleteCompletion,
  detectComposerAutocompleteToken,
  type ComposerAutocompleteToken,
} from "../utils/composerAutocomplete";
import { getTextareaCaretRect } from "../utils/textareaCaret";
import type { ComposerAutocompleteItem } from "../components/ComposerAutocompleteMenu";

export type SlashCommandDescriptor = {
  name: string;
  description?: string;
};

export function useComposerAutocomplete({
  sessionId,
  value,
  setValue,
  textareaRef,
  slashCommands,
}: {
  sessionId: string | null;
  value: string;
  setValue: (next: string) => void;
  textareaRef: { current: HTMLTextAreaElement | null };
  slashCommands: SlashCommandDescriptor[];
}) {
  const [token, setToken] = useState<ComposerAutocompleteToken | null>(null);
  const [anchorRect, setAnchorRect] = useState<DOMRect | null>(null);
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);

  const [fileItems, setFileItems] = useState<ComposerAutocompleteItem[]>([]);
  const [loadingFiles, setLoadingFiles] = useState(false);

  const dismissedRef = useRef<{ start: number; end: number; text: string } | null>(null);
  const debounceTimerRef = useRef<number | null>(null);

  const syncFromDom = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;

    const cursor = el.selectionStart ?? value.length;
    const next = detectComposerAutocompleteToken(value, cursor);
    if (
      next &&
      dismissedRef.current &&
      dismissedRef.current.start === next.start &&
      dismissedRef.current.end === next.end &&
      dismissedRef.current.text === value.slice(next.start, next.end)
    ) {
      setToken(null);
      setOpen(false);
      return;
    }

    setToken(next);
    setOpen(!!next);
    if (next) {
      setAnchorRect(getTextareaCaretRect(el));
    } else {
      setAnchorRect(null);
    }
  }, [textareaRef, value]);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    requestAnimationFrame(() => syncFromDom());
  }, [value, textareaRef, syncFromDom]);

  const slashItems = useMemo((): ComposerAutocompleteItem[] => {
    if (!token || token.kind !== "slash") return [];
    const q = token.query.trim().toLowerCase();
    const filtered = slashCommands.filter((c) => {
      if (!q) return true;
      const name = c.name.toLowerCase();
      const full = `/${name}`;
      return name.startsWith(q) || name.includes(q) || full.includes(q);
    });
    return filtered.map((c) => ({
      key: `slash:${c.name}`,
      label: `/${c.name}`,
      insertText: `/${c.name}`,
      description: c.description,
      kind: "slash",
    }));
  }, [slashCommands, token]);

  useEffect(() => {
    // File completion is optional and not wired up in this UI yet.
    // Keep the hook functional for slash completion without requiring daemon support.
    if (!token || token.kind !== "at") {
      setFileItems([]);
      setLoadingFiles(false);
      if (debounceTimerRef.current) {
        window.clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }
      return;
    }
    setFileItems([]);
    setLoadingFiles(false);
  }, [token]);

  const items = token?.kind === "slash" ? slashItems : token?.kind === "at" ? fileItems : [];
  const loading = token?.kind === "at" ? loadingFiles : false;

  useEffect(() => {
    setActiveIndex(0);
  }, [token?.kind, token?.query]);

  const dismiss = useCallback(() => {
    if (token) {
      dismissedRef.current = {
        start: token.start,
        end: token.end,
        text: value.slice(token.start, token.end),
      };
    }
    setToken(null);
    setOpen(false);
  }, [token, value]);

  const pick = useCallback(
    (index: number) => {
      if (!token) return;
      const it = items[index];
      if (!it) return;
      const out = applyComposerAutocompleteCompletion(value, token, it.insertText);
      setValue(out.nextText);
      requestAnimationFrame(() => {
        const el = textareaRef.current;
        if (!el) return;
        el.focus();
        el.setSelectionRange(out.nextCursor, out.nextCursor);
      });
      dismissedRef.current = null;
      setToken(null);
      setOpen(false);
    },
    [items, setValue, textareaRef, token, value],
  );

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>): boolean => {
      if (!open) return false;
      if (e.key === "Escape") {
        e.preventDefault();
        dismiss();
        return true;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIndex((prev) => (items.length === 0 ? 0 : (prev + 1) % items.length));
        return true;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIndex((prev) => (items.length === 0 ? 0 : (prev - 1 + items.length) % items.length));
        return true;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        if (e.key === "Enter" && (e.shiftKey || e.metaKey || e.ctrlKey || e.altKey)) {
          return false;
        }
        if (items.length > 0) {
          e.preventDefault();
          pick(activeIndex);
          return true;
        }
      }
      return false;
    },
    [activeIndex, dismiss, items.length, open, pick],
  );

  const inlineFallback = !anchorRect;

  return {
    open,
    loading,
    items,
    activeIndex,
    anchorRect,
    inlineFallback,
    setActiveIndex,
    pick,
    dismiss,
    onKeyDown,
    syncFromDom,
  };
}
