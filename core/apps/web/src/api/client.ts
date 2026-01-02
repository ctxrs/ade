import type {
  AttachmentMode,
  AttachmentUpdatePolicy,
  Diagnostics,
  Message,
  MessageAttachment,
  MobileConnectionProfile,
  MobileDeviceRegistration,
  ProviderStatus,
  ResourceUtilization,
  Session,
  SessionEvent,
  SessionTurn,
  SessionTurnTool,
  SessionTurnToolSummary,
  SessionSummary,
  SessionHead,
  SessionHeadDelta,
  SessionHistoryPage,
  SessionEventsPage,
  SessionCatchupSummary,
  TerminalSession,
  Task,
  Track,
  TrackDiffSummary,
  TrackDiffSummaryResponse,
  TrackSummary,
  Workspace,
  WorkspaceCatchupCursor,
  WorkspaceCatchupClientMessage,
  WorkspaceCatchupSessionSubscription,
  WorkspaceCatchupEvent,
  WorkspaceCatchupSnapshot,
  WorkspaceCatchupTaskSummary,
  WorkspaceCatchupTrackSummary,
  Worktree,
  WorkspaceAttachment,
  WorkspaceAttachmentKind,
} from "@ctx/types";
import { desktopDaemonRequest, desktopUploadBlob, isDesktopApp } from "../utils/desktop";

export type {
  AttachmentMode,
  AttachmentUpdatePolicy,
  Diagnostics,
  Message,
  MessageAttachment,
  MobileConnectionProfile,
  MobileDeviceRegistration,
  ProviderStatus,
  ResourceUtilization,
  Session,
  SessionEvent,
  SessionTurn,
  SessionTurnTool,
  SessionTurnToolSummary,
  SessionSummary,
  SessionHead,
  SessionHeadDelta,
  SessionHistoryPage,
  SessionEventsPage,
  SessionCatchupSummary,
  TerminalSession,
  Task,
  Track,
  TrackDiffSummary,
  TrackDiffSummaryResponse,
  TrackSummary,
  Workspace,
  WorkspaceCatchupCursor,
  WorkspaceCatchupClientMessage,
  WorkspaceCatchupSessionSubscription,
  WorkspaceCatchupEvent,
  WorkspaceCatchupSnapshot,
  WorkspaceCatchupTaskSummary,
  WorkspaceCatchupTrackSummary,
  Worktree,
  WorkspaceAttachment,
  WorkspaceAttachmentKind,
} from "@ctx/types";

export type BlobUploadResp = {
  blob_id: string;
  sha256: string;
  bytes: number;
  mime_type: string;
  name?: string | null;
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

export type TelemetrySettings = {
  enabled: boolean;
  endpoint: string;
};

export type ResourceGovernanceStatusState = "disabled" | "applied" | "pending" | "unsupported" | "error";

export type ResourceGovernanceStatus = {
  state: ResourceGovernanceStatusState;
  can_apply_now: boolean;
  requires_restart: boolean;
  message?: string | null;
};

export type ResourceGovernanceLimits = {
  cpu_quota_pct: number;
  memory_high_mb: number;
  memory_max_mb: number;
};

export type ResourceGovernanceSettings = {
  enabled: boolean;
  mode: "auto" | "custom";
  cpu_quota_pct?: number | null;
  memory_high_mb?: number | null;
  memory_max_mb?: number | null;
  effective?: ResourceGovernanceLimits | null;
  status?: ResourceGovernanceStatus | null;
};

export type TitleGenerationSettings = {
  base_url: string;
  api_key: string;
  model: string;
  use_json: boolean;
};

export type Settings = {
  dictation?: DictationSettings | null;
  telemetry?: TelemetrySettings | null;
  title_generation?: TitleGenerationSettings | null;
  resource_governance?: ResourceGovernanceSettings | null;
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
    return sessionStorage.getItem("ctxAuthToken");
  } catch {
    return null;
  }
};

export const getDaemonBaseUrl = (): string | null => {
  try {
    return sessionStorage.getItem("contextDaemonBaseUrl") || localStorage.getItem("contextDaemonBaseUrl");
  } catch {
    return null;
  }
};

