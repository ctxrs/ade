import type {
  Diagnostics,
  Message,
  MessageAttachment,
  MobileDeviceRegistration,
  ProviderStatus,
  Session,
  SessionHead,
  SessionHistoryPage,
  SessionTurnTool,
  Task,
  Track,
  Workspace,
  WorkspaceCatchupCursor,
  WorkspaceCatchupSnapshot,
} from "@context/types";

export type ConnectionConfig = {
  baseUrl: string;
  token: string;
};

export type WorkspaceSummary = {
  id: string;
  name: string;
  root_path: string;
  created_at: string;
};

export type TaskSummary = {
  id: string;
  workspace_id: string;
  title: string;
  description?: string | null;
  status: string;
  updated_at: string;
  last_activity_at?: string | null;
};

export type TrackSummary = {
  id: string;
  task_id: string;
  label: string;
  status: string;
  worktree_id: string;
};

export type SessionSummary = {
  id: string;
  track_id: string;
  task_id?: string;
  workspace_id?: string;
  worktree_id?: string;
  provider_id: string;
  model_id: string;
  title: string;
  status: string;
  agent_role: string;
};

export type MessageSummary = {
  id: string;
  session_id: string;
  role: "user" | "assistant" | "system";
  content: string;
  delivery: "immediate" | "queued";
  created_at: string;
};

const ensureSlash = (input: string): string => {
  if (!input.trim()) return "";
  return input.endsWith("/") ? input : `${input}/`;
};

const buildUrl = (conn: ConnectionConfig, path: string): string => {
  const normalized = ensureSlash(conn.baseUrl);
  const absolute = path.startsWith("/") ? path.slice(1) : path;
  return `${normalized}${absolute}`;
};

export const idToString = (value: unknown): string => {
  if (typeof value === "string") return value;
  if (value && typeof value === "object" && "0" in (value as Record<string, unknown>)) {
    return String((value as Record<string, unknown>)["0"]);
  }
  return value ? String(value) : "";
};

const mapWorkspace = (item: Workspace): WorkspaceSummary => ({
  id: idToString(item.id),
  name: item.name,
  root_path: item.root_path,
  created_at: item.created_at,
});

const mapTask = (item: Task): TaskSummary => ({
  id: idToString(item.id),
  workspace_id: idToString(item.workspace_id),
  title: item.title,
  description: item.description,
  status: item.status,
  updated_at: item.updated_at,
  last_activity_at: item.last_activity_at,
});

const mapTrack = (item: Track): TrackSummary => ({
  id: idToString(item.id),
  task_id: idToString(item.task_id),
  label: item.label,
  status: item.status,
  worktree_id: idToString(item.worktree_id),
});

const mapSession = (item: Session): SessionSummary => ({
  id: idToString(item.id),
  track_id: idToString(item.track_id),
  task_id: idToString(item.task_id),
  workspace_id: idToString(item.workspace_id),
  worktree_id: idToString(item.worktree_id),
  provider_id: item.provider_id,
  model_id: item.model_id,
  title: item.title,
  status: item.status,
  agent_role: item.agent_role,
});

const mapMessage = (item: Message): MessageSummary => ({
  id: idToString(item.id),
  session_id: idToString(item.session_id),
  role: item.role,
  content: item.content,
  delivery: item.delivery,
  created_at: item.created_at,
});

