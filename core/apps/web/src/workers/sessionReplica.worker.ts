import { SessionReplicaCore } from "../state/sessionReplicaCore";
import type { SessionReplicaCommand, SessionReplicaWorkerMessage } from "../state/sessionReplicaProtocol";
import type { Artifact, SessionHeadSnapshot, SessionSnapshot, SessionState } from "@ctx/types";

let apiBaseUrl: string | null = null;
let apiAuthToken: string | null = null;

const setAuth = (baseUrl?: string | null, authToken?: string | null) => {
  apiBaseUrl = baseUrl ?? null;
  apiAuthToken = authToken ?? null;
};

const buildUrl = (path: string): string => {
  if (!apiBaseUrl) return path;
  const base = apiBaseUrl.replace(/\/+$/, "");
  if (path.startsWith("http://") || path.startsWith("https://")) return path;
  if (path.startsWith("/")) return `${base}${path}`;
  return `${base}/${path}`;
};

const fetchJson = async <T>(path: string): Promise<T> => {
  const res = await fetch(buildUrl(path), {
    headers: {
      "content-type": "application/json",
      ...(apiAuthToken ? { authorization: `Bearer ${apiAuthToken}` } : {}),
    },
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(text || `Request failed (${res.status})`);
  }
  return res.json() as Promise<T>;
};

const api = {
  getSessionSnapshot: (sessionId: string, limit?: number, includeEvents?: boolean): Promise<SessionSnapshot> => {
    const qs = new URLSearchParams();
    if (limit) qs.set("limit", String(limit));
    if (includeEvents !== undefined) qs.set("include_events", includeEvents ? "1" : "0");
    const suffix = qs.toString() ? `?${qs.toString()}` : "";
    return fetchJson<SessionSnapshot>(`/api/sessions/${sessionId}/snapshot${suffix}`);
  },
  getSessionHead: (sessionId: string, limit?: number, includeEvents?: boolean): Promise<SessionHeadSnapshot> => {
    const qs = new URLSearchParams();
    if (limit) qs.set("limit", String(limit));
    if (includeEvents !== undefined) qs.set("include_events", includeEvents ? "1" : "0");
    const suffix = qs.toString() ? `?${qs.toString()}` : "";
    return fetchJson<SessionHeadSnapshot>(`/api/sessions/${sessionId}/head${suffix}`);
  },
  listSessionArtifacts: (sessionId: string): Promise<Artifact[]> =>
    fetchJson<Artifact[]>(`/api/sessions/${sessionId}/artifacts`),
  getSessionState: (sessionId: string): Promise<SessionState> => fetchJson<SessionState>(`/api/sessions/${sessionId}/state`),
  setAuth,
};

const core = new SessionReplicaCore({
  api,
  emit: (patches) => {
    const message: SessionReplicaWorkerMessage = { type: "patches", patches };
    self.postMessage(message);
  },
});

self.onmessage = (event: MessageEvent<SessionReplicaCommand>) => {
  const cmd = event.data;
  if (!cmd) return;
  core.handleCommand(cmd);
};
