import { getWebappStorage } from "./storage";

export type UiKvRecord = {
  key: string;
  value: unknown;
  updatedAtMs: number;
};

export type UiStateBatchOp =
  | { kind: "set"; key: string; value: unknown }
  | { kind: "delete"; key: string };

const SESSION_HISTORY_PAGE_LIMIT = 120;
const SESSION_HISTORY_PAGE_TTL_MS = 7 * 24 * 60 * 60 * 1000;
const SESSION_HISTORY_TOUCH_GRACE_MS = 30 * 1000;

const storage = getWebappStorage();

export async function uiStateBatch(ops: UiStateBatchOp[]): Promise<void> {
  if (ops.length === 0) return;
  for (const op of ops) {
    if (op.kind === "set") {
      await storage.setKv(op.key, op.value);
    } else {
      await storage.deleteKv(op.key);
    }
  }
  await storage.flush();
}

export async function uiStateGet(key: string): Promise<unknown | null> {
  return (await storage.getKv<unknown>(key)) ?? null;
}

export async function uiStateSet(key: string, value: unknown): Promise<void> {
  await storage.setKv(key, value);
}

export async function uiStateDelete(key: string): Promise<void> {
  await storage.deleteKv(key);
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
  worktreeVcsSnapshots?: import("@ctx/types").WorktreeVcsSnapshot[];
  active: {
    tasks: PersistedWorkspaceActiveTaskSummaryV1[];
    totalCount?: number;
  };
  updatedAtMs: number;
};

export function workspaceActiveSnapshotKeyV1(workspaceId: string) {
  return `wb.active_snapshot.v1.${workspaceId}`;
}

