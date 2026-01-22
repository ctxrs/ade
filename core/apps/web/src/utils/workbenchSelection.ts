import { idToString } from "../api/client";

export function pickPreferredSession(
  sessions: any[] | undefined | null,
  preferredSessionId?: string | null,
): any | null {
  const list = sessions ?? [];
  if (!Array.isArray(list) || list.length === 0) return null;
  const isSubagent = (s: any) => s?.relationship === "sub_agent";
  const nonSubagents = list.filter((s) => !isSubagent(s));
  const candidates = nonSubagents.length > 0 ? nonSubagents : list;
  if (preferredSessionId) {
    const preferred = candidates.find((s) => idToString((s as any)?.id) === preferredSessionId);
    if (preferred) return preferred;
  }
  for (let i = candidates.length - 1; i >= 0; i--) {
    const s = candidates[i];
    if (s?.status === "active") return s;
  }
  return candidates[candidates.length - 1] ?? null;
}

export function pickPreferredSessionId(
  sessions: any[] | undefined | null,
  preferredSessionId?: string | null,
): string | null {
  const s = pickPreferredSession(sessions, preferredSessionId);
  const id = s ? idToString((s as any).id) : "";
  return id ? String(id) : null;
}
