import { useEffect, useRef, type ReactNode } from "react";
import {
  createPretextVirtualizerCore,
  type PretextVirtualizerLogicalAnchor,
  type PretextVirtualizerSnapshot,
} from "@pretext-virtualizer/core";
import type {
  PretextVirtualizerItemAlign,
  PretextVirtualizerItemLocation,
} from "@pretext-virtualizer/interface";
import type { WorkbenchListItem } from "../sessionView";
import { recordSessionMessageListRowSizeMismatch } from "../sessionMessageListDebug";
import type { WorkbenchThreadProjectionOp } from "../sessionThreadProjection";

const DEBUG_ROW_SIZE_DELTA_PX = 1;

export function AuditedPretextRow({
  id,
  itemKind,
  itemKey,
  plannedHeight,
  children,
}: {
  id: string;
  itemKind: WorkbenchListItem["kind"];
  itemKey: string;
  plannedHeight: number;
  children: ReactNode;
}) {
  const rowRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let debugEnabled = false;
    try {
      debugEnabled = new URLSearchParams(window.location.search).get("debug") === "1";
    } catch {
      debugEnabled = false;
    }
    if (!debugEnabled) return;

    const rowEl = rowRef.current;
    if (!rowEl) return;
    const shellEl = rowEl.closest("[data-pretext-virtualizer-row-shell='1']") as HTMLElement | null;
    if (!shellEl) return;

    let lastSignature = "";
    const emitMismatch = (reason: string) => {
      const actualHeight = rowEl.getBoundingClientRect().height;
      const shellHeight = shellEl.getBoundingClientRect().height;
      const plannedVsActualDeltaPx = actualHeight - plannedHeight;
      const plannedVsShellDeltaPx = shellHeight - plannedHeight;
      const shellVsActualDeltaPx = actualHeight - shellHeight;
      if (Math.abs(plannedVsActualDeltaPx) <= DEBUG_ROW_SIZE_DELTA_PX) return;
      const signature = `${reason}:${plannedHeight}:${Math.round(actualHeight)}:${Math.round(shellHeight)}`;
      if (signature === lastSignature) return;
      lastSignature = signature;
      recordSessionMessageListRowSizeMismatch({
        id,
        itemKind,
        itemKey,
        reason,
        dataIndex: null,
        knownSize: plannedHeight,
        actualHeight,
        parentHeight: shellHeight,
        knownVsActualDeltaPx: plannedVsActualDeltaPx,
        knownVsParentDeltaPx: plannedVsShellDeltaPx,
        parentVsActualDeltaPx: shellVsActualDeltaPx,
      });
      // eslint-disable-next-line no-console
      console.log("[PretextVirtualizer][row-size-mismatch]", {
        id,
        itemKind,
        itemKey,
        reason,
        plannedHeight,
        actualHeight,
        shellHeight,
        plannedVsActualDeltaPx,
        plannedVsShellDeltaPx,
        shellVsActualDeltaPx,
      });
    };

    emitMismatch("mount");
    const observer = new ResizeObserver(() => emitMismatch("resize"));
    observer.observe(rowEl);
    observer.observe(shellEl);
    const rafId = requestAnimationFrame(() => emitMismatch("raf"));
    return () => {
      cancelAnimationFrame(rafId);
      observer.disconnect();
    };
  }, [id, itemKey, itemKind, plannedHeight]);

  return (
    <div ref={rowRef} role="listitem" data-thread-item-id={id}>
      {children}
    </div>
  );
}

export function resolveScrollTopForLocation(
  snapshot: PretextVirtualizerSnapshot<WorkbenchListItem>,
  location: PretextVirtualizerItemLocation,
  core: ReturnType<typeof createPretextVirtualizerCore<WorkbenchListItem>>,
  itemCount: number,
): number {
  if (snapshot.visibleItems.length === 0 && location.index === "LAST") {
    return Math.max(0, snapshot.totalHeight - snapshot.viewportHeight);
  }
  const totalHeight = snapshot.totalHeight;
  const viewportHeight = snapshot.viewportHeight;
  const maxScrollTop = Math.max(0, totalHeight - viewportHeight);
  const rawIndex = location.index === "LAST" ? Math.max(0, itemCount - 1) : location.index;
  const targetIndex = Math.max(0, Math.min(rawIndex, Math.max(0, itemCount - 1)));
  const top = core.getOffsetForIndex(targetIndex);
  const height = core.getHeightForIndex(targetIndex);
  const align: PretextVirtualizerItemAlign = location.align ?? "start";
  if (align === "end") {
    return Math.max(0, Math.min(maxScrollTop, top + height - viewportHeight));
  }
  if (align === "center") {
    return Math.max(0, Math.min(maxScrollTop, top + height / 2 - viewportHeight / 2));
  }
  return Math.max(0, Math.min(maxScrollTop, top));
}

