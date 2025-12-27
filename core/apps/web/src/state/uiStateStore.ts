export type UiKvRecord = {
  key: string;
  value: unknown;
  updatedAtMs: number;
};

const DB_NAME = "context-ui";
const DB_VERSION = 1;
const STORE_NAME = "kv";

let dbPromise: Promise<IDBDatabase> | null = null;

function requestToPromise<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.addEventListener("success", () => resolve(req.result));
    req.addEventListener("error", () => reject(req.error ?? new Error("IndexedDB request failed")));
  });
}

function txDone(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.addEventListener("complete", () => resolve());
    tx.addEventListener("abort", () => reject(tx.error ?? new Error("IndexedDB transaction aborted")));
    tx.addEventListener("error", () => reject(tx.error ?? new Error("IndexedDB transaction failed")));
  });
}

async function openDb(): Promise<IDBDatabase> {
  if (dbPromise) return dbPromise;
  if (typeof indexedDB === "undefined") {
    throw new Error("IndexedDB is unavailable in this environment.");
  }

  dbPromise = new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.addEventListener("upgradeneeded", () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME, { keyPath: "key" });
      }
    });
    req.addEventListener("success", () => resolve(req.result));
    req.addEventListener("error", () => reject(req.error ?? new Error("Failed to open IndexedDB")));
  });

  return dbPromise;
}

export async function uiStateGet(key: string): Promise<unknown | null> {
  const db = await openDb();
  const tx = db.transaction(STORE_NAME, "readonly");
  const store = tx.objectStore(STORE_NAME);
  const rec = (await requestToPromise(store.get(key))) as UiKvRecord | undefined;
  await txDone(tx);
  return rec?.value ?? null;
}

export async function uiStateSet(key: string, value: unknown): Promise<void> {
  const db = await openDb();
  const tx = db.transaction(STORE_NAME, "readwrite");
  const store = tx.objectStore(STORE_NAME);
  await requestToPromise(store.put({ key, value, updatedAtMs: Date.now() } satisfies UiKvRecord));
  await txDone(tx);
}

export async function uiStateDelete(key: string): Promise<void> {
  const db = await openDb();
  const tx = db.transaction(STORE_NAME, "readwrite");
  const store = tx.objectStore(STORE_NAME);
  await requestToPromise(store.delete(key));
  await txDone(tx);
}

export type PersistedWorkbenchSelectionV1 = {
  v: 1;
  taskId: string | null;
  trackId: string | null;
  sessionId: string | null;
};

export function workbenchSelectionKeyV1(workspaceId: string) {
  return `wb.selection.v1.${workspaceId}`;
}

function isStringOrNull(v: unknown): v is string | null {
  return v === null || typeof v === "string";
}

export function decodeWorkbenchSelectionV1(raw: unknown): PersistedWorkbenchSelectionV1 | null {
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as Record<string, unknown>;
  if (rec.v !== 1) return null;
  if (!isStringOrNull(rec.taskId)) return null;
  if (!isStringOrNull(rec.trackId)) return null;
  if (!isStringOrNull(rec.sessionId)) return null;

  const taskId = rec.taskId;
  const trackId = taskId ? rec.trackId : null;
  const sessionId = taskId && trackId ? rec.sessionId : null;
  return { v: 1, taskId, trackId, sessionId };
}

export async function loadWorkbenchSelectionV1(workspaceId: string): Promise<PersistedWorkbenchSelectionV1 | null> {
  const raw = await uiStateGet(workbenchSelectionKeyV1(workspaceId));
  return decodeWorkbenchSelectionV1(raw);
}

export async function saveWorkbenchSelectionV1(workspaceId: string, sel: PersistedWorkbenchSelectionV1): Promise<void> {
  await uiStateSet(workbenchSelectionKeyV1(workspaceId), sel);
}

export async function clearWorkbenchSelectionV1(workspaceId: string): Promise<void> {
  await uiStateDelete(workbenchSelectionKeyV1(workspaceId));
}

export type PersistedWorkspaceCatchupV1 = {
  v: 1;
  snapshot: import("@context/types").WorkspaceCatchupSnapshot;
  updatedAtMs: number;
};

export type PersistedSettingsV1 = {
  v: 1;
  settings: import("../api/client").Settings;
  updatedAtMs: number;
};

