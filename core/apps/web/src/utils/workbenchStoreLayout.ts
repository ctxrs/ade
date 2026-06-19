import { randomUuid } from "./randomUuid";
import type {
  LayoutNode,
  PersistedWorkbenchTemplateV1,
  PersistedWorkbenchWindowV1,
  SplitDirection,
  WorkbenchTab,
} from "../workbench/types";
import {
  decodePersistedWorkbenchTemplateV1,
  decodePersistedWorkbenchWindowV1,
  workbenchDaemonKey,
} from "../workbench/persistence";

const WINDOW_ID_STORAGE_KEY = "contextUiWindowId.v1";
const WINDOW_SESSION_STORAGE_PREFIX = "wb.window.session.v1";
const TEMPLATE_SESSION_STORAGE_PREFIX = "wb.template.session.v1";

function safeSessionKeyPart(value: string): string {
  return encodeURIComponent(value);
}

function sessionWindowKeyV1(workspaceId: string, windowId: string): string {
  return `${WINDOW_SESSION_STORAGE_PREFIX}.${safeSessionKeyPart(workbenchDaemonKey())}.${safeSessionKeyPart(workspaceId)}.${safeSessionKeyPart(windowId)}`;
}

function sessionTemplateKeyV1(workspaceId: string, windowId: string): string {
  return `${TEMPLATE_SESSION_STORAGE_PREFIX}.${safeSessionKeyPart(workbenchDaemonKey())}.${safeSessionKeyPart(workspaceId)}.${safeSessionKeyPart(windowId)}`;
}

export function readSessionWindowV1(workspaceId: string, windowId: string): PersistedWorkbenchWindowV1 | null {
  try {
    const raw = sessionStorage.getItem(sessionWindowKeyV1(workspaceId, windowId));
    if (!raw) return null;
    return decodePersistedWorkbenchWindowV1(JSON.parse(raw));
  } catch {
    return null;
  }
}

export function readSessionTemplateV1(workspaceId: string, windowId: string): PersistedWorkbenchTemplateV1 | null {
  try {
    const raw = sessionStorage.getItem(sessionTemplateKeyV1(workspaceId, windowId));
    if (!raw) return null;
    return decodePersistedWorkbenchTemplateV1(JSON.parse(raw));
  } catch {
    return null;
  }
}

export function writeSessionWindowV1(workspaceId: string, windowId: string, win: PersistedWorkbenchWindowV1) {
  try {
    sessionStorage.setItem(sessionWindowKeyV1(workspaceId, windowId), JSON.stringify(win));
  } catch {
    // ignore
  }
}

export function writeSessionTemplateV1(
  workspaceId: string,
  windowId: string,
  template: PersistedWorkbenchTemplateV1,
) {
  try {
    sessionStorage.setItem(sessionTemplateKeyV1(workspaceId, windowId), JSON.stringify(template));
  } catch {
    // ignore
  }
}

export function getOrCreateWindowId(): string {
  const windowNamePrefix = "ctx-ui-window-id:";
  const readWindowName = (): string | null => {
    try {
      if (typeof window === "undefined") return null;
      const name = String(window.name ?? "");
      if (!name.startsWith(windowNamePrefix)) return null;
      const id = name.slice(windowNamePrefix.length).trim();
      return id || null;
    } catch {
      return null;
    }
  };
  const writeWindowName = (id: string) => {
    try {
      if (typeof window === "undefined") return;
      const name = String(window.name ?? "");
      if (name && !name.startsWith(windowNamePrefix)) return;
      window.name = `${windowNamePrefix}${id}`;
    } catch {
      // ignore
    }
  };
  try {
    const existing = sessionStorage.getItem(WINDOW_ID_STORAGE_KEY);
    if (existing && existing.trim()) return existing;
    const fromName = readWindowName();
    if (fromName) {
      sessionStorage.setItem(WINDOW_ID_STORAGE_KEY, fromName);
      return fromName;
    }
    const created = randomUuid();
    sessionStorage.setItem(WINDOW_ID_STORAGE_KEY, created);
    writeWindowName(created);
    return created;
  } catch {
    const fromName = readWindowName();
    if (fromName) return fromName;
    const created = randomUuid();
    writeWindowName(created);
    return created;
  }
}

export function defaultWindowState(): PersistedWorkbenchWindowV1 {
  const leafId = randomUuid();
  const tabId = randomUuid();
  return {
    v: 1,
    layout: {
      kind: "leaf",
      id: leafId,
      tabs: [{ id: tabId, kind: "new_task" }],
      activeTabId: tabId,
    },
    focusedLeafId: leafId,
  };
}

