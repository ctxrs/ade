import React, { createContext, useCallback, useContext, useEffect, useMemo, useSyncExternalStore } from "react";
import { randomUuid } from "../utils/randomUuid";
import type { WorkbenchModeId } from "../components/WorkbenchComposer";
import type { LayoutNode, PersistedWorkbenchWindowV1, WorkbenchDraft, WorkbenchScrollState, WorkbenchTab } from "./types";
import {
  loadWorkbenchDraftV1,
  loadWorkbenchWindowV1,
  migrateLegacySelectionToWindowV1,
  saveWorkbenchDraftV1,
  saveWorkbenchWindowV1,
  workbenchDaemonKey,
} from "./persistence";

const WINDOW_ID_STORAGE_KEY = "contextUiWindowId.v1";
const SCROLL_CACHE_LIMIT = 2;
export const NEW_TASK_DRAFT_KEY = "new_task";

export function sessionDraftKey(sessionId: string): string {
  return `session:${sessionId}`;
}

export function scrollKey(sessionId: string): string {
  return `session:${sessionId}`;
}

function getOrCreateWindowId(): string {
  try {
    const existing = sessionStorage.getItem(WINDOW_ID_STORAGE_KEY);
    if (existing && existing.trim()) return existing;
    const created = randomUuid();
    sessionStorage.setItem(WINDOW_ID_STORAGE_KEY, created);
    return created;
  } catch {
    return randomUuid();
  }
}

function defaultWindowState(): PersistedWorkbenchWindowV1 {
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
    scrollByKey: {},
  };
}

function findLeaf(node: LayoutNode, leafId: string): Extract<LayoutNode, { kind: "leaf" }> | null {
  if (node.kind === "leaf") return node.id === leafId ? node : null;
  return findLeaf(node.first, leafId) ?? findLeaf(node.second, leafId);
}

function mapLayout(node: LayoutNode, fn: (n: LayoutNode) => LayoutNode): LayoutNode {
  const next = fn(node);
  if (next.kind === "split") {
    return { ...next, first: mapLayout(next.first, fn), second: mapLayout(next.second, fn) };
  }
  return next;
}

function updateLeaf(node: LayoutNode, leafId: string, fn: (leaf: Extract<LayoutNode, { kind: "leaf" }>) => LayoutNode): LayoutNode {
  return mapLayout(node, (n) => {
    if (n.kind !== "leaf") return n;
    if (n.id !== leafId) return n;
    return fn(n);
  });
}

function ensureLeafActiveTab(leaf: Extract<LayoutNode, { kind: "leaf" }>): Extract<LayoutNode, { kind: "leaf" }> {
  const activeOk = leaf.tabs.some((t) => t.id === leaf.activeTabId);
  if (activeOk) return leaf;
  return { ...leaf, activeTabId: leaf.tabs[0]?.id ?? leaf.activeTabId };
}

function getActiveTabFromLeaf(leaf: Extract<LayoutNode, { kind: "leaf" }>): WorkbenchTab | null {
  const found = leaf.tabs.find((t) => t.id === leaf.activeTabId);
  return found ?? leaf.tabs[0] ?? null;
}

type DraftSnapshot = {
  byKey: Record<string, WorkbenchDraft | undefined>;
  loadedKeys: Record<string, boolean | undefined>;
};

export type WorkbenchNavToken = number;
export type WorkbenchNavSource = "system" | "user";
export type WorkbenchNavOpts = {
  navToken?: WorkbenchNavToken;
  source?: WorkbenchNavSource;
};

export type WorkbenchStoreSnapshot = {
  workspaceId: string;
  windowId: string;
  hydrated: boolean;
  persistEnabled: boolean;
  warnings: string[];
  window: PersistedWorkbenchWindowV1;
  drafts: DraftSnapshot;
};

export type WorkbenchShellSnapshot = {
  workspaceId: string;
  windowId: string;
  hydrated: boolean;
  warnings: string[];
  window: PersistedWorkbenchWindowV1;
};

type WorkbenchStoreListener = () => void;

type DraftBroadcastMsg =
  | {
    type: "draft";
    workspaceId: string;
    windowId: string;
    draftKey: string;
    draft: WorkbenchDraft;
  }
  | {
    type: "draft_delete";
    workspaceId: string;
    windowId: string;
    draftKey: string;
    updatedAtMs: number;
  };