export function approximateIndexForLocation(
  items: readonly WorkbenchListItem[],
  location: PretextVirtualizerItemLocation,
): number {
  if (items.length === 0) return 0;
  if (location.index === "LAST") return items.length - 1;
  return Math.max(0, Math.min(location.index, items.length - 1));
}

export function haveSameItemRefs(
  current: readonly WorkbenchListItem[],
  next: readonly WorkbenchListItem[],
): boolean {
  if (current === next) return true;
  if (current.length !== next.length) return false;
  for (let index = 0; index < current.length; index += 1) {
    if (current[index] !== next[index]) return false;
  }
  return true;
}

export function haveSameLayoutInputs(
  current: readonly WorkbenchListItem[],
  next: readonly WorkbenchListItem[],
  getLayoutRevision: (item: WorkbenchListItem) => string | number,
): boolean {
  if (current === next) return true;
  if (current.length !== next.length) return false;
  for (let index = 0; index < current.length; index += 1) {
    const currentItem = current[index];
    const nextItem = next[index];
    if (!currentItem || !nextItem) return false;
    if (currentItem.id !== nextItem.id) return false;
    if (getLayoutRevision(currentItem) !== getLayoutRevision(nextItem)) return false;
  }
  return true;
}

export function haveSameItemIds(
  current: readonly WorkbenchListItem[],
  next: readonly WorkbenchListItem[],
): boolean {
  if (current === next) return true;
  if (current.length !== next.length) return false;
  for (let index = 0; index < current.length; index += 1) {
    if (current[index]?.id !== next[index]?.id) return false;
  }
  return true;
}

export function isLocalizedProjectionOp(kind: WorkbenchThreadProjectionOp["kind"]): boolean {
  return kind === "hydrate_tools" || kind === "terminalize_turn" || kind === "toggle_expansion";
}

export function createVisibleItemAnchor(
  visibleItem: PretextVirtualizerSnapshot<WorkbenchListItem>["visibleItems"][number],
  scrollTop: number,
): PretextVirtualizerLogicalAnchor {
  const maxOffsetPx = Math.max(0, visibleItem.height - 1);
  const offsetPx = Math.max(0, Math.min(scrollTop - visibleItem.top, maxOffsetPx));
  return {
    kind: "item",
    id: visibleItem.id,
    index: visibleItem.index,
    offsetPx,
    offsetRatio: visibleItem.height > 0 ? offsetPx / visibleItem.height : 0,
  };
}

export function resolveLocalizedAnchorOverride(
  currentSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>,
  projectionOp: WorkbenchThreadProjectionOp,
  activeChangedItemId: string | null,
  fallback: PretextVirtualizerLogicalAnchor,
): PretextVirtualizerLogicalAnchor {
  if (
    projectionOp.changedItemIds.length === 0 ||
    projectionOp.kind === "replace_session" ||
    activeChangedItemId == null
  ) {
    return fallback;
  }
  const viewportTop = currentSnapshot.scrollTop;
  const viewportBottom = viewportTop + currentSnapshot.viewportHeight;
  const visibleChangedItem = currentSnapshot.visibleItems.find((visibleItem) => {
    if (visibleItem.id !== activeChangedItemId) return false;
    const itemBottom = visibleItem.top + visibleItem.height;
    return itemBottom > viewportTop && visibleItem.top < viewportBottom;
  });
  if (!visibleChangedItem) {
    return fallback;
  }
  return createVisibleItemAnchor(visibleChangedItem, currentSnapshot.scrollTop);
}
