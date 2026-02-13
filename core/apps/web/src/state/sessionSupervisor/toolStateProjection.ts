import type { SessionEvent, SessionTurn, SessionTurnTool } from "../../api/client";
import { pickFirstString } from "./eventNormalization";

export const isNonToolStatus = (value: string): boolean => {
  const s = value.trim().toLowerCase();
  return ![
    "pending",
    "queued",
    "running",
    "in_progress",
    "completed",
    "failed",
    "error",
    "ok",
    "success",
    "succeeded",
  ].includes(s);
};

export function isStatusUpdateMeta(meta: any): boolean {
  if (!meta || typeof meta !== "object") return false;
  const codexMeta = meta?.codex ?? {};
  const reasoningKind = codexMeta?.reasoning_kind ?? codexMeta?.reasoningKind;
  if (reasoningKind === "status") return true;

  const statusText = pickFirstString(
    meta?.status_text,
    meta?.statusText,
    meta?.status_string,
    meta?.statusString,
    codexMeta?.status_text,
    codexMeta?.statusText,
    codexMeta?.status_string,
    codexMeta?.statusString,
  );
  if (statusText) return true;

  const statusValue =
    typeof meta?.status === "string"
      ? meta.status
      : typeof codexMeta?.status === "string"
        ? codexMeta.status
        : null;
  if (statusValue && isNonToolStatus(statusValue)) return true;

  return false;
}

export function shouldRenderThoughtChunk(ev: SessionEvent): boolean {
  const payload = ev.payload_json ?? {};
  const meta =
    payload?.acp_update?._meta ??
    payload?.acp_update?.meta ??
    payload?._meta ??
    payload?.meta ??
    {};
  if (meta?.heartbeat === true) return false;
  if (isStatusUpdateMeta(meta)) return false;
  const reasoningKind = meta?.codex?.reasoning_kind ?? meta?.codex?.reasoningKind;
  if (reasoningKind === "summary") return false;
  return true;
}

export function shouldRenderAssistantChunk(ev: SessionEvent): boolean {
  const payload = ev.payload_json ?? {};
  const meta =
    payload?.acp_update?._meta ??
    payload?.acp_update?.meta ??
    payload?._meta ??
    payload?.meta ??
    {};
  if (meta?.heartbeat === true) return false;
  if (isStatusUpdateMeta(meta)) return false;
  return true;
}

export const extractToolCallId = (event: SessionEvent): string | null => {
  const payload = event.payload_json ?? {};
  const direct = payload?.tool_call_id ?? payload?.tool_call?.id ?? payload?.tool?.id;
  if (typeof direct === "string" && direct.trim()) return String(direct);
  const fromUpdate = payload?.acp_update?.tool_call_id ?? payload?.acp_update?.tool_call?.id;
  if (typeof fromUpdate === "string" && fromUpdate.trim()) return String(fromUpdate);
  return null;
};

export const normalizeToolStatus = (raw: string, eventType: string): string => {
  const s = String(raw ?? "").toLowerCase();
  if (s === "inprogress" || s === "in_progress" || s === "running") return "in_progress";
  if (s === "pending" || s === "queued") return "pending";
  if (s === "completed" || s === "complete" || s === "ok" || s === "succeeded") return "completed";
  if (s === "failed" || s === "error") return "failed";
  if (eventType === "tool_result") return "completed";
  return s || "pending";
};

export const extractToolStatus = (event: SessionEvent): string | null => {
  const payload = event.payload_json ?? {};
  const direct = payload?.tool_status ?? payload?.tool?.status ?? payload?.status;
  if (typeof direct === "string" && direct.trim()) return normalizeToolStatus(direct, String(event.event_type ?? ""));
  const fromUpdate = payload?.acp_update?.tool_status ?? payload?.acp_update?.tool?.status;
  if (typeof fromUpdate === "string" && fromUpdate.trim()) return normalizeToolStatus(fromUpdate, String(event.event_type ?? ""));
  if (event.event_type === "tool_result") return "completed";
  if (event.event_type === "tool_call") return "pending";
  return null;
};

export const toolStatusBucket = (status?: string | null): string | null => {
  const s = String(status ?? "").toLowerCase();
  if (s === "pending" || s === "queued") return "pending";
  if (s === "in_progress" || s === "inprogress" || s === "running") return "in_progress";
  if (s === "completed" || s === "complete" || s === "ok" || s === "succeeded") return "completed";
  if (s === "failed" || s === "error") return "failed";
  if (!s) return "pending";
  return "pending";
};

export const applyToolBucketDelta = (turn: SessionTurn, bucket: string | null, delta: number) => {
  if (!bucket || delta === 0) return;
  switch (bucket) {
    case "pending":
      turn.tool_pending = Math.max(0, (turn.tool_pending ?? 0) + delta);
      break;
    case "in_progress":
      turn.tool_running = Math.max(0, (turn.tool_running ?? 0) + delta);
      break;
    case "completed":
      turn.tool_completed = Math.max(0, (turn.tool_completed ?? 0) + delta);
      break;
    case "failed":
      turn.tool_failed = Math.max(0, (turn.tool_failed ?? 0) + delta);
      break;
    default:
      break;
  }
};

export const readTurnStatusFromPayload = (event: SessionEvent): SessionTurn["status"] | null => {
  const raw = event.payload_json?.status;
  if (typeof raw !== "string") return null;
  const status = raw.trim();
  switch (status) {
    case "queued":
    case "running":
    case "completed":
    case "interrupted":
    case "failed":
      return status;
    default:
      return null;
  }
};

export const deriveTurnStatusFromEvent = (event: SessionEvent): SessionTurn["status"] => {
  const payloadStatus = readTurnStatusFromPayload(event);
  if (payloadStatus) return payloadStatus;
  const eventType = String(event.event_type ?? "");
  switch (eventType) {
    case "done":
      return "completed";
    case "turn_finished":
      return "completed";
    case "turn_queued":
      return "queued";
    case "turn_started":
      return "running";
    case "turn_interrupted":
      return "interrupted";
    case "error":
      return "failed";
    default:
      return "running";
  }
};

const TOOL_INPUT_PREVIEW_KEYS = [
  "command",
  "query",
  "pattern",
  "regex",
  "text",
  "path",
  "file",
  "filename",
  "file_path",
  "filePath",
  "filepath",
  "target",
  "paths",
  "paths_total",
  "files",
  "file_paths",
  "filePaths",
  "glob",
  "parsed_cmd",
  "diff_stats",
  "url",
  "uri",
  "href",
  "method",
  "server",
  "tool",
  "tool_name",
  "toolName",
  "cwd",
  "root",
];

export const toolInputPreview = (input: unknown): Record<string, unknown> | null => {
  if (!input || typeof input !== "object" || Array.isArray(input)) return null;
  const obj = input as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const key of TOOL_INPUT_PREVIEW_KEYS) {
    if (obj[key] !== undefined) out[key] = obj[key];
  }
  return Object.keys(out).length > 0 ? out : null;
};

export const summarizeToolPayload = (
  tool: SessionTurnTool,
): SessionTurnTool & { summary_only: boolean } => ({
  ...tool,
  input_json: toolInputPreview(tool.input_json) ?? null,
  output_text: null,
  input_truncated: tool.input_truncated ?? null,
  input_original_bytes: tool.input_original_bytes ?? null,
  output_truncated: tool.output_truncated ?? null,
  output_original_bytes: tool.output_original_bytes ?? null,
  summary_only: true,
});