export class WorkbenchStore {
  private listeners = new Set<WorkbenchStoreListener>();
  private snapshot: WorkbenchStoreSnapshot;
  private persistEnabled = true;
  private persistTimer: number | null = null;
  private draftTimers = new Map<string, number>();
  private draftLoadsInFlight = new Map<string, Promise<void>>();
  private channel: BroadcastChannel | null = null;
  private layoutDirtyBeforeHydrate = false;
  private shellSnapshotCache: {
    window: PersistedWorkbenchWindowV1;
    warnings: string[];
    hydrated: boolean;
    value: WorkbenchShellSnapshot;
  } | null = null;
  private navEpoch = 0;

  constructor(workspaceId: string) {
    const windowId = getOrCreateWindowId();
    this.snapshot = {
      workspaceId,
      windowId,
      hydrated: false,
      persistEnabled: true,
      warnings: [],
      window: defaultWindowState(),
      drafts: { byKey: {}, loadedKeys: {} },
    };
  }

  subscribe = (listener: WorkbenchStoreListener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): WorkbenchStoreSnapshot => this.snapshot;

  getShellSnapshot = (): WorkbenchShellSnapshot => {
    const { workspaceId, windowId, hydrated, warnings, window } = this.snapshot;
    const cache = this.shellSnapshotCache;
    if (cache && cache.window === window && cache.warnings === warnings && cache.hydrated === hydrated) {
      return cache.value;
    }
    const value: WorkbenchShellSnapshot = { workspaceId, windowId, hydrated, warnings, window };
    this.shellSnapshotCache = { window, warnings, hydrated, value };
    return value;
  };

  init = () => {
    this.hydrate().catch(() => { });
    this.initBroadcast();
    this.ensureDraftLoaded(NEW_TASK_DRAFT_KEY).catch(() => { });
  };

  private publish() {
    for (const l of this.listeners) l();
  }

  private markLayoutDirty() {
    if (!this.snapshot.hydrated) {
      this.layoutDirtyBeforeHydrate = true;
    }
  }

  private pruneScrollByKey(
    scrollByKey: Record<string, WorkbenchScrollState | undefined>,
  ): Record<string, WorkbenchScrollState | undefined> {
    const entries = Object.entries(scrollByKey)
      .map(([key, state]) => (state && !state.stickToBottom ? ([key, state] as const) : null))
      .filter((entry): entry is [string, WorkbenchScrollState] => Boolean(entry));
    if (entries.length === 0) return {};
    entries.sort((a, b) => (b[1].updatedAtMs ?? 0) - (a[1].updatedAtMs ?? 0));
    const next: Record<string, WorkbenchScrollState | undefined> = {};
    for (const [key, state] of entries.slice(0, SCROLL_CACHE_LIMIT)) {
      next[key] = state;
    }
    return next;
  }

  private scrollByKeyEquals(
    a: Record<string, WorkbenchScrollState | undefined>,
    b: Record<string, WorkbenchScrollState | undefined>,
  ): boolean {
    const aKeys = Object.keys(a).filter((key) => a[key]);
    const bKeys = Object.keys(b).filter((key) => b[key]);
    if (aKeys.length !== bKeys.length) return false;
    for (const key of aKeys) {
      if (a[key] !== b[key]) return false;
    }
    return true;
  }

  private addWarning(msg: string) {
    if (this.snapshot.warnings.includes(msg)) return;
    this.snapshot = { ...this.snapshot, warnings: [...this.snapshot.warnings, msg] };
    this.publish();
  }

  private setWindow(
    next: PersistedWorkbenchWindowV1,
    opts?: { skipPersist?: boolean; persistDelayMs?: number },
  ) {
    this.snapshot = { ...this.snapshot, window: next };
    this.publish();
    if (!opts?.skipPersist) this.schedulePersistWindow(opts?.persistDelayMs);
  }

