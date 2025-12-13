export type Workspace = {
  id: { 0: string } | string;
  name: string;
  root_path: string;
  created_at: string;
};

export type Task = {
  id: { 0: string } | string;
  workspace_id: { 0: string } | string;
  title: string;
  description?: string | null;
  status: string;
  created_at: string;
  updated_at: string;
};

export type Track = {
  id: { 0: string } | string;
  task_id: { 0: string } | string;
  workspace_id: { 0: string } | string;
  worktree_id: { 0: string } | string;
  label: string;
  status: string;
};

export type Session = {
  id: { 0: string } | string;
  track_id: { 0: string } | string;
  task_id: { 0: string } | string;
  workspace_id: { 0: string } | string;
  worktree_id: { 0: string } | string;
  provider_id: string;
  model_id: string;
  agent_role: string;
  status: string;
};

export type Message = {
  id: { 0: string } | string;
  session_id: { 0: string } | string;
  role: "user" | "assistant" | "system";
  content: string;
  attachments?: MessageAttachment[];
  delivery: "immediate" | "queued";
  created_at: string;
};

export type MessageAttachment =
  | {
      kind: "image";
      mime_type: string;
      data_base64: string;
      name?: string | null;
    };

export type SessionEvent = {
  id: { 0: string } | string;
  session_id: { 0: string } | string;
  run_id?: { 0: string } | string | null;
  turn_id?: { 0: string } | string | null;
  event_type: string;
  payload_json: any;
  created_at: string;
};

export type ProviderStatus = {
  provider_id: string;
  installed: boolean;
  detected_path?: string | null;
  version?: string | null;
  health: string;
  diagnostics: string[];
};

const api = async <T>(path: string, init?: RequestInit): Promise<T> => {
  const res = await fetch(path, {
    headers: { "content-type": "application/json" },
    ...init,
  });
  if (!res.ok) {
    throw new Error(`${res.status} ${res.statusText}`);
  }
  if (res.status === 204) {
    return undefined as T;
  }
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
};

const idToString = (id: any): string =>
  typeof id === "string" ? id : id?.["0"];

export const listWorkspaces = () =>
  api<Workspace[]>("/api/workspaces");

export const createWorkspace = (root_path: string, name?: string) =>
  api<Workspace>("/api/workspaces", {
    method: "POST",
    body: JSON.stringify({ root_path, name }),
  });

export const getWorkspace = (id: string) =>
  api<Workspace>(`/api/workspaces/${id}`);

export const listTasks = (workspaceId: string) =>
  api<Task[]>(`/api/workspaces/${workspaceId}/tasks`);

export const createTask = (workspaceId: string, title: string, description?: string) =>
  api<Task>(`/api/workspaces/${workspaceId}/tasks`, {
    method: "POST",
    body: JSON.stringify({ title, description }),
  });

export const getTask = (taskId: string) =>
  api<Task>(`/api/tasks/${taskId}`);

export const listTracks = (taskId: string) =>
  api<Track[]>(`/api/tasks/${taskId}/tracks`);

export const createSession = (trackId: string, provider_id: string, model_id: string) =>
  api<Session>(`/api/tracks/${trackId}/sessions`, {
    method: "POST",
    body: JSON.stringify({ provider_id, model_id }),
  });

export const listSessionsForTrack = (trackId: string) =>
  api<Session[]>(`/api/tracks/${trackId}/sessions`);

export const getSession = (sessionId: string) =>
  api<Session>(`/api/sessions/${sessionId}`);

export const listMessages = (sessionId: string) =>
  api<Message[]>(`/api/sessions/${sessionId}/messages`);

export const listSessionEvents = (sessionId: string) =>
  api<SessionEvent[]>(`/api/sessions/${sessionId}/events`);

export const postMessage = (
  sessionId: string,
  content: string,
  delivery?: "immediate" | "queued",
  attachments?: MessageAttachment[],
) =>
  api<Message>(`/api/sessions/${sessionId}/messages`, {
    method: "POST",
    body: JSON.stringify({ content, delivery, attachments: attachments ?? [] }),
  });

export const cancelSession = (sessionId: string) =>
  api(`/api/sessions/${sessionId}/cancel`, { method: "POST" });

export const interruptSession = (sessionId: string) =>
  api(`/api/sessions/${sessionId}/interrupt`, { method: "POST" });

export const setSessionModel = (sessionId: string, model_id: string) =>
  api<Session>(`/api/sessions/${sessionId}/model`, {
    method: "POST",
    body: JSON.stringify({ model_id }),
  });

export const setSessionMode = (sessionId: string, mode_id: string) =>
  api(`/api/sessions/${sessionId}/mode`, {
    method: "POST",
    body: JSON.stringify({ mode_id }),
  });

export const trackDiff = (trackId: string) =>
  api<{ diff: string }>(`/api/tracks/${trackId}/diff`);

export const listQueue = (sessionId: string) =>
  api<Message[]>(`/api/sessions/${sessionId}/queue`);

export const deleteMessage = (messageId: string) =>
  api(`/api/messages/${messageId}`, { method: "DELETE" });

export const listProviders = () =>
  api<ProviderStatus[]>(`/api/providers`);

export { idToString };
