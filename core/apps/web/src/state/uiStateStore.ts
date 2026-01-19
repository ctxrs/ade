export type UiKvRecord = {
  key: string;
  value: unknown;
  updatedAtMs: number;
};

const DB_NAME = "ctx-ui";
const DB_VERSION = 1;
const STORE_NAME = "kv";
const SESSION_HISTORY_PAGE_LIMIT = 120;

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
  if (!isStringOrNull(rec.sessionId)) return null;

  const taskId = rec.taskId;
  const sessionId = taskId ? rec.sessionId : null;
  return { v: 1, taskId, sessionId };
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

export type PersistedWorkspaceActiveTaskSummaryV1 = {
  task: import("@ctx/types").Task;
  primary_session: import("@ctx/types").SessionSnapshotSummary | null;
  primary_session_head: import("@ctx/types").SessionHeadSnapshot | null;
  sessions: import("@ctx/types").SessionSnapshotSummary[];
  sort_at?: string | null;
};

export type PersistedWorkspaceActiveSnapshotV1 = {
  v: 1;
  workspaceId: string;
  snapshotRev?: number;
  archivedRev?: number;
  active: {
    tasks: PersistedWorkspaceActiveTaskSummaryV1[];
    totalCount?: number;
  };
  updatedAtMs: number;
};

export function workspaceActiveSnapshotKeyV1(workspaceId: string) {
  return `wb.active_snapshot.v1.${workspaceId}`;
}

export async function loadWorkspaceActiveSnapshotV1(
  workspaceId: string,
): Promise<PersistedWorkspaceActiveSnapshotV1 | null> {
  const raw = await uiStateGet(workspaceActiveSnapshotKeyV1(workspaceId));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedWorkspaceActiveSnapshotV1;
  if (rec.v !== 1 || rec.workspaceId !== workspaceId) return null;
  if (!rec.active || !Array.isArray(rec.active.tasks)) {
    const legacy = rec as PersistedWorkspaceActiveSnapshotV1 & {
      tasks?: PersistedWorkspaceActiveTaskSummaryV1[];
      totalCount?: number;
    };
    if (!Array.isArray(legacy.tasks)) return null;
    return {
      v: 1,
      workspaceId,
      snapshotRev: legacy.snapshotRev,
      archivedRev: legacy.archivedRev,
      active: {
        tasks: legacy.tasks,
        totalCount: legacy.totalCount,
      },
      updatedAtMs: legacy.updatedAtMs,
    };
  }
  return rec;
}

export async function saveWorkspaceActiveSnapshotV1(
  workspaceId: string,
  payload: Omit<PersistedWorkspaceActiveSnapshotV1, "v" | "workspaceId" | "updatedAtMs">,
): Promise<void> {
  await uiStateSet(workspaceActiveSnapshotKeyV1(workspaceId), {
    v: 1,
    workspaceId,
    updatedAtMs: Date.now(),
    ...payload,
  } satisfies PersistedWorkspaceActiveSnapshotV1);
}