  private schedulePersistWindow(delayMs?: number) {
    if (!this.persistEnabled) return;
    if (this.persistTimer) window.clearTimeout(this.persistTimer);
    const waitMs = Math.max(0, Math.min(5_000, typeof delayMs === "number" ? delayMs : 250));
    this.persistTimer = window.setTimeout(() => {
      this.persistTimer = null;
      const { workspaceId, windowId, window } = this.snapshot;
      saveWorkbenchWindowV1(workspaceId, windowId, window).catch((e: any) => {
        this.persistEnabled = false;
        this.snapshot = { ...this.snapshot, persistEnabled: false };
        this.addWarning(`Workbench persistence disabled: ${e?.message ?? String(e)}`);
      });
    }, waitMs);
  }

  private async hydrate() {
    const { workspaceId, windowId } = this.snapshot;
    try {
      const loaded = await loadWorkbenchWindowV1(workspaceId, windowId);
      if (loaded && !this.layoutDirtyBeforeHydrate) {
        this.snapshot = { ...this.snapshot, window: loaded };
      }
      if (!this.layoutDirtyBeforeHydrate) {
        const migrated = await migrateLegacySelectionToWindowV1({
          workspaceId,
          windowId,
          defaultWindow: this.snapshot.window,
        });
        if (migrated.migrated) {
          this.snapshot = { ...this.snapshot, window: migrated.window };
        }
      }
      const normalized = this.pruneScrollByKey(this.snapshot.window.scrollByKey);
      if (!this.scrollByKeyEquals(this.snapshot.window.scrollByKey, normalized)) {
        this.snapshot = { ...this.snapshot, window: { ...this.snapshot.window, scrollByKey: normalized } };
        this.schedulePersistWindow(0);
      }
    } catch (e: any) {
      this.persistEnabled = false;
      this.snapshot = { ...this.snapshot, persistEnabled: false };
      this.addWarning(`IndexedDB unavailable: ${e?.message ?? String(e)}`);
    } finally {
      this.snapshot = { ...this.snapshot, hydrated: true };
      this.publish();
    }
  }

  private initBroadcast() {
    if (typeof BroadcastChannel === "undefined") {
      this.addWarning("BroadcastChannel unavailable: drafts will not sync across windows.");
      return;
    }
    const { workspaceId } = this.snapshot;
    const name = `ctx-wb-drafts-${encodeURIComponent(workbenchDaemonKey())}-${encodeURIComponent(workspaceId)}`;
    const ch = new BroadcastChannel(name);
    this.channel = ch;
    ch.addEventListener("message", (ev) => {
      const data = ev.data as DraftBroadcastMsg | undefined;
      if (!data || typeof data !== "object") return;
      if (data.workspaceId !== workspaceId) return;
      if (data.windowId === this.snapshot.windowId) return;
      if (data.type === "draft") {
        this.applyRemoteDraft(data.draftKey, data.draft);
      } else if (data.type === "draft_delete") {
        this.applyRemoteDraftDelete(data.draftKey, data.updatedAtMs);
      }
    });
  }

  private applyRemoteDraft(draftKey: string, draft: WorkbenchDraft) {
    const existing = this.snapshot.drafts.byKey[draftKey];
    if (existing && existing.updatedAtMs >= draft.updatedAtMs) return;
    const nextDrafts: DraftSnapshot = {
      byKey: { ...this.snapshot.drafts.byKey, [draftKey]: draft },
      loadedKeys: { ...this.snapshot.drafts.loadedKeys, [draftKey]: true },
    };
    this.snapshot = { ...this.snapshot, drafts: nextDrafts };
    this.publish();
  }

  private applyRemoteDraftDelete(draftKey: string, updatedAtMs: number) {
    const existing = this.snapshot.drafts.byKey[draftKey];
    if (existing && existing.updatedAtMs > updatedAtMs) return;
    const nextByKey = { ...this.snapshot.drafts.byKey };
    delete nextByKey[draftKey];
    const nextLoaded = { ...this.snapshot.drafts.loadedKeys, [draftKey]: true };
    this.snapshot = { ...this.snapshot, drafts: { byKey: nextByKey, loadedKeys: nextLoaded } };
    this.publish();
  }

  getFocusedLeaf(): Extract<LayoutNode, { kind: "leaf" }> | null {
    return findLeaf(this.snapshot.window.layout, this.snapshot.window.focusedLeafId);
  }

