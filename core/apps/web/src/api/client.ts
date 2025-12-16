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
  env_target?: "worktree" | "local" | string;
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
    }
  | {
      kind: "image_ref";
      blob_id: string;
      mime_type: string;
      name?: string | null;
    };

export type BlobUploadResp = {
  blob_id: string;
  sha256: string;
  bytes: number;
  mime_type: string;
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
  details?: Record<string, string>;
};

export type InstallEventLevel = "info" | "warning" | "error" | "success";

export type InstallProgressEvent = {
  install_id: string;
  provider_id: string;
  at: string;
  stage: string;
  message: string;
  level: InstallEventLevel;
  bytes?: number;
  total_bytes?: number;
  attempt?: number;
};

export type InstallInfo = {
  install_id: string;
  provider_id: string;
  state: "running" | "succeeded" | "failed";
  started_at: string;
  finished_at?: string;
  error?: string;
  last_event?: InstallProgressEvent;
};

export type InstallStartResponse = {
  provider_id: string;
  install_id: string;
};

export type LogFileInfo = {
  name: string;
  bytes: number;
  modified_utc?: string | null;
};

export type Diagnostics = {
  daemon: {
    version: string;
    pid: number;
    data_root: string;
    daemon_url: string;
    auth_required: boolean;
  };
  platform: { os: string; arch: string };
  logs: { dir: string; files: LogFileInfo[] };
  providers: ProviderStatus[];
  managed_installs: any;
};

export type Health = {
  version: string;
  pid: number;
  data_root: string;
  daemon_url: string;
  auth_required: boolean;
};

export type UpdateCheck = {
  channel: string;
  base_url: string;
  platform?: string | null;
  current_version: string;
  latest_version?: string | null;
  update_available: boolean;
  manifest?: any;
};

export type DownloadAppImageUpdateResp = {
  downloaded_path: string;
  can_apply_in_place: boolean;
};

export type ApplyAppImageUpdateResp = {
  applied: boolean;
  target_path?: string | null;
  message: string;
};

export type LspServerStatus = {
  language: string;
  command: string;
  args: string[];
  found: boolean;
  resolved_path?: string | null;
  version?: string | null;
  install_hints: string[];
};

export type LspStatus = {
  enabled: boolean;
  edit_plans_enabled: boolean;
  servers: LspServerStatus[];
};

export type LiveKitDictationSettings = {
  base_url: string;
  api_key: string;
  api_secret?: string | null;
  api_secret_set?: boolean;
  model?: string | null;
  language?: string | null;
};

export type DictationSettings = {
  enabled: boolean;
  provider: "disabled" | "livekit_inference";
  livekit?: LiveKitDictationSettings | null;
};

export type Settings = {
  dictation?: DictationSettings | null;
};

export type EditPlanSummary = {
  id: { 0: string } | string;
  title: string;
  created_at: string;
  remaining_files: number;
  remaining_hunks: number;
  diff: string;
};

const authToken = (): string | null => {
  try {
    return sessionStorage.getItem("contextAuthToken");
  } catch {
    return null;
  }
};

