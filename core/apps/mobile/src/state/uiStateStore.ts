import AsyncStorage from "@react-native-async-storage/async-storage";

export type UiKvRecord = {
  key: string;
  value: unknown;
  updatedAtMs: number;
};

async function readRecord(key: string): Promise<UiKvRecord | null> {
  const raw = await AsyncStorage.getItem(key);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as UiKvRecord;
  } catch {
    return null;
  }
}

async function writeRecord(key: string, value: unknown): Promise<void> {
  const record: UiKvRecord = { key, value, updatedAtMs: Date.now() };
  await AsyncStorage.setItem(key, JSON.stringify(record));
}

export async function uiStateGet(key: string): Promise<unknown | null> {
  const rec = await readRecord(key);
  return rec?.value ?? null;
}

export async function uiStateSet(key: string, value: unknown): Promise<void> {
  await writeRecord(key, value);
}

export async function uiStateDelete(key: string): Promise<void> {
  await AsyncStorage.removeItem(key);
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

export function workspaceCatchupKeyV1(workspaceId: string) {
  return `wb.catchup.v1.${workspaceId}`;
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

export async function saveSessionHeadV1(sessionId: string, head: PersistedSessionHeadV1["head"]): Promise<void> {
  await uiStateSet(sessionHeadKeyV1(sessionId), {
    v: 1,
    sessionId,
    head,
    updatedAtMs: Date.now(),
  } satisfies PersistedSessionHeadV1);
}