async function fetchJson<T>(conn: ConnectionConfig, path: string, init?: RequestInit): Promise<T> {
  const url = buildUrl(conn, path);
  const res = await fetch(url, {
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${conn.token}`,
      ...(init?.headers as Record<string, string> | undefined),
    },
    ...init,
  });

  const text = await res.text();
  if (!res.ok) {
    const message = text || `${res.status} ${res.statusText}`;
    throw new Error(message.trim());
  }

  if (!text) {
    return undefined as T;
  }

  try {
    return JSON.parse(text) as T;
  } catch {
    throw new Error(`Unexpected response from daemon: ${text.slice(0, 200)}`);
  }
}

export const listWorkspaces = (conn: ConnectionConfig) =>
  fetchJson<Workspace[]>(conn, "/api/workspaces").then((items) => items.map(mapWorkspace));

export const createWorkspace = (conn: ConnectionConfig, root_path: string, name?: string) =>
  fetchJson<Workspace>(conn, "/api/workspaces", {
    method: "POST",
    body: JSON.stringify({ root_path, name }),
  }).then(mapWorkspace);

export const listTasks = (conn: ConnectionConfig, workspaceId: string) =>
  fetchJson<Task[]>(conn, `/api/workspaces/${workspaceId}/tasks`).then((items) => items.map(mapTask));

export const listTracks = (conn: ConnectionConfig, taskId: string) =>
  fetchJson<Track[]>(conn, `/api/tasks/${taskId}/tracks`).then((items) => items.map(mapTrack));

export const listSessionsForTrack = (conn: ConnectionConfig, trackId: string) =>
  fetchJson<Session[]>(conn, `/api/tracks/${trackId}/sessions`).then((items) => items.map(mapSession));

export const listMessages = (conn: ConnectionConfig, sessionId: string) =>
  fetchJson<Message[]>(conn, `/api/sessions/${sessionId}/messages`).then((items) => items.map(mapMessage));

export const postMessage = (
  conn: ConnectionConfig,
  sessionId: string,
  content: string,
  delivery?: "immediate" | "queued",
  attachments?: MessageAttachment[],
) =>
  fetchJson<Message>(conn, `/api/sessions/${sessionId}/messages`, {
    method: "POST",
    body: JSON.stringify({ content, delivery, attachments: attachments ?? [] }),
  }).then(mapMessage);

export const fetchTrackDiff = (conn: ConnectionConfig, trackId: string) =>
  fetchJson<{ diff: string }>(conn, `/api/tracks/${trackId}/diff`);

export const listProviders = (conn: ConnectionConfig) =>
  fetchJson<ProviderStatus[]>(conn, "/api/providers");

export const getDiagnostics = (conn: ConnectionConfig) =>
  fetchJson<Diagnostics>(conn, "/api/diagnostics");

export const listQueue = (conn: ConnectionConfig, sessionId: string) =>
  fetchJson<Message[]>(conn, `/api/sessions/${sessionId}/queue`).then((items) => items.map(mapMessage));

export type RegisterMobileDeviceRequest = {
  device_id: string;
  device_label?: string;
  platform?: string;
  push_token?: string;
  push_provider?: string;
  public_key?: string;
  app_version?: string;
};

export const registerMobileDevice = (conn: ConnectionConfig, payload: RegisterMobileDeviceRequest) =>
  fetchJson<MobileDeviceRegistration>(conn, `/api/mobile/register`, {
    method: "POST",
    body: JSON.stringify(payload),
  });

export type WorkspaceCatchupParams = {
  limit?: number;
  includeArchived?: boolean;
  archivedOnly?: boolean;
  activeCursor?: WorkspaceCatchupCursor | null;
  archivedCursor?: WorkspaceCatchupCursor | null;
};

export const getWorkspaceCatchup = (
  conn: ConnectionConfig,
  workspaceId: string,
  params?: WorkspaceCatchupParams,
) => {
  const search = new URLSearchParams();
  if (params?.limit) search.set("limit", String(params.limit));
  if (params?.includeArchived) search.set("include_archived", "1");
  if (params?.archivedOnly) search.set("archived_only", "1");
  if (params?.activeCursor) {
    search.set("active_cursor_sort_at", params.activeCursor.sort_at);
    search.set("active_cursor_task_id", idToString(params.activeCursor.task_id));
  }
  if (params?.archivedCursor) {
    search.set("archived_cursor_sort_at", params.archivedCursor.sort_at);
    search.set("archived_cursor_task_id", idToString(params.archivedCursor.task_id));
  }
  const qs = search.toString();
  const suffix = qs ? `?${qs}` : "";
  return fetchJson<WorkspaceCatchupSnapshot>(conn, `/api/workspaces/${workspaceId}/catchup${suffix}`);
};

export const createTask = (
  conn: ConnectionConfig,
  workspaceId: string,
  title: string,
  description?: string,
  opts?: { create_default_track?: boolean; default_track_label?: string },
) =>
  fetchJson<Task>(conn, `/api/workspaces/${workspaceId}/tasks`, {
    method: "POST",
    body: JSON.stringify({
      title,
      description,
      ...(opts?.create_default_track === undefined ? {} : { create_default_track: opts.create_default_track }),
      ...(opts?.default_track_label === undefined ? {} : { default_track_label: opts.default_track_label }),
    }),
  });

export const updateTaskTitle = (conn: ConnectionConfig, taskId: string, title: string) =>
  fetchJson<Task>(conn, `/api/tasks/${taskId}/title`, { method: "POST", body: JSON.stringify({ title }) });

export const deleteTask = (conn: ConnectionConfig, taskId: string) =>
  fetchJson<void>(conn, `/api/tasks/${taskId}`, { method: "DELETE" });

export const archiveTask = (conn: ConnectionConfig, taskId: string) =>
  fetchJson<Task>(conn, `/api/tasks/${taskId}/archive`, { method: "POST" });

export const unarchiveTask = (conn: ConnectionConfig, taskId: string) =>
  fetchJson<Task>(conn, `/api/tasks/${taskId}/unarchive`, { method: "POST" });

export const markTaskRead = (conn: ConnectionConfig, taskId: string) =>
  fetchJson<Task>(conn, `/api/tasks/${taskId}/mark_read`, { method: "POST" });

export const markTaskUnread = (conn: ConnectionConfig, taskId: string) =>
  fetchJson<Task>(conn, `/api/tasks/${taskId}/mark_unread`, { method: "POST" });

export const createTrack = (conn: ConnectionConfig, taskId: string, label?: string, opts?: { env_target?: "worktree" | "local" }) =>
  fetchJson<Track>(conn, `/api/tasks/${taskId}/tracks`, {
    method: "POST",
    body: JSON.stringify({
      label,
      ...(opts?.env_target ? { env_target: opts.env_target } : {}),
    }),
  });

export const createSession = (conn: ConnectionConfig, trackId: string, provider_id: string, model_id: string) =>
  fetchJson<Session>(conn, `/api/tracks/${trackId}/sessions`, {
    method: "POST",
    body: JSON.stringify({ provider_id, model_id }),
  });

export const getSessionHead = (conn: ConnectionConfig, sessionId: string, limit?: number) => {
  const qs = new URLSearchParams();
  if (limit) qs.set("limit", String(limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return fetchJson<SessionHead>(conn, `/api/sessions/${sessionId}/head${suffix}`);
};

export const getSessionHistory = (conn: ConnectionConfig, sessionId: string, beforeSeq?: number, limit?: number) => {
  const qs = new URLSearchParams();
  if (typeof beforeSeq === "number") qs.set("before_seq", String(beforeSeq));
  if (limit) qs.set("limit", String(limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return fetchJson<SessionHistoryPage>(conn, `/api/sessions/${sessionId}/history${suffix}`);
};

export const listTurnTools = (conn: ConnectionConfig, sessionId: string, turnId: string) =>
  fetchJson<SessionTurnTool[]>(conn, `/api/sessions/${sessionId}/turns/${turnId}/tools`);

export type ProviderOptions = {
  provider_id: string;
  workspace_id: string;
  installed?: boolean;
  probe_ok?: boolean;
  probe_error?: string;
  supports_load: boolean;
  auth_required: boolean;
  auth_methods?: any;
  modes?: any;
  models?: any;
  acp_error?: any;
  verify?: any;
  probed_at: string;
};

export const getProviderOptions = (conn: ConnectionConfig, workspaceId: string, providerId: string) =>
  fetchJson<ProviderOptions>(conn, `/api/workspaces/${workspaceId}/providers/${providerId}/options`);