export const setDaemonBaseUrl = (baseUrl: string | null, persist?: boolean) => {
  try {
    if (!baseUrl) {
      sessionStorage.removeItem("contextDaemonBaseUrl");
      if (persist) localStorage.removeItem("contextDaemonBaseUrl");
      return;
    }
    sessionStorage.setItem("contextDaemonBaseUrl", baseUrl);
    if (persist) localStorage.setItem("contextDaemonBaseUrl", baseUrl);
  } catch {
    // ignore
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
        "Cannot reach the ctx daemon via /api. If you're running the web dev server, start the daemon (default http://127.0.0.1:4399) or set CTX_DAEMON_URL before `pnpm dev`.",
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

const desktopApi = async <T>(path: string, init?: RequestInit): Promise<T> => {
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

  const method = init?.method ? String(init.method) : "GET";
  const body =
    init?.body === undefined || init?.body === null
      ? null
      : typeof init.body === "string"
        ? init.body
        : String(init.body);

  const resp = await desktopDaemonRequest({
    method,
    path,
    body,
    headers: Object.entries({
      "content-type": "application/json",
      ...extraHeaders,
    }),
  });

  const contentType = String(resp.content_type ?? "");
  const text = String(resp.body ?? "");

  const looksLikeHtml = (t: string): boolean => {
    const s = String(t || "").trimStart().toLowerCase();
    return s.startsWith("<!doctype html") || s.startsWith("<html");
  };

  const trimForError = (t: string): string => {
    const s = String(t || "").trim();
    if (s.length <= 800) return s;
    return `${s.slice(0, 800)}…`;
  };

  const ok = resp.status >= 200 && resp.status < 300;
  if (!ok) {
    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
      throw new Error(
        `The daemon returned HTML for ${path} (${resp.status}). Restart/update the daemon.`,
      );
    }
    const lowered = String(text || "").toLowerCase();
    if (
      resp.status >= 500 &&
      (lowered.includes("econnrefused") ||
        lowered.includes("proxy error") ||
        lowered.includes("connect econnrefused") ||
        lowered.includes("socket hang up"))
    ) {
      throw new Error("Cannot reach the ctx daemon. Connect to a host from the launcher first.");
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
    throw new Error(trimForError(text) || `${resp.status}`);
  }

  if (resp.status === 204) {
    return undefined as T;
  }
  if (!text) return undefined as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
      throw new Error(`The daemon returned HTML for ${path}. Restart/update the daemon.`);
    }
    throw new Error(`Unexpected non-JSON response from ${path}.`);
  }
};

const apiAny = async <T>(path: string, init?: RequestInit): Promise<T> => {
  if (isDesktopApp()) return desktopApi<T>(path, init);
  return api<T>(path, init);
};

export type DaemonRawResponse = {
  status: number;
  body: string;
  content_type: string;
};

// For endpoints that need to handle non-2xx statuses without throwing (e.g. buffers update conflict 409).
export const daemonFetchRaw = async (path: string, init?: RequestInit): Promise<DaemonRawResponse> => {
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

  const method = init?.method ? String(init.method) : "GET";
  const body =
    init?.body === undefined || init?.body === null
      ? null
      : typeof init.body === "string"
        ? init.body
        : String(init.body);

  if (isDesktopApp()) {
    const resp = await desktopDaemonRequest({
      method,
      path,
      body,
      headers: Object.entries({
        "content-type": "application/json",
        ...extraHeaders,
      }),
    });
    return {
      status: resp.status,
      body: String(resp.body ?? ""),
      content_type: String(resp.content_type ?? ""),
    };
  }

  const res = await fetch(path, {
    headers: {
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      ...extraHeaders,
    },
    ...init,
  });
  const text = await res.text();
  return {
    status: res.status,
    body: text,
    content_type: res.headers.get("content-type") ?? "",
  };
};

const idToString = (id: any): string =>
  typeof id === "string" ? id : id?.["0"];

export const listWorkspaces = () =>
  apiAny<Workspace[]>("/api/workspaces");

export const getSettings = () =>
  apiAny<Settings>("/api/settings");

export const updateSettings = (settings: Settings) =>
  apiAny<Settings>("/api/settings", {
    method: "POST",
    body: JSON.stringify(settings),
  });