const api = async <T>(path: string, init?: RequestInit): Promise<T> => {
  const token = authToken();
  const extraHeaders: Record<string, string> = {};
  if (init?.headers) {
    if (init.headers instanceof Headers) {
      init.headers.forEach((value, key) => {
        extraHeaders[key] = value;
      });
    } else if (Array.isArray(init.headers)) {
      for (const [key, value] of init.headers) {
        extraHeaders[key] = value;
      }
    } else {
      Object.assign(extraHeaders, init.headers as any);
    }
  }
  const res = await fetch(path, {
    headers: {
      "content-type": "application/json",
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      ...extraHeaders,
    },
    ...init,
  });

  const looksLikeHtml = (text: string): boolean => {
    const t = String(text || "").trimStart().toLowerCase();
    return t.startsWith("<!doctype html") || t.startsWith("<html");
  };

  const trimForError = (text: string): string => {
    const s = String(text || "").trim();
    if (s.length <= 800) return s;
    return `${s.slice(0, 800)}…`;
  };

  if (!res.ok) {
    const text = await res.text();
    const contentType = res.headers.get("content-type") ?? "";

    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
      // This usually means the web UI server served its SPA fallback for an /api route.
      // Most commonly: the daemon is old and doesn't implement the endpoint, or the dev proxy isn't pointing at the daemon.
      throw new Error(
        `The daemon returned HTML for ${path} (${res.status}). Restart/update the daemon (and ensure Vite is proxying /api to it).`,
      );
    }

    const lowered = String(text || "").toLowerCase();
    if (
      res.status >= 500 &&
      (lowered.includes("econnrefused") ||
        lowered.includes("proxy error") ||
        lowered.includes("connect econnrefused") ||
        lowered.includes("socket hang up"))
    ) {
      throw new Error(
        "Cannot reach the Context daemon via /api. If you're running the web dev server, start the daemon (default http://127.0.0.1:4399) or set CONTEXT_DAEMON_URL before `pnpm dev`.",
      );
    }
    try {
      const parsed = text ? JSON.parse(text) : null;
      const msg = parsed?.error ?? parsed?.message;
      if (typeof msg === "string" && msg.length > 0) {
        throw new Error(msg);
      }
    } catch {
      // ignore
    }
    throw new Error(trimForError(text) || `${res.status} ${res.statusText}`);
  }
  if (res.status === 204) {
    return undefined as T;
  }
  const text = await res.text();
  if (!text) return undefined as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    const contentType = res.headers.get("content-type") ?? "";
    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
      throw new Error(
        `The daemon returned HTML for ${path}. Restart/update the daemon (and ensure Vite is proxying /api to it).`,
      );
    }
    throw new Error(`Unexpected non-JSON response from ${path}.`);
  }
};

const idToString = (id: any): string =>
  typeof id === "string" ? id : id?.["0"];

export const listWorkspaces = () =>
  api<Workspace[]>("/api/workspaces");

export const getSettings = () =>
  api<Settings>("/api/settings");

export const updateSettings = (settings: Settings) =>
  api<Settings>("/api/settings", {
    method: "POST",
    body: JSON.stringify(settings),
  });

export const createWorkspace = (root_path: string, name?: string) =>
  api<Workspace>("/api/workspaces", {
    method: "POST",
    body: JSON.stringify({ root_path, name }),
  });

export const getWorkspace = (id: string) =>
  api<Workspace>(`/api/workspaces/${id}`);

export const listTasks = (workspaceId: string) =>
  api<Task[]>(`/api/workspaces/${workspaceId}/tasks`);

export const createTask = (
  workspaceId: string,
  title: string,
  description?: string,
  opts?: { create_default_track?: boolean; default_track_label?: string },
) =>
  api<Task>(`/api/workspaces/${workspaceId}/tasks`, {
    method: "POST",
    body: JSON.stringify({
      title,
      description,
      ...(opts?.create_default_track === undefined ? {} : { create_default_track: opts.create_default_track }),
      ...(opts?.default_track_label === undefined ? {} : { default_track_label: opts.default_track_label }),
    }),
  });

export const getTask = (taskId: string) =>
  api<Task>(`/api/tasks/${taskId}`);

export const listTracks = (taskId: string) =>
  api<Track[]>(`/api/tasks/${taskId}/tracks`);

export const createTrack = (taskId: string, label?: string, opts?: { env_target?: "worktree" | "local" }) =>
  api<Track>(`/api/tasks/${taskId}/tracks`, {
    method: "POST",
    body: JSON.stringify({
      label,
      ...(opts?.env_target ? { env_target: opts.env_target } : {}),
    }),
  });

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

