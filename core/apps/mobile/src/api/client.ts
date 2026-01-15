import { Buffer } from "buffer";

import type {
  Diagnostics,
  Message,
  MessageAttachment,
  MobileDeviceRegistration,
  ProviderStatus,
  Session,
  SessionHistoryPage,
  SessionSnapshot,
  SessionTurnTool,
  Task,
  Workspace,
  WorkspaceActiveSnapshot,
} from "@ctx/types";

import type { E2eeEnvelope } from "../utils/e2ee";
import { decodeBase64, decryptPayload, encryptPayload } from "../utils/e2ee";
import { nextSecureSeq } from "../state/secureSeq";
import { getSecureConnectionContext } from "../utils/secureConnection";

export type ConnectionConfig = {
  baseUrl: string;
  token?: string;
  deviceId?: string;
  daemonPublicKey?: string;
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

const buildUrlFromBase = (baseUrl: string, path: string): string => {
  const normalized = ensureSlash(baseUrl);
  const absolute = path.startsWith("/") ? path.slice(1) : path;
  return `${normalized}${absolute}`;
};

const buildUrl = (conn: ConnectionConfig, path: string): string => buildUrlFromBase(conn.baseUrl, path);

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

const mapMessage = (item: Message): MessageSummary => ({
  id: idToString(item.id),
  session_id: idToString(item.session_id),
  role: item.role,
  content: item.content,
  delivery: item.delivery,
  created_at: item.created_at,
});

type SecureRequestPayload = {
  method: string;
  path: string;
  query?: string;
  headers: Array<[string, string]>;
  body_b64: string;
};

type SecureResponsePayload = {
  status: number;
  headers: Array<[string, string]>;
  body_b64: string;
};

const isSecureConnection = (conn: ConnectionConfig): boolean =>
  Boolean(conn.daemonPublicKey && conn.deviceId);

const decodeText = (bytes: Uint8Array): string => {
  if (typeof TextDecoder !== "undefined") {
    return new TextDecoder().decode(bytes);
  }
  return Buffer.from(bytes).toString("utf-8");
};

const encodeBody = async (body?: BodyInit | null): Promise<Uint8Array> => {
  if (body == null) return new Uint8Array();
  if (typeof body === "string") {
    return new Uint8Array(Buffer.from(body, "utf-8"));
  }
  if (body instanceof Uint8Array) {
    return body;
  }
  if (body instanceof ArrayBuffer) {
    return new Uint8Array(body);
  }
  if (ArrayBuffer.isView(body)) {
    return new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
  }
  if (typeof (body as Blob)?.arrayBuffer === "function") {
    const ab = await (body as Blob).arrayBuffer();
    return new Uint8Array(ab);
  }
  throw new Error("Unsupported request body type");
};

async function fetchJson<T>(conn: ConnectionConfig, path: string, init?: RequestInit): Promise<T> {
  if (isSecureConnection(conn)) {
    return secureFetchJson(conn, path, init);
  }
  return legacyFetchJson(conn, path, init);
}

async function legacyFetchJson<T>(conn: ConnectionConfig, path: string, init?: RequestInit): Promise<T> {
  if (!conn.token) {
    throw new Error("Missing API token.");
  }
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

async function secureFetchJson<T>(conn: ConnectionConfig, path: string, init?: RequestInit): Promise<T> {
  if (!conn.deviceId || !conn.daemonPublicKey) {
    throw new Error("Secure connection not configured.");
  }

  const { key, deviceId } = await getSecureConnectionContext(conn.deviceId, conn.daemonPublicKey);
  const normalizedPath = path.startsWith("/") ? path : `/${path}`;
  const [pathOnly, query] = normalizedPath.split("?");
  const method = (init?.method || "GET").toUpperCase();
  const headers = new Headers(init?.headers);
  if (init?.body && !headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }
  const bodyBytes = await encodeBody(init?.body);
  const payload: SecureRequestPayload = {
    method,
    path: pathOnly,
    query: query || undefined,
    headers: Array.from(headers.entries()),
    body_b64: bodyBytes.length ? Buffer.from(bodyBytes).toString("base64") : "",
  };

  const seq = await nextSecureSeq(deviceId);
  const plaintext = new Uint8Array(Buffer.from(JSON.stringify(payload), "utf-8"));
  const envelope = encryptPayload(key, deviceId, seq, plaintext);

  const secureRes = await fetch(buildUrl(conn, "/api/mobile/secure"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(envelope),
  });
  const secureText = await secureRes.text();
  if (!secureRes.ok) {
    const message = secureText || `${secureRes.status} ${secureRes.statusText}`;
    throw new Error(message.trim());
  }

  if (!secureText) {
    return undefined as T;
  }

  let responseEnvelope: E2eeEnvelope;
  try {
    responseEnvelope = JSON.parse(secureText) as E2eeEnvelope;
  } catch {
    throw new Error("Invalid secure response.");
  }
  if (responseEnvelope.device_id !== deviceId) {
    throw new Error("Secure response device mismatch.");
  }

  const decrypted = decryptPayload(key, deviceId, seq, responseEnvelope);
  const responsePayload = JSON.parse(decodeText(decrypted)) as SecureResponsePayload;
  if (responsePayload.status < 200 || responsePayload.status >= 300) {
    const errorBody = responsePayload.body_b64 ? decodeText(decodeBase64(responsePayload.body_b64)) : "";
    throw new Error(errorBody || `Request failed (${responsePayload.status})`);
  }

  if (!responsePayload.body_b64) {
    return undefined as T;
  }
  const responseText = decodeText(decodeBase64(responsePayload.body_b64));
  if (!responseText) {
    return undefined as T;
  }
  try {
    return JSON.parse(responseText) as T;
  } catch {
    throw new Error(`Unexpected response from daemon: ${responseText.slice(0, 200)}`);
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

export const listTaskSessions = (conn: ConnectionConfig, taskId: string) =>
  fetchJson<Session[]>(conn, `/api/tasks/${taskId}/sessions`);

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

export const getSessionDiff = (conn: ConnectionConfig, sessionId: string) =>
  fetchJson<{ diff: string }>(conn, `/api/sessions/${sessionId}/diff`);

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

export type PairMobileDeviceRequest = {
  pairing_token: string;
  device_id: string;
  device_label?: string;
  platform?: string;
  public_key: string;
  app_version?: string;
};

export const registerMobileDevice = (conn: ConnectionConfig, payload: RegisterMobileDeviceRequest) =>
  fetchJson<MobileDeviceRegistration>(conn, `/api/mobile/register`, {
    method: "POST",
    body: JSON.stringify(payload),
  });

export const pairMobileDevice = async (baseUrl: string, payload: PairMobileDeviceRequest): Promise<E2eeEnvelope> => {
  const url = buildUrlFromBase(baseUrl, "/api/mobile/pair");
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  const text = await res.text();
  if (!res.ok) {
    const message = text || `${res.status} ${res.statusText}`;
    throw new Error(message.trim());
  }
  if (!text) {
    throw new Error("Pairing response missing");
  }
  return JSON.parse(text) as E2eeEnvelope;
};

export type WorkspaceActiveSnapshotParams = {
  limit?: number;
};

export const getWorkspaceActiveSnapshot = (
  conn: ConnectionConfig,
  workspaceId: string,
  params?: WorkspaceActiveSnapshotParams,
) => {
  const search = new URLSearchParams();
  if (params?.limit) search.set("limit", String(params.limit));
  const qs = search.toString();
  const suffix = qs ? `?${qs}` : "";
  return fetchJson<WorkspaceActiveSnapshot>(conn, `/api/workspaces/${workspaceId}/active_snapshot${suffix}`);
};

export const createTask = (
  conn: ConnectionConfig,
  workspaceId: string,
  title: string,
  description?: string,
  opts?: { create_default_session?: boolean },
) =>
  fetchJson<Task>(conn, `/api/workspaces/${workspaceId}/tasks`, {
    method: "POST",
    body: JSON.stringify({
      title,
      description,
      ...(opts?.create_default_session === undefined ? {} : { create_default_session: opts.create_default_session }),
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

export const createSession = (
  conn: ConnectionConfig,
  taskId: string,
  provider_id: string,
  model_id: string,
  opts?: { env_target?: "worktree" | "local" | "cloud" },
) =>
  fetchJson<Session>(conn, `/api/tasks/${taskId}/sessions`, {
    method: "POST",
    body: JSON.stringify({
      provider_id,
      model_id,
      ...(opts?.env_target ? { env_target: opts.env_target } : {}),
    }),
  });

export const getSessionSnapshot = (
  conn: ConnectionConfig,
  sessionId: string,
  limit?: number,
  includeEvents?: boolean,
) => {
  const qs = new URLSearchParams();
  if (limit) qs.set("limit", String(limit));
  if (includeEvents !== undefined) qs.set("include_events", includeEvents ? "1" : "0");
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  return fetchJson<SessionSnapshot>(conn, `/api/sessions/${sessionId}/snapshot${suffix}`);
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