export function findLeaf(node: LayoutNode, leafId: string): Extract<LayoutNode, { kind: "leaf" }> | null {
  if (node.kind === "leaf") return node.id === leafId ? node : null;
  return findLeaf(node.first, leafId) ?? findLeaf(node.second, leafId);
}

export function updateLeaf(
  node: LayoutNode,
  leafId: string,
  fn: (leaf: Extract<LayoutNode, { kind: "leaf" }>) => LayoutNode,
): LayoutNode {
  if (node.kind === "leaf") {
    return node.id === leafId ? fn(node) : node;
  }
  return { ...node, first: updateLeaf(node.first, leafId, fn), second: updateLeaf(node.second, leafId, fn) };
}

export function ensureLeafActiveTab(leaf: Extract<LayoutNode, { kind: "leaf" }>): Extract<LayoutNode, { kind: "leaf" }> {
  const activeOk = leaf.tabs.some((t) => t.id === leaf.activeTabId);
  if (activeOk) return leaf;
  return { ...leaf, activeTabId: leaf.tabs[0]?.id ?? leaf.activeTabId };
}

export function getActiveTabFromLeaf(leaf: Extract<LayoutNode, { kind: "leaf" }>): WorkbenchTab | null {
  const found = leaf.tabs.find((t) => t.id === leaf.activeTabId);
  return found ?? leaf.tabs[0] ?? null;
}

export function clampSplitRatio(ratio: number, min = 0.1, max = 0.9): number {
  if (!Number.isFinite(ratio)) return 0.5;
  return Math.min(max, Math.max(min, ratio));
}

export function firstLeafId(node: LayoutNode): string | null {
  if (node.kind === "leaf") return node.id;
  return firstLeafId(node.first) ?? firstLeafId(node.second);
}

export function normalizeFocusedLeaf(layout: LayoutNode, focusedLeafId: string | null | undefined): string {
  if (focusedLeafId && findLeaf(layout, focusedLeafId)) return focusedLeafId;
  return firstLeafId(layout) ?? "";
}

export function focusLeaf(win: PersistedWorkbenchWindowV1, leafId: string): PersistedWorkbenchWindowV1 {
  const nextFocusedLeafId = normalizeFocusedLeaf(win.layout, leafId);
  if (nextFocusedLeafId === win.focusedLeafId) return win;
  return { ...win, focusedLeafId: nextFocusedLeafId };
}

export function resizeSplitRatio(node: LayoutNode, splitId: string, ratio: number): LayoutNode {
  if (node.kind === "leaf") return node;
  const first = resizeSplitRatio(node.first, splitId, ratio);
  const second = resizeSplitRatio(node.second, splitId, ratio);
  const nextRatio = node.id === splitId ? clampSplitRatio(ratio) : node.ratio;
  if (first === node.first && second === node.second && nextRatio === node.ratio) {
    return node;
  }
  return {
    ...node,
    ratio: nextRatio,
    first,
    second,
  };
}

export function splitFocusedLeaf(
  win: PersistedWorkbenchWindowV1,
  direction: SplitDirection,
  opts?: {
    ratio?: number;
    splitId?: string;
    leafId?: string;
    tabId?: string;
    focus?: "first" | "second";
  },
): PersistedWorkbenchWindowV1 {
  const focusedLeafId = normalizeFocusedLeaf(win.layout, win.focusedLeafId);
  const leaf = findLeaf(win.layout, focusedLeafId);
  if (!leaf) return { ...win, focusedLeafId };

  const tabId = opts?.tabId ?? randomUuid();
  const nextLeafId = opts?.leafId ?? randomUuid();
  const nextLeaf: Extract<LayoutNode, { kind: "leaf" }> = {
    kind: "leaf",
    id: nextLeafId,
    tabs: [{ id: tabId, kind: "new_task" }],
    activeTabId: tabId,
  };
  const split: LayoutNode = {
    kind: "split",
    id: opts?.splitId ?? randomUuid(),
    direction,
    ratio: clampSplitRatio(opts?.ratio ?? 0.5),
    first: leaf,
    second: nextLeaf,
  };

  const layout = updateLeaf(win.layout, focusedLeafId, () => split);
  return { ...win, layout, focusedLeafId: opts?.focus === "first" ? focusedLeafId : nextLeafId };
}