export const createWorkspace = (root_path: string, name?: string) =>
  apiAny<Workspace>("/api/workspaces", {
    method: "POST",
    body: JSON.stringify({ root_path, name }),
  });

export const getWorkspace = (id: string) =>
  apiAny<Workspace>(`/api/workspaces/${id}`);

export type CreateWorkspaceAttachmentRequest = {
  kind: WorkspaceAttachmentKind;
  name: string;
  source: string;
  revision?: string | null;
  subpath?: string | null;
  mount_relpath?: string | null;
  mode?: AttachmentMode | null;
  update_policy?: AttachmentUpdatePolicy | null;
};

export type DeleteWorkspaceAttachmentRequest = {
  kind: WorkspaceAttachmentKind;
  name: string;
};

export const listWorkspaceAttachments = (workspaceId: string) =>
  apiAny<WorkspaceAttachment[]>(`/api/workspaces/${workspaceId}/attachments`);

export const createWorkspaceAttachment = (workspaceId: string, req: CreateWorkspaceAttachmentRequest) =>
  apiAny<WorkspaceAttachment[]>(`/api/workspaces/${workspaceId}/attachments`, {
    method: "POST",
    body: JSON.stringify(req),
  });

export const deleteWorkspaceAttachment = (workspaceId: string, req: DeleteWorkspaceAttachmentRequest) =>
  apiAny<WorkspaceAttachment[]>(`/api/workspaces/${workspaceId}/attachments`, {
    method: "DELETE",
    body: JSON.stringify(req),
  });

export const syncWorkspaceAttachments = (workspaceId: string, refresh?: boolean) =>
  apiAny<WorkspaceAttachment[]>(`/api/workspaces/${workspaceId}/attachments/sync`, {
    method: "POST",
    body: JSON.stringify({ refresh }),
  });

export type CreateTerminalRequest = {
  task_id?: string | null;
  track_id?: string | null;
  session_id?: string | null;
  worktree_id?: string | null;
  cwd?: string | null;
  shell?: string | null;
};

export const listWorkspaceTerminals = (workspaceId: string) =>
  apiAny<TerminalSession[]>(`/api/workspaces/${workspaceId}/terminals`);

export const createWorkspaceTerminal = (workspaceId: string, req: CreateTerminalRequest) =>
  apiAny<TerminalSession>(`/api/workspaces/${workspaceId}/terminals`, {
    method: "POST",
    body: JSON.stringify(req),
  });

export const deleteTerminal = (terminalId: string) =>
  apiAny<void>(`/api/terminals/${terminalId}`, { method: "DELETE" });

export type WorkspaceCatchupParams = {
  limit?: number;
  includeArchived?: boolean;
  archivedOnly?: boolean;
  activeCursor?: WorkspaceCatchupCursor | null;
  archivedCursor?: WorkspaceCatchupCursor | null;
};

export const getWorkspaceCatchup = (workspaceId: string, params?: WorkspaceCatchupParams) => {
  const search = new URLSearchParams();
  if (params?.limit) search.set("limit", String(params.limit));
  if (params?.includeArchived) search.set("include_archived", params.includeArchived ? "1" : "0");
  if (params?.archivedOnly) search.set("archived_only", params.archivedOnly ? "1" : "0");
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
  return apiAny<WorkspaceCatchupSnapshot>(`/api/workspaces/${workspaceId}/catchup${suffix}`);
};

export const createTask = (
  workspaceId: string,
  title: string,
  description?: string,
  opts?: { create_default_track?: boolean; default_track_label?: string },
) =>
  apiAny<Task>(`/api/workspaces/${workspaceId}/tasks`, {
    method: "POST",
    body: JSON.stringify({
      title,
      description,
      ...(opts?.create_default_track === undefined ? {} : { create_default_track: opts.create_default_track }),
      ...(opts?.default_track_label === undefined ? {} : { default_track_label: opts.default_track_label }),
    }),
  });


export const updateTaskTitle = (taskId: string, title: string) =>
  apiAny<Task>(`/api/tasks/${taskId}/title`, { method: "POST", body: JSON.stringify({ title }) });