  getActiveTab(): WorkbenchTab | null {
    const leaf = this.getFocusedLeaf();
    if (!leaf) return null;
    return getActiveTabFromLeaf(leaf);
  }

  getNavToken = (): WorkbenchNavToken => this.navEpoch;

  bumpNavToken = (): WorkbenchNavToken => {
    this.navEpoch += 1;
    return this.navEpoch;
  };

  private shouldApplyNavToken(navToken?: WorkbenchNavToken): boolean {
    return navToken === undefined || navToken === this.navEpoch;
  }

  private applyNavSource(source?: WorkbenchNavSource) {
    if (source !== "system") this.bumpNavToken();
  }

  focusNewTask = (opts?: WorkbenchNavOpts): boolean => {
    if (!this.shouldApplyNavToken(opts?.navToken)) return false;
    this.applyNavSource(opts?.source);
    this.markLayoutDirty();
    const win = this.snapshot.window;
    const leafId = win.focusedLeafId;
    const leaf = findLeaf(win.layout, leafId);
    if (!leaf) return false;
    const existing = leaf.tabs.find((t) => t.kind === "new_task");
    const tabId = existing?.id ?? randomUuid();
    const newTab: WorkbenchTab = { id: tabId, kind: "new_task" };
    const nextTabs = existing ? leaf.tabs : [newTab, ...leaf.tabs];
    const nextLeaf = ensureLeafActiveTab({ ...leaf, tabs: nextTabs, activeTabId: tabId });
    this.setWindow({ ...win, layout: updateLeaf(win.layout, leafId, () => nextLeaf) }, { persistDelayMs: 0 });
    return true;
  };

  focusTask = (taskId: string, sessionId?: string | null, opts?: WorkbenchNavOpts): boolean => {
    const tid = String(taskId).trim();
    if (!tid) return false;
    if (!this.shouldApplyNavToken(opts?.navToken)) return false;
    this.applyNavSource(opts?.source);
    this.markLayoutDirty();
    const win = this.snapshot.window;
    const leafId = win.focusedLeafId;
    const leaf = findLeaf(win.layout, leafId);
    if (!leaf) return false;

    const existing = leaf.tabs.find((t) => t.kind === "task" && t.ref.taskId === tid);
    const tabId = existing?.id ?? randomUuid();
    const newTab: WorkbenchTab = {
      id: tabId,
      kind: "task",
      ref: { taskId: tid, sessionId: sessionId ?? null },
    };
    const nextTabs = existing
      ? leaf.tabs.map((t) => {
        if (t.id !== tabId) return t;
        if (t.kind !== "task") return t;
        const nextSessionId = sessionId === undefined ? (t.ref.sessionId ?? null) : sessionId;
        return { ...t, ref: { ...t.ref, sessionId: nextSessionId } };
      })
      : [newTab, ...leaf.tabs];

    const nextLeaf = ensureLeafActiveTab({ ...leaf, tabs: nextTabs, activeTabId: tabId });
    this.setWindow({ ...win, layout: updateLeaf(win.layout, leafId, () => nextLeaf) }, { persistDelayMs: 0 });
    return true;
  };

  setActiveSessionForActiveTask = (sessionId: string | null, opts?: WorkbenchNavOpts): boolean => {
    if (!this.shouldApplyNavToken(opts?.navToken)) return false;
    this.applyNavSource(opts?.source);
    this.markLayoutDirty();
    const win = this.snapshot.window;
    const leafId = win.focusedLeafId;
    const leaf = findLeaf(win.layout, leafId);
    if (!leaf) return false;
    const tab = getActiveTabFromLeaf(leaf);
    if (!tab || tab.kind !== "task") return false;
    const nextLeaf = ensureLeafActiveTab({
      ...leaf,
      tabs: leaf.tabs.map((t) =>
        t.id === tab.id && t.kind === "task"
          ? { ...t, ref: { ...t.ref, sessionId } }
          : t,
      ),
    });
    this.setWindow({ ...win, layout: updateLeaf(win.layout, leafId, () => nextLeaf) }, { persistDelayMs: 0 });
    return true;
  };