export type PersistedTrackDiffV1 = {
  v: 1;
  trackId: string;
  diff: string;
  updatedAtMs: number;
};

export type PersistedWorktreeV1 = {
  v: 1;
  worktreeId: string;
  rootPath: string;
  updatedAtMs: number;
};

export function workspaceCatchupKeyV1(workspaceId: string) {
  return `wb.catchup.v1.${workspaceId}`;
}

export function settingsKeyV1() {
  return "ui.settings.v1";
}

export function trackDiffKeyV1(trackId: string) {
  return `wb.track_diff.v1.${trackId}`;
}

export function worktreeKeyV1(worktreeId: string) {
  return `wb.worktree.v1.${worktreeId}`;
}

export async function loadWorkspaceCatchupV1(
  workspaceId: string,
): Promise<PersistedWorkspaceCatchupV1 | null> {
  const raw = await uiStateGet(workspaceCatchupKeyV1(workspaceId));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedWorkspaceCatchupV1;
  if (rec.v !== 1 || !rec.snapshot) return null;
  return rec;
}

export async function saveWorkspaceCatchupV1(
  workspaceId: string,
  snapshot: PersistedWorkspaceCatchupV1["snapshot"],
): Promise<void> {
  await uiStateSet(workspaceCatchupKeyV1(workspaceId), {
    v: 1,
    snapshot,
    updatedAtMs: Date.now(),
  } satisfies PersistedWorkspaceCatchupV1);
}

export async function loadSettingsV1(): Promise<PersistedSettingsV1 | null> {
  const raw = await uiStateGet(settingsKeyV1());
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSettingsV1;
  if (rec.v !== 1 || !rec.settings) return null;
  return rec;
}

export async function saveSettingsV1(settings: PersistedSettingsV1["settings"]): Promise<void> {
  await uiStateSet(settingsKeyV1(), {
    v: 1,
    settings,
    updatedAtMs: Date.now(),
  } satisfies PersistedSettingsV1);
}

export async function loadTrackDiffV1(trackId: string): Promise<PersistedTrackDiffV1 | null> {
  const raw = await uiStateGet(trackDiffKeyV1(trackId));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedTrackDiffV1;
  if (rec.v !== 1 || rec.trackId !== trackId || typeof rec.diff !== "string") return null;
  return rec;
}

export async function saveTrackDiffV1(trackId: string, diff: string): Promise<void> {
  await uiStateSet(trackDiffKeyV1(trackId), {
    v: 1,
    trackId,
    diff,
    updatedAtMs: Date.now(),
  } satisfies PersistedTrackDiffV1);
}

export async function deleteTrackDiffV1(trackId: string): Promise<void> {
  await uiStateDelete(trackDiffKeyV1(trackId));
}

export async function loadWorktreeV1(worktreeId: string): Promise<PersistedWorktreeV1 | null> {
  const raw = await uiStateGet(worktreeKeyV1(worktreeId));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedWorktreeV1;
  if (rec.v !== 1 || rec.worktreeId !== worktreeId || typeof rec.rootPath !== "string") return null;
  return rec;
}

export async function saveWorktreeV1(worktreeId: string, rootPath: string): Promise<void> {
  await uiStateSet(worktreeKeyV1(worktreeId), {
    v: 1,
    worktreeId,
    rootPath,
    updatedAtMs: Date.now(),
  } satisfies PersistedWorktreeV1);
}

export type PersistedSessionHeadV1 = {
  v: 1;
  sessionId: string;
  head: import("@context/types").SessionHead;
  updatedAtMs: number;
};

export function sessionHeadKeyV1(sessionId: string) {
  return `wb.session_head.v1.${sessionId}`;
}

export async function loadSessionHeadV1(sessionId: string): Promise<PersistedSessionHeadV1 | null> {
  const raw = await uiStateGet(sessionHeadKeyV1(sessionId));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSessionHeadV1;
  if (rec.v !== 1 || rec.sessionId !== sessionId || !rec.head) return null;
  return rec;
}

export async function saveSessionHeadV1(
  sessionId: string,
  head: PersistedSessionHeadV1["head"],
): Promise<void> {
  await uiStateSet(sessionHeadKeyV1(sessionId), {
    v: 1,
    sessionId,
    head,
    updatedAtMs: Date.now(),
  } satisfies PersistedSessionHeadV1);
}