export const deleteTask = (taskId: string) =>
  apiAny<void>(`/api/tasks/${taskId}`, { method: "DELETE" });

export const archiveTask = (taskId: string) =>
  apiAny<Task>(`/api/tasks/${taskId}/archive`, { method: "POST" });

export const unarchiveTask = (taskId: string) =>
  apiAny<Task>(`/api/tasks/${taskId}/unarchive`, { method: "POST" });

export const markTaskRead = (taskId: string) =>
  apiAny<Task>(`/api/tasks/${taskId}/mark_read`, { method: "POST" });

export const markTaskUnread = (taskId: string) =>
  apiAny<Task>(`/api/tasks/${taskId}/mark_unread`, { method: "POST" });

export const createTrack = (taskId: string, label?: string, opts?: { env_target?: "worktree" | "local" }) =>
  apiAny<Track>(`/api/tasks/${taskId}/tracks`, {
    method: "POST",
    body: JSON.stringify({
      label,
      ...(opts?.env_target ? { env_target: opts.env_target } : {}),
    }),
  });

export const createSession = (trackId: string, provider_id: string, model_id: string) =>
  apiAny<Session>(`/api/tracks/${trackId}/sessions`, {
    method: "POST",
    body: JSON.stringify({ provider_id, model_id }),
  });


export const getWorktree = (worktreeId: string) =>
  apiAny<Worktree>(`/api/worktrees/${worktreeId}`);

export const getWorktreeBootstrapLogs = async (worktreeId: string): Promise<string> => {
  const resp = await daemonFetchRaw(`/api/worktrees/${worktreeId}/bootstrap/logs`);
  if (resp.status >= 400) {
    const msg = String(resp.body || "").trim();
    throw new Error(msg || `Failed to download logs (${resp.status}).`);
  }
  return resp.body ?? "";
};

export const getSessionHead = (sessionId: string, limit?: number, includeEvents?: boolean) => {
  const qs = new URLSearchParams();
  if (limit) qs.set("limit", String(limit));
  if (includeEvents !== undefined) qs.set("include_events", includeEvents ? "1" : "0");
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return apiAny<SessionHead>(`/api/sessions/${sessionId}/head${suffix}`);
};

export const getSessionEvents = (
  sessionId: string,
  opts?: { afterSeq?: number; limit?: number; tail?: number },
) => {
  const qs = new URLSearchParams();
  if (typeof opts?.afterSeq === "number") qs.set("after_seq", String(opts.afterSeq));
  if (opts?.limit) qs.set("limit", String(opts.limit));
  if (opts?.tail) qs.set("tail", String(opts.tail));
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return apiAny<SessionEventsPage>(`/api/sessions/${sessionId}/events${suffix}`);
};