  setScrollState = (
    key: string,
    next: Omit<WorkbenchScrollState, "updatedAtMs"> & { updatedAtMs?: number },
  ) => {
    const now = Date.now();
    const current = this.snapshot.window.scrollByKey[key];
    const updatedAtMs = next.updatedAtMs ?? now;
    if (
      current &&
      current.stickToBottom === next.stickToBottom &&
      current.anchorItemId === next.anchorItemId &&
      (current.anchorOffset ?? null) === (next.anchorOffset ?? null) &&
      (current.scrollTop ?? null) === (next.scrollTop ?? null) &&
      Math.abs(current.updatedAtMs - updatedAtMs) < 5
    ) {
      return;
    }
    const win = this.snapshot.window;
    const nextEntry: WorkbenchScrollState = {
      stickToBottom: next.stickToBottom,
      anchorItemId: next.anchorItemId,
      anchorOffset: next.anchorOffset ?? null,
      scrollTop: next.scrollTop ?? null,
      updatedAtMs,
    };
    const nextScrollByKey = { ...win.scrollByKey };
    if (nextEntry.stickToBottom) {
      if (!current) return;
      delete nextScrollByKey[key];
    } else {
      nextScrollByKey[key] = nextEntry;
    }
    const pruned = this.pruneScrollByKey(nextScrollByKey);
    if (this.scrollByKeyEquals(win.scrollByKey, pruned)) return;
    this.setWindow({ ...win, scrollByKey: pruned }, { persistDelayMs: 500 });
  };

  getDraft = (draftKey: string): WorkbenchDraft | null => {
    return this.snapshot.drafts.byKey[draftKey] ?? null;
  };

  ensureDraftLoaded = async (draftKey: string) => {
    const key = String(draftKey || "").trim();
    if (!key) return;
    if (this.snapshot.drafts.loadedKeys[key]) return;
    if (this.draftLoadsInFlight.has(key)) return this.draftLoadsInFlight.get(key)!;
    const p = (async () => {
      try {
        const loaded = await loadWorkbenchDraftV1(this.snapshot.workspaceId, key);
        if (loaded) {
          const existing = this.snapshot.drafts.byKey[key];
          if (!existing || existing.updatedAtMs < loaded.updatedAtMs) {
            const nextDrafts: DraftSnapshot = {
              byKey: { ...this.snapshot.drafts.byKey, [key]: loaded },
              loadedKeys: { ...this.snapshot.drafts.loadedKeys, [key]: true },
            };
            this.snapshot = { ...this.snapshot, drafts: nextDrafts };
            this.publish();
          }
        } else {
          const nextLoaded = { ...this.snapshot.drafts.loadedKeys, [key]: true };
          this.snapshot = { ...this.snapshot, drafts: { ...this.snapshot.drafts, loadedKeys: nextLoaded } };
          this.publish();
        }
      } catch (e: any) {
        this.addWarning(`Draft persistence error: ${e?.message ?? String(e)}`);
      } finally {
        this.draftLoadsInFlight.delete(key);
      }
    })();
    this.draftLoadsInFlight.set(key, p);
    return p;
  };

  setDraft = (draftKey: string, next: { text: string; modeId: WorkbenchModeId }) => {
    const key = String(draftKey || "").trim();
    if (!key) return;
    const now = Date.now();
    const current = this.snapshot.drafts.byKey[key];
    const draft: WorkbenchDraft = { text: next.text, modeId: next.modeId, updatedAtMs: now };
    if (current && current.text === draft.text && current.modeId === draft.modeId) return;
    const nextDrafts: DraftSnapshot = {
      byKey: { ...this.snapshot.drafts.byKey, [key]: draft },
      loadedKeys: { ...this.snapshot.drafts.loadedKeys, [key]: true },
    };
    this.snapshot = { ...this.snapshot, drafts: nextDrafts };
    this.publish();

    this.schedulePersistDraft(key, draft);

    try {
      this.channel?.postMessage({
        type: "draft",
        workspaceId: this.snapshot.workspaceId,
        windowId: this.snapshot.windowId,
        draftKey: key,
        draft,
      } satisfies DraftBroadcastMsg);
    } catch {
      // ignore
    }
  };

