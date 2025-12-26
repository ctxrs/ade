import { idToString } from "../api/client";

export function pickPreferredSession(
  sessions: any[] | undefined | null,
  preferredSessionId?: string | null,
): any | null {
  const list = sessions ?? [];
  if (!Array.isArray(list) || list.length === 0) return null;
  if (preferredSessionId) {
    const preferred = list.find((s) => idToString((s as any)?.id) === preferredSessionId);
    if (preferred) return preferred;
  }
  for (let i = list.length - 1; i >= 0; i--) {
    const s = list[i];
    if (s?.status === "active") return s;
  }
  return list[list.length - 1] ?? null;
}

export function pickPreferredSessionId(
  sessions: any[] | undefined | null,
  preferredSessionId?: string | null,
): string | null {
  const s = pickPreferredSession(sessions, preferredSessionId);
  const id = s ? idToString((s as any).id) : "";
  return id ? String(id) : null;
}

export function pickPreferredTrackId(
  trackIds: string[],
  sessionsByTrack: Record<string, any[] | undefined>,
  currentTrackId: string | null,
): string | null {
  const ids = Array.isArray(trackIds) ? trackIds.filter(Boolean) : [];
  if (ids.length === 0) return null;

  const isValid = (id: string | null) => !!id && ids.includes(id);
  const hasSession = (id: string | null) => isValid(id) && (sessionsByTrack[id!]?.length ?? 0) > 0;

  const validCurrent = isValid(currentTrackId) ? currentTrackId : null;
  if (hasSession(validCurrent)) return validCurrent;

  const firstWithSession = ids.find((id) => (sessionsByTrack[id]?.length ?? 0) > 0) ?? null;
  return firstWithSession ?? validCurrent ?? ids[0] ?? null;
}