export type PersistedSessionHeadV1 = {
  v: 1;
  sessionId: string;
  head: import("@ctx/types").SessionHead;
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

export type PersistedSessionHistoryPageV1 = {
  v: 1;
  sessionId: string;
  beforeSeq: number;
  limit: number;
  page: import("@ctx/types").SessionHistoryPage;
  updatedAtMs: number;
};

type PersistedSessionHistoryIndexV1 = {
  v: 1;
  entries: Array<{ key: string; updatedAtMs: number }>;
};

function sessionHistoryIndexKeyV1() {
  return "wb.session_history_index.v1";
}

export function sessionHistoryPageKeyV1(sessionId: string, beforeSeq: number, limit: number) {
  return "wb.session_history_page.v1." + sessionId + "." + beforeSeq + "." + limit;
}

function decodeSessionHistoryIndexV1(raw: unknown): PersistedSessionHistoryIndexV1 {
  if (!raw || typeof raw !== "object") return { v: 1, entries: [] };
  const rec = raw as PersistedSessionHistoryIndexV1;
  if (rec.v !== 1 || !Array.isArray(rec.entries)) return { v: 1, entries: [] };
  const entries = rec.entries.filter(
    (entry) => entry && typeof entry.key === "string" && typeof entry.updatedAtMs === "number",
  );
  return { v: 1, entries };
}

export async function loadSessionHistoryPageV1(
  sessionId: string,
  beforeSeq: number,
  limit: number,
): Promise<PersistedSessionHistoryPageV1 | null> {
  const raw = await uiStateGet(sessionHistoryPageKeyV1(sessionId, beforeSeq, limit));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSessionHistoryPageV1;
  if (rec.v !== 1 || rec.sessionId !== sessionId) return null;
  return rec;
}

export async function saveSessionHistoryPageV1(
  sessionId: string,
  beforeSeq: number,
  limit: number,
  page: PersistedSessionHistoryPageV1["page"],
): Promise<void> {
  const key = sessionHistoryPageKeyV1(sessionId, beforeSeq, limit);
  const now = Date.now();
  await uiStateSet(key, {
    v: 1,
    sessionId,
    beforeSeq,
    limit,
    page,
    updatedAtMs: now,
  } satisfies PersistedSessionHistoryPageV1);

  const index = decodeSessionHistoryIndexV1(await uiStateGet(sessionHistoryIndexKeyV1()));
  const entries = index.entries.filter((entry) => entry.key !== key);
  entries.unshift({ key, updatedAtMs: now });
  entries.sort((a, b) => b.updatedAtMs - a.updatedAtMs);

  const pruned = entries.slice(SESSION_HISTORY_PAGE_LIMIT);
  const trimmed = entries.slice(0, SESSION_HISTORY_PAGE_LIMIT);
  await uiStateSet(sessionHistoryIndexKeyV1(), { v: 1, entries: trimmed } satisfies PersistedSessionHistoryIndexV1);
  for (const entry of pruned) {
    await uiStateDelete(entry.key);
  }
}

export type PersistedSessionAcpMetaV1 = {
  v: 1;
  sessionId: string;
  models?: any;
  modes?: any;
  currentModelId?: string;
  updatedAtMs: number;
};

export function sessionAcpMetaKeyV1(sessionId: string) {
  return `wb.session_acp_meta.v1.${sessionId}`;
}

export async function loadSessionAcpMetaV1(sessionId: string): Promise<PersistedSessionAcpMetaV1 | null> {
  const raw = await uiStateGet(sessionAcpMetaKeyV1(sessionId));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSessionAcpMetaV1;
  if (rec.v !== 1 || rec.sessionId !== sessionId) return null;
  return rec;
}

export async function saveSessionAcpMetaV1(
  sessionId: string,
  meta: Omit<PersistedSessionAcpMetaV1, "v" | "sessionId" | "updatedAtMs">,
): Promise<void> {
  await uiStateSet(sessionAcpMetaKeyV1(sessionId), {
    v: 1,
    sessionId,
    updatedAtMs: Date.now(),
    ...meta,
  } satisfies PersistedSessionAcpMetaV1);
}

export type PersistedSettingsV1 = {
  v: 1;
  settings: import("../api/client").Settings;
  updatedAtMs: number;
};

export function settingsKeyV1() {
  return "wb.settings.v1";
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

export type SessionViewVerbosity = "terse" | "default" | "verbose";

export type PersistedSessionViewPrefsV1 = {
  v: 1;
  verbosity: SessionViewVerbosity;
  updatedAtMs: number;
};

export function sessionViewPrefsKeyV1() {
  return "wb.session_view_prefs.v1";
}

export async function loadSessionViewPrefsV1(): Promise<PersistedSessionViewPrefsV1 | null> {
  const raw = await uiStateGet(sessionViewPrefsKeyV1());
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSessionViewPrefsV1;
  if (rec.v !== 1) return null;
  if (!["terse", "default", "verbose"].includes(String(rec.verbosity))) return null;
  return rec;
}

export async function saveSessionViewPrefsV1(verbosity: SessionViewVerbosity): Promise<void> {
  await uiStateSet(sessionViewPrefsKeyV1(), {
    v: 1,
    verbosity,
    updatedAtMs: Date.now(),
  } satisfies PersistedSessionViewPrefsV1);
}