export function decodeWorkspaceActiveSnapshotV1(
  raw: unknown,
  workspaceId: string,
): PersistedWorkspaceActiveSnapshotV1 | null {
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

export async function loadWorkspaceActiveSnapshotV1(
  workspaceId: string,
): Promise<PersistedWorkspaceActiveSnapshotV1 | null> {
  const raw = await storage.getSnapshot<PersistedWorkspaceActiveSnapshotV1>(
    workspaceActiveSnapshotKeyV1(workspaceId),
  );
  return decodeWorkspaceActiveSnapshotV1(raw, workspaceId);
}

export async function saveWorkspaceActiveSnapshotV1(
  workspaceId: string,
  payload: Omit<PersistedWorkspaceActiveSnapshotV1, "v" | "workspaceId" | "updatedAtMs">,
): Promise<void> {
  await storage.setSnapshot(workspaceActiveSnapshotKeyV1(workspaceId), {
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
  const raw = await storage.getSnapshot<PersistedSessionHeadV1>(sessionHeadKeyV1(sessionId));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSessionHeadV1;
  if (rec.v !== 1 || rec.sessionId !== sessionId || !rec.head) return null;
  return rec;
}

export async function saveSessionHeadV1(
  sessionId: string,
  head: PersistedSessionHeadV1["head"],
): Promise<void> {
  await storage.setSnapshot(sessionHeadKeyV1(sessionId), {
    v: 1,
    sessionId,
    head,
    updatedAtMs: Date.now(),
  } satisfies PersistedSessionHeadV1);
}

export type PersistedSessionAcpMetaV1 = {
  v: 1;
  sessionId: string;
  models?: unknown;
  modes?: unknown;
  currentModelId?: string;
  updatedAtMs: number;
};

export function sessionAcpMetaKeyV1(sessionId: string) {
  return `wb.session_acp_meta.v1.${sessionId}`;
}

export async function loadSessionAcpMetaV1(sessionId: string): Promise<PersistedSessionAcpMetaV1 | null> {
  const raw = await storage.getSnapshot<PersistedSessionAcpMetaV1>(sessionAcpMetaKeyV1(sessionId));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSessionAcpMetaV1;
  if (rec.v !== 1 || rec.sessionId !== sessionId) return null;
  return rec;
}

export async function saveSessionAcpMetaV1(
  sessionId: string,
  payload: Omit<PersistedSessionAcpMetaV1, "v" | "sessionId" | "updatedAtMs">,
): Promise<void> {
  await storage.setSnapshot(sessionAcpMetaKeyV1(sessionId), {
    v: 1,
    sessionId,
    updatedAtMs: Date.now(),
    ...payload,
  } satisfies PersistedSessionAcpMetaV1);
}

export type PersistedThoughtRowV1 = {
  key: string;
  event: import("@ctx/types").SessionEvent;
  updatedAtMs?: number;
};

export type PersistedSessionThoughtsV1 = {
  sessionId: string;
  thoughts: Record<string, PersistedThoughtRowV1>;
};

export type PersistedTaskThoughtsV1 = {
  v: 1;
  taskId: string;
  sessions: Record<string, PersistedSessionThoughtsV1>;
  updatedAtMs: number;
};

export function taskThoughtsKeyV1(taskId: string) {
  return `wb.task_thoughts.v1.${taskId}`;
}

export async function loadTaskThoughtsV1(taskId: string): Promise<PersistedTaskThoughtsV1 | null> {
  const raw = await storage.getSnapshot<PersistedTaskThoughtsV1>(taskThoughtsKeyV1(taskId));
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedTaskThoughtsV1;
  if (rec.v !== 1 || rec.taskId !== taskId || !rec.sessions || typeof rec.sessions !== "object") {
    return null;
  }
  return rec;
}

export async function saveTaskThoughtsV1(
  taskId: string,
  payload: Omit<PersistedTaskThoughtsV1, "v" | "taskId" | "updatedAtMs">,
): Promise<void> {
  await storage.setSnapshot(taskThoughtsKeyV1(taskId), {
    v: 1,
    taskId,
    updatedAtMs: Date.now(),
    ...payload,
  } satisfies PersistedTaskThoughtsV1);
}

export async function clearTaskThoughtsV1(taskId: string): Promise<void> {
  await storage.deleteSnapshot(taskThoughtsKeyV1(taskId));
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
  const entries: PersistedSessionHistoryIndexV1["entries"] = [];
  for (const entry of rec.entries) {
    if (!entry || typeof entry.key !== "string") continue;
    if (!Number.isFinite(entry.updatedAtMs)) continue;
    entries.push({ key: entry.key, updatedAtMs: entry.updatedAtMs });
  }
  return { v: 1, entries };
}

function planSessionHistoryEvictions(
  entries: PersistedSessionHistoryIndexV1["entries"],
  nowMs: number,
): { kept: PersistedSessionHistoryIndexV1["entries"]; evictKeys: string[] } {
  const expiresBefore = nowMs - SESSION_HISTORY_PAGE_TTL_MS;
  const evict = new Set<string>();
  const latestByKey = new Map<string, number>();

  for (const entry of entries) {
    if (!entry.key || !Number.isFinite(entry.updatedAtMs)) continue;
    if (entry.updatedAtMs < expiresBefore) {
      evict.add(entry.key);
      continue;
    }
    const existing = latestByKey.get(entry.key);
    if (existing === undefined || existing < entry.updatedAtMs) {
      latestByKey.set(entry.key, entry.updatedAtMs);
    }
  }

  const sorted = Array.from(latestByKey.entries())
    .map(([key, updatedAtMs]) => ({ key, updatedAtMs }))
    .sort((a, b) => b.updatedAtMs - a.updatedAtMs);
  const kept = sorted.slice(0, SESSION_HISTORY_PAGE_LIMIT);
  const keptKeys = new Set(kept.map((entry) => entry.key));

  for (const entry of sorted.slice(SESSION_HISTORY_PAGE_LIMIT)) {
    evict.add(entry.key);
  }

  const evictKeys = Array.from(evict).filter((key) => !keptKeys.has(key));
  return { kept, evictKeys };
}

async function updateSessionHistoryIndexV1(
  key: string,
  nowMs: number,
  opts?: { force?: boolean },
): Promise<void> {
  const index = decodeSessionHistoryIndexV1(
    await storage.getKv<PersistedSessionHistoryIndexV1>(sessionHistoryIndexKeyV1()),
  );
  const existing = index.entries.find((entry) => entry.key === key);
  if (!opts?.force && existing && nowMs - existing.updatedAtMs < SESSION_HISTORY_TOUCH_GRACE_MS) {
    return;
  }
  const entries = index.entries.filter((entry) => entry.key !== key);
  entries.unshift({ key, updatedAtMs: nowMs });
  const { kept, evictKeys } = planSessionHistoryEvictions(entries, nowMs);
  await storage.setKv(sessionHistoryIndexKeyV1(), { v: 1, entries: kept } satisfies PersistedSessionHistoryIndexV1);
  if (evictKeys.length === 0) return;
  await Promise.all(evictKeys.map((evictKey) => storage.deleteHistoryPage(evictKey)));
}

export async function loadSessionHistoryPageV1(
  sessionId: string,
  beforeSeq: number,
  limit: number,
): Promise<PersistedSessionHistoryPageV1 | null> {
  const key = sessionHistoryPageKeyV1(sessionId, beforeSeq, limit);
  const raw = await storage.getHistoryPage<PersistedSessionHistoryPageV1>(key);
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSessionHistoryPageV1;
  if (rec.v !== 1 || rec.sessionId !== sessionId) return null;
  void updateSessionHistoryIndexV1(key, Date.now()).catch(() => {});
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
  const pageValue: PersistedSessionHistoryPageV1 = {
    v: 1,
    sessionId,
    beforeSeq,
    limit,
    page,
    updatedAtMs: now,
  };

  await storage.setHistoryPage(key, pageValue);
  await updateSessionHistoryIndexV1(key, now, { force: true });
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
  const raw = await storage.getKv<PersistedSettingsV1>(settingsKeyV1());
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSettingsV1;
  if (rec.v !== 1 || !rec.settings) return null;
  return rec;
}

export async function saveSettingsV1(settings: PersistedSettingsV1["settings"]): Promise<void> {
  await storage.setKv(settingsKeyV1(), {
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
  const raw = await storage.getKv<PersistedSessionViewPrefsV1>(sessionViewPrefsKeyV1());
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as PersistedSessionViewPrefsV1;
  if (rec.v !== 1) return null;
  if (!["terse", "default", "verbose"].includes(String(rec.verbosity))) return null;
  return rec;
}

export async function saveSessionViewPrefsV1(verbosity: SessionViewVerbosity): Promise<void> {
  await storage.setKv(sessionViewPrefsKeyV1(), {
    v: 1,
    verbosity,
    updatedAtMs: Date.now(),
  } satisfies PersistedSessionViewPrefsV1);
}
