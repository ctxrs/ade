import type {
  Artifact,
  Message,
  MessageAttachment,
  Session,
  SessionEventsPage,
  SessionHeadSnapshot,
  SessionHistoryPage,
  SessionSnapshot,
  SessionState,
  SessionSummary,
  SessionTurnTool,
  SubagentInvocation,
} from "@ctx/types";
import { apiAny, authToken, resolveDaemonBaseUrl } from "./clientBase";
import { desktopUploadBlob, isDesktopApp } from "../utils/desktop";

export type BlobUploadResp = {
  blob_id: string;
  sha256: string;
  bytes: number;
  mime_type: string;
  name?: string | null;
};

export type WebSessionViewport = {
  width: number;
  height: number;
};

export type WebSessionInfo = {
  id: string;
  kind: string;
  session_id?: string | null;
  worktree_id?: string | null;
  status: string;
  created_at: string;
  updated_at: string;
  last_activity: string;
  url: string;
  viewport: WebSessionViewport;
  fps: number;
  viewers: number;
  stream_path: string;
  stream_url?: string | null;
};

export type EditPlanSummary = {
  id: { 0: string } | string;
  title: string;
  created_at: string;
  remaining_files: number;
  remaining_hunks: number;
  diff: string;
};

export const createSession = (
  taskId: string,
  provider_id: string,
  model_id: string,
  opts?: {
    parent_session_id?: string | null;
    relationship?: string | null;
    env_target?: "worktree" | "local" | "cloud";
    worktree_id?: string | null;
    initial_prompt?: string | null;
  },
) =>
  apiAny<Session>(`/api/tasks/${taskId}/sessions`, {
    method: "POST",
    body: JSON.stringify({
      provider_id,
      model_id,
      ...(opts?.parent_session_id ? { parent_session_id: opts.parent_session_id } : {}),
      ...(opts?.relationship ? { relationship: opts.relationship } : {}),
      ...(opts?.env_target ? { env_target: opts.env_target } : {}),
      ...(opts?.worktree_id ? { worktree_id: opts.worktree_id } : {}),
      ...(opts?.initial_prompt ? { initial_prompt: opts.initial_prompt } : {}),
    }),
  });

export type SessionDiffSummary = {
  base_commit_sha?: string;
  head_commit_sha?: string;
  file_count?: number;
  files?: number;
  line_additions?: number;
  additions?: number;
  line_deletions?: number;
  deletions?: number;
};

export type GitStatusEntry = {
  path: string;
  orig_path?: string | null;
  index_status: string;
  worktree_status: string;
};

export type GitStatusSummary = {
  raw?: string;
  summary_line?: string;
  summaryLine?: string;
  summary?: string;
  status?: string;
  lines?: string[];
  branch?: string | null;
  upstream?: string | null;
  ahead?: number;
  behind?: number;
  detached?: boolean;
  staged?: number;
  unstaged?: number;
  untracked?: number;
  entries?: GitStatusEntry[];
};

export const getSessionGitStatusSummary = (sessionId: string) =>
  apiAny<GitStatusSummary | string>(`/api/sessions/${sessionId}/git/status`);

export const getSessionDiffSummary = (sessionId: string) =>
  apiAny<SessionDiffSummary>(`/api/sessions/${sessionId}/diff/summary`);

export const getSessionSnapshot = (sessionId: string, limit?: number, includeEvents?: boolean) => {
  const qs = new URLSearchParams();
  if (limit) qs.set("limit", String(limit));
  if (includeEvents !== undefined) qs.set("include_events", includeEvents ? "1" : "0");
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return apiAny<SessionSnapshot>(`/api/sessions/${sessionId}/snapshot${suffix}`);
};

export const getSessionHead = (sessionId: string, limit?: number, includeEvents?: boolean) => {
  const qs = new URLSearchParams();
  if (limit) qs.set("limit", String(limit));
  if (includeEvents !== undefined) qs.set("include_events", includeEvents ? "1" : "0");
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return apiAny<SessionHeadSnapshot>(`/api/sessions/${sessionId}/head${suffix}`);
};

export const getSessionState = (sessionId: string) =>
  apiAny<SessionState>(`/api/sessions/${sessionId}/state`);

export type ArtifactInput = {
  absolute_file_path: string;
  name?: string | null;
  mime_type?: string | null;
};

export const listSessionArtifacts = (sessionId: string) =>
  apiAny<Artifact[]>(`/api/sessions/${sessionId}/artifacts`);

export const setSessionArtifacts = (sessionId: string, artifacts: ArtifactInput[]) =>
  apiAny<Artifact[]>(`/api/sessions/${sessionId}/artifacts`, {
    method: "POST",
    body: JSON.stringify({ artifacts }),
  });

export const listSessionSubagents = (sessionId: string) =>
  apiAny<SessionSummary[]>(`/api/sessions/${sessionId}/subagents`);

export const listSessionSubagentInvocations = (sessionId: string, opts?: { turnId?: string }) => {
  const qs = new URLSearchParams();
  if (opts?.turnId) qs.set("turn_id", opts.turnId);
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return apiAny<SubagentInvocation[]>(`/api/sessions/${sessionId}/subagent_invocations${suffix}`);
};

export const getSubagentInvocation = (invocationId: string) =>
  apiAny<SubagentInvocation>(`/api/subagent_invocations/${invocationId}`);

export const listWebSessions = () => apiAny<WebSessionInfo[]>("/api/sessions/web");

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

export const getSessionDiff = (sessionId: string) =>
  apiAny<{ diff: string }>(`/api/sessions/${sessionId}/diff`);

export const applySessionDiffPatch = (sessionId: string, action: "accept" | "reject", patch: string) =>
  apiAny<{ diff: string }>(`/api/sessions/${sessionId}/diff/apply`, {
    method: "POST",
    body: JSON.stringify({ action, patch }),
  });

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

export const blobUrl = (blobId: string): string => {
  const base = resolveDaemonBaseUrl();
  const token = authToken();
  const prefix = base ? base.replace(/\/+$/, "") : "";
  const url = `${prefix}/api/blobs/${encodeURIComponent(String(blobId || ""))}`;
  return token ? `${url}?token=${encodeURIComponent(token)}` : url;
};

export const artifactUrl = (artifactId: string): string => {
  const base = resolveDaemonBaseUrl();
  const token = authToken();
  const prefix = base ? base.replace(/\/+$/, "") : "";
  const url = `${prefix}/api/artifacts/${encodeURIComponent(String(artifactId || ""))}`;
  return token ? `${url}?token=${encodeURIComponent(token)}` : url;
};