export const listSessionEventsPage = (sessionId: string, after?: string, limit?: number) => {
  const qs = new URLSearchParams();
  if (after) qs.set("after", after);
  if (limit) qs.set("limit", String(limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return api<SessionEvent[]>(`/api/sessions/${sessionId}/events${suffix}`);
};

export const listSessionFileCompletions = (
  sessionId: string,
  query: string,
  limit?: number,
  signal?: AbortSignal,
) => {
  const qs = new URLSearchParams();
  qs.set("query", query);
  if (limit) qs.set("limit", String(limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return api<string[]>(`/api/sessions/${sessionId}/completions/files${suffix}`, { signal });
};

export const listWorkspaceFileCompletions = (
  workspaceId: string,
  query: string,
  limit?: number,
  signal?: AbortSignal,
) => {
  const qs = new URLSearchParams();
  qs.set("query", query);
  if (limit) qs.set("limit", String(limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return api<string[]>(`/api/workspaces/${workspaceId}/completions/files${suffix}`, { signal });
};

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

export const uploadBlob = async (file: File): Promise<BlobUploadResp> => {
  const token = authToken();
  const form = new FormData();
  form.append("file", file, file.name);
  const res = await fetch("/api/blobs", {
    method: "POST",
    headers: {
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: form,
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `${res.status} ${res.statusText}`);
  }
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as BlobUploadResp;
};

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

export const authenticateSession = (sessionId: string, method_id?: string) =>
  api(`/api/sessions/${sessionId}/authenticate`, {
    method: "POST",
    body: JSON.stringify(method_id ? { method_id } : {}),
  });

export const trackDiff = (trackId: string) =>
  api<{ diff: string }>(`/api/tracks/${trackId}/diff`);

export const applyTrackDiffPatch = (trackId: string, action: "accept" | "reject", patch: string) =>
  api<{ diff: string }>(`/api/tracks/${trackId}/diff/apply`, {
    method: "POST",
    body: JSON.stringify({ action, patch }),
  });

export const listEditPlansForTrack = (trackId: string) =>
  api<EditPlanSummary[]>(`/api/tracks/${trackId}/edit_plans`);

export const getEditPlan = (planId: string) =>
  api<EditPlanSummary>(`/api/edit_plans/${planId}`);

export const applyEditPlanPatch = (planId: string, action: "accept" | "reject", patch: string) =>
  api<EditPlanSummary>(`/api/edit_plans/${planId}/apply`, {
    method: "POST",
    body: JSON.stringify({ action, patch }),
  });

export const discardEditPlan = (planId: string) =>
  api<void>(`/api/edit_plans/${planId}/discard`, {
    method: "POST",
    body: JSON.stringify({}),
  });

export const listQueue = (sessionId: string) =>
  api<Message[]>(`/api/sessions/${sessionId}/queue`);

export const deleteMessage = (messageId: string) =>
  api(`/api/messages/${messageId}`, { method: "DELETE" });

export const listProviders = () =>
  api<ProviderStatus[]>(`/api/providers`);

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
  probed_at: string;
};

export const getProviderOptions = (workspaceId: string, providerId: string) =>
  api<ProviderOptions>(`/api/workspaces/${workspaceId}/providers/${providerId}/options`);

export const installProvider = (providerId: string) =>
  api<InstallStartResponse>(`/api/providers/${providerId}/install`, { method: "POST" });

export const installAllProviders = () =>
  api<InstallStartResponse[]>(`/api/providers/install_all`, { method: "POST" });

export const getInstall = (installId: string) =>
  api<InstallInfo>(`/api/providers/install/${installId}`);

export const listInstallEvents = (installId: string) =>
  api<InstallProgressEvent[]>(`/api/providers/install/${installId}/events`);

export const installStreamUrl = (installId: string): string => {
  const token = authToken();
  return token
    ? `/api/providers/install/${installId}/stream?token=${encodeURIComponent(token)}`
    : `/api/providers/install/${installId}/stream`;
};

export const getDiagnostics = () => api<Diagnostics>(`/api/diagnostics`);

export const getHealth = () => api<Health>(`/api/health`);

export const getLspStatus = () => api<LspStatus>(`/api/lsp/status`);

export const openLogsFolder = () => api(`/api/logs/open`, { method: "POST" });

export const appendDesktopLog = (message: string, level?: string) =>
  api(`/api/desktop/log`, {
    method: "POST",
    body: JSON.stringify({ message, level }),
  });

export const checkUpdates = (channel?: string) =>
  api<UpdateCheck>(`/api/updates/check${channel ? `?channel=${encodeURIComponent(channel)}` : ""}`);

export const downloadAppImageUpdate = (channel?: string) =>
  api<DownloadAppImageUpdateResp>(`/api/updates/appimage/download`, {
    method: "POST",
    body: JSON.stringify(channel ? { channel } : {}),
  });

export const applyAppImageUpdate = () =>
  api<ApplyAppImageUpdateResp>(`/api/updates/appimage/apply`, {
    method: "POST",
    body: JSON.stringify({ confirm: true }),
  });

export { idToString };