  flushDraft = async (draftKey: string): Promise<void> => {
    const key = String(draftKey || "").trim();
    if (!key) return;
    if (!this.persistEnabled) return;
    const draft = this.snapshot.drafts.byKey[key];
    if (!draft) return;

    const existingTimer = this.draftTimers.get(key);
    if (existingTimer) window.clearTimeout(existingTimer);
    this.draftTimers.delete(key);

    try {
      await saveWorkbenchDraftV1(this.snapshot.workspaceId, key, draft);
    } catch (e: any) {
      this.addWarning(`Draft persistence failed: ${e?.message ?? String(e)}`);
    }
  };

  private schedulePersistDraft(key: string, draft: WorkbenchDraft) {
    if (!this.persistEnabled) return;
    const existingTimer = this.draftTimers.get(key);
    if (existingTimer) window.clearTimeout(existingTimer);
    const timer = window.setTimeout(() => {
      this.draftTimers.delete(key);
      saveWorkbenchDraftV1(this.snapshot.workspaceId, key, draft).catch((e: any) => {
        this.addWarning(`Draft persistence failed: ${e?.message ?? String(e)}`);
      });
    }, 200);
    this.draftTimers.set(key, timer);
  }
}

const WorkbenchStoreContext = createContext<WorkbenchStore | null>(null);

export function WorkbenchStoreProvider({ workspaceId, children }: { workspaceId: string; children: React.ReactNode }) {
  const store = useMemo(() => new WorkbenchStore(workspaceId), [workspaceId]);
  useEffect(() => {
    store.init();
    return () => {
      // best-effort cleanup
    };
  }, [store]);
  return <WorkbenchStoreContext.Provider value={store}>{children}</WorkbenchStoreContext.Provider>;
}

export function useWorkbenchStore(): WorkbenchStore {
  const s = useContext(WorkbenchStoreContext);
  if (!s) throw new Error("WorkbenchStoreProvider missing");
  return s;
}

export function useWorkbenchSnapshot(): WorkbenchStoreSnapshot {
  const store = useWorkbenchStore();
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
}

export function useWorkbenchShellSnapshot(): WorkbenchShellSnapshot {
  const store = useWorkbenchStore();
  return useSyncExternalStore(store.subscribe, store.getShellSnapshot, store.getShellSnapshot);
}

export function useActiveWorkbenchTab(): WorkbenchTab | null {
  const snap = useWorkbenchSnapshot();
  const leaf = findLeaf(snap.window.layout, snap.window.focusedLeafId);
  if (!leaf) return null;
  return getActiveTabFromLeaf(leaf);
}

export function useActiveWorkbenchIds(): { taskId: string | null; sessionId: string | null } {
  const tab = useActiveWorkbenchTab();
  if (!tab) return { taskId: null, sessionId: null };
  if (tab.kind === "task") return { taskId: tab.ref.taskId, sessionId: tab.ref.sessionId ?? null };
  return { taskId: null, sessionId: null };
}

export function useWorkbenchDraft(
  draftKey: string,
  fallback?: { text: string; modeId: WorkbenchModeId },
): {
  value: { text: string; modeId: WorkbenchModeId };
  setValue: (next: { text: string; modeId: WorkbenchModeId }) => void;
  updatedAtMs: number;
} {
  const store = useWorkbenchStore();
  const snap = useWorkbenchSnapshot();
  const key = String(draftKey || "").trim();
  const draft = (key && snap.drafts.byKey[key]) || null;
  const loaded = !!(key && snap.drafts.loadedKeys[key]);

  useEffect(() => {
    if (!key) return;
    if (loaded) return;
    store.ensureDraftLoaded(key).catch(() => { });
  }, [store, key, loaded]);

  const value = useMemo<{ text: string; modeId: WorkbenchModeId }>(() => {
    if (draft) return { text: draft.text, modeId: draft.modeId };
    return fallback ?? { text: "", modeId: "default" };
  }, [draft, fallback]);

  const setValue = useCallback(
    (next: { text: string; modeId: WorkbenchModeId }) => {
      store.setDraft(key, next);
    },
    [store, key],
  );

  return { value, setValue, updatedAtMs: draft?.updatedAtMs ?? 0 };
}

export function useNewTaskDraft() {
  return useWorkbenchDraft(NEW_TASK_DRAFT_KEY, { text: "", modeId: "default" });
}