export const getSessionHistory = (sessionId: string, beforeSeq?: number, limit?: number) => {
  const qs = new URLSearchParams();
  if (typeof beforeSeq === "number") qs.set("before_seq", String(beforeSeq));
  if (limit) qs.set("limit", String(limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return apiAny<SessionHistoryPage>(`/api/sessions/${sessionId}/history${suffix}`);
};

export const listTurnTools = (sessionId: string, turnId: string) =>
  apiAny<SessionTurnTool[]>(`/api/sessions/${sessionId}/turns/${turnId}/tools`);

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
  return apiAny<string[]>(`/api/sessions/${sessionId}/completions/files${suffix}`, { signal });
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
  return apiAny<string[]>(`/api/workspaces/${workspaceId}/completions/files${suffix}`, { signal });
};

export const postMessage = (
  sessionId: string,
  content: string,
  delivery?: "immediate" | "queued",
  attachments?: MessageAttachment[],
) =>
  apiAny<Message>(`/api/sessions/${sessionId}/messages`, {
    method: "POST",
    body: JSON.stringify({ content, delivery, attachments: attachments ?? [] }),
  });

export const uploadBlob = async (file: File): Promise<BlobUploadResp> => {
  if (isDesktopApp()) {
    const buf = await file.arrayBuffer();
    const bytes = Array.from(new Uint8Array(buf));
    const resp = await desktopUploadBlob({
      bytes,
      mime_type: file.type || "application/octet-stream",
      name: file.name,
    });
    return resp as BlobUploadResp;
  }
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
  apiAny(`/api/sessions/${sessionId}/cancel`, { method: "POST" });

export const interruptSession = (sessionId: string) =>
  apiAny(`/api/sessions/${sessionId}/interrupt`, { method: "POST" });

export const setSessionModel = (sessionId: string, model_id: string) =>
  apiAny<Session>(`/api/sessions/${sessionId}/model`, {
    method: "POST",
    body: JSON.stringify({ model_id }),
  });

export const setSessionMode = (sessionId: string, mode_id: string) =>
  apiAny(`/api/sessions/${sessionId}/mode`, {
    method: "POST",
    body: JSON.stringify({ mode_id }),
  });

export const authenticateSession = (sessionId: string, method_id?: string) =>
  apiAny(`/api/sessions/${sessionId}/authenticate`, {
    method: "POST",
    body: JSON.stringify(method_id ? { method_id } : {}),
  });

export type AskUserQuestionOutcome = "submitted" | "cancelled";

export const submitAskUserQuestion = (
  sessionId: string,
  tool_call_id: string,
  outcome: AskUserQuestionOutcome,
  answers?: Record<string, string>,
) =>
  apiAny(`/api/sessions/${sessionId}/ask_user_question`, {
    method: "POST",
    body: JSON.stringify({ tool_call_id, outcome, answers }),
  });

export const trackDiff = (trackId: string) =>
  apiAny<{ diff: string }>(`/api/tracks/${trackId}/diff`);

export const getTrackDiffSummary = (trackId: string) =>
  apiAny<TrackDiffSummaryResponse>(`/api/tracks/${trackId}/diff_summary`);

export const applyTrackDiffPatch = (trackId: string, action: "accept" | "reject", patch: string) =>
  apiAny<{ diff: string }>(`/api/tracks/${trackId}/diff/apply`, {
    method: "POST",
    body: JSON.stringify({ action, patch }),
  });

export const listEditPlansForTrack = (trackId: string) =>
  apiAny<EditPlanSummary[]>(`/api/tracks/${trackId}/edit_plans`);

export const getEditPlan = (planId: string) =>
  apiAny<EditPlanSummary>(`/api/edit_plans/${planId}`);

export const applyEditPlanPatch = (planId: string, action: "accept" | "reject", patch: string) =>
  apiAny<EditPlanSummary>(`/api/edit_plans/${planId}/apply`, {
    method: "POST",
    body: JSON.stringify({ action, patch }),
  });

export const discardEditPlan = (planId: string) =>
  apiAny<void>(`/api/edit_plans/${planId}/discard`, {
    method: "POST",
    body: JSON.stringify({}),
  });


export const deleteMessage = (messageId: string) =>
  apiAny(`/api/messages/${messageId}`, { method: "DELETE" });

export const listProviders = () =>
  apiAny<ProviderStatus[]>(`/api/providers`);

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

export const getProviderOptions = (workspaceId: string, providerId: string) =>
  apiAny<ProviderOptions>(`/api/workspaces/${workspaceId}/providers/${providerId}/options`);

export type ProviderAuthCheck = {
  provider_id: string;
  workspace_id: string;
  status: string;
  auth_required?: boolean;
  auth_methods?: any;
  acp_error?: any;
  checked_at?: string;
};

export const authenticateProviderForWorkspace = (workspaceId: string, providerId: string, method_id?: string) =>
  apiAny<ProviderAuthCheck>(`/api/workspaces/${workspaceId}/providers/${providerId}/authenticate`, {
    method: "POST",
    body: JSON.stringify(method_id ? { method_id } : {}),
  });

export const verifyProviderForWorkspace = (workspaceId: string, providerId: string) =>
  apiAny<ProviderAuthCheck>(`/api/workspaces/${workspaceId}/providers/${providerId}/verify`, { method: "POST" });

export const installProvider = (providerId: string) =>
  apiAny<InstallStartResponse>(`/api/providers/${providerId}/install`, { method: "POST" });

export const installAllProviders = () =>
  apiAny<InstallStartResponse[]>(`/api/providers/install_all`, { method: "POST" });

export const getInstall = (installId: string) =>
  apiAny<InstallInfo>(`/api/providers/install/${installId}`);

export const listInstallEvents = (installId: string) =>
  apiAny<InstallProgressEvent[]>(`/api/providers/install/${installId}/events`);

export const installStreamUrl = (installId: string): string => {
  const token = authToken();
  return token
    ? `/api/providers/install/${installId}/stream?token=${encodeURIComponent(token)}`
    : `/api/providers/install/${installId}/stream`;
};

export const getDiagnostics = () => apiAny<Diagnostics>(`/api/diagnostics`);

export const getResourceUtilization = (workspaceId: string) =>
  apiAny<ResourceUtilization>(`/api/resource_utilization?workspace_id=${encodeURIComponent(workspaceId)}`);

export const getHealth = () => apiAny<Health>(`/api/health`);

export const getLspStatus = () => apiAny<LspStatus>(`/api/lsp/status`);

export const openLogsFolder = () => apiAny(`/api/logs/open`, { method: "POST" });

export const appendDesktopLog = (message: string, level?: string) =>
  apiAny(`/api/desktop/log`, {
    method: "POST",
    body: JSON.stringify({ message, level }),
  });

export const checkUpdates = (channel?: string) =>
  apiAny<UpdateCheck>(`/api/updates/check${channel ? `?channel=${encodeURIComponent(channel)}` : ""}`);

export const downloadAppImageUpdate = (channel?: string) =>
  apiAny<DownloadAppImageUpdateResp>(`/api/updates/appimage/download`, {
    method: "POST",
    body: JSON.stringify(channel ? { channel } : {}),
  });

export const applyAppImageUpdate = () =>
  apiAny<ApplyAppImageUpdateResp>(`/api/updates/appimage/apply`, {
    method: "POST",
    body: JSON.stringify({ confirm: true }),
  });

export const blobUrl = (blobId: string): string => {
  const base = getDaemonBaseUrl();
  const token = authToken();
  const prefix = base ? base.replace(/\/+$/, "") : "";
  const url = `${prefix}/api/blobs/${encodeURIComponent(String(blobId || ""))}`;
  return token ? `${url}?token=${encodeURIComponent(token)}` : url;
};

export type CreateMobileProfileRequest = {
  label: string;
  base_url: string;
  scopes?: string[];
};

export type CreateMobileProfileResponse = {
  profile: MobileConnectionProfile;
  token: string;
  qr_payload: any;
};

export type MobileTunnelState = "idle" | "running" | "error";

export type MobileAccessStatus = {
  enabled: boolean;
  tunnel_id?: string | null;
  public_base_url?: string | null;
  relay_base_url?: string | null;
  daemon_public_key?: string | null;
  tunnel_state: MobileTunnelState;
  last_error?: string | null;
};

export type EnableMobileAccessResponse = {
  status: MobileAccessStatus;
  qr_payload: any;
  pairing_expires_at: string;
};

export const listMobileConnectionProfiles = () =>
  api<MobileConnectionProfile[]>(`/api/mobile/connection_profiles`);

export const createMobileConnectionProfile = (payload: CreateMobileProfileRequest) =>
  api<CreateMobileProfileResponse>(`/api/mobile/connection_profiles`, {
    method: "POST",
    body: JSON.stringify(payload),
  });

export const deleteMobileConnectionProfile = (id: string) =>
  api<void>(`/api/mobile/connection_profiles/${id}`, { method: "DELETE" });

export const listMobileDevicesForProfile = (profileId: string) =>
  api<MobileDeviceRegistration[]>(`/api/mobile/connection_profiles/${profileId}/devices`);

export const getMobileAccessStatus = () => api<MobileAccessStatus>(`/api/mobile/access/status`);

export const enableMobileAccess = (supabase_token: string) =>
  api<EnableMobileAccessResponse>(`/api/mobile/access/enable`, {
    method: "POST",
    body: JSON.stringify({ supabase_token }),
  });

export const disableMobileAccess = (supabase_token: string) =>
  api<void>(`/api/mobile/access/disable`, {
    method: "POST",
    body: JSON.stringify({ supabase_token }),
  });

export { idToString };
