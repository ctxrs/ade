import { blobUrl, type MessageAttachment, type SessionTurn, type SubagentInvocationChild } from "../api/client";

const PLAIN_TEXT_CACHE_LIMIT = 500;
const plainTextCache = new Map<string, string>();

export function imageAttachmentSrc(a: MessageAttachment): string {
  return a.kind === "image_ref" ? blobUrl(a.blob_id) : `data:${a.mime_type};base64,${a.data_base64}`;
}

export function attachmentDisplayName(name?: string | null) {
  const n = String(name ?? "").trim();
  if (!n) return "image";
  return n.split(/[\\/]/).pop() || "image";
}

export function appendSegment(base: string, addition: string): string {
  const trimmed = addition.trim();
  if (!trimmed) return base;
  if (!base) return trimmed;
  const needsSpace = /\S$/.test(base) && !/^[,.;!?]/.test(trimmed);
  return `${base}${needsSpace ? " " : ""}${trimmed}`;
}

export function appendFragment(base: string, fragment: string): string {
  const b = base ?? "";
  const f = fragment ?? "";
  if (!b) return f;
  if (!f) return b;
  if (f.startsWith(b)) return f;
  if (b.endsWith(f)) return b;
  return `${b}${f}`;
}

export function markdownToPlainText(input: string): string {
  if (!input) return "";
  const cached = plainTextCache.get(input);
  if (cached != null) return cached;
  let text = input.replace(/\r/g, "");
  text = text.replace(/```[a-zA-Z0-9_-]*\n/g, "");
  text = text.replace(/```/g, "");
  text = text.replace(/`([^`]*)`/g, "$1");
  text = text.replace(/!\[([^\]]*)\]\([^)]+\)/g, "$1");
  text = text.replace(/\[([^\]]+)\]\([^)]+\)/g, "$1");
  text = text
    .split("\n")
    .map((line) => line.replace(/^\s*(?:[#>*+-]|\d+\.)\s+/, ""))
    .join("\n");
  text = text.replace(/\n{3,}/g, "\n\n");
  const trimmed = text.trim();
  if (plainTextCache.size >= PLAIN_TEXT_CACHE_LIMIT) {
    const oldest = plainTextCache.keys().next().value as string | undefined;
    if (oldest) plainTextCache.delete(oldest);
  }
  plainTextCache.set(input, trimmed);
  return trimmed;
}

export function parseIsoMs(value?: string | null): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function formatElapsedMs(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600);

  if (hours > 0) {
    return `${hours}h ${String(minutes).padStart(2, "0")}m`;
  }
  if (minutes > 0) {
    return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
  }
  return `${seconds}s`;
}

export function humanTurnStatus(status: SessionTurn["status"]): string {
  switch (status) {
    case "completed":
      return "Completed";
    case "interrupted":
      return "Interrupted";
    case "failed":
      return "Error";
    case "queued":
      return "Queued";
    case "running":
    default:
      return "Working";
  }
}

export function humanToolStatus(status: string): string {
  const s = String(status ?? "").trim().toLowerCase();
  switch (s) {
    case "pending":
    case "queued":
      return "Pending";
    case "running":
    case "in_progress":
    case "inprogress":
      return "Running";
    case "completed":
    case "complete":
    case "ok":
    case "success":
    case "succeeded":
      return "Completed";
    case "failed":
    case "error":
      return "Failed";
    default:
      return status ? String(status) : "";
  }
}

export function subagentChildLabel(child: SubagentInvocationChild): string {
  const label = child.label?.trim();
  if (label) return label;
  return `Subagent ${child.position + 1}`;
}

export function formatSubagentChildMeta(child: SubagentInvocationChild): string {
  const parts: string[] = [];
  if (child.harness) parts.push(child.harness);
  if (child.model) parts.push(child.model);
  if (child.reasoning_effort) parts.push(child.reasoning_effort);
  parts.push(`${child.prompt_length} chars`);
  return parts.join(" · ");
}

export function toolKindIcon(kind: string): string {
  const k = String(kind ?? "").trim().toLowerCase();
  if (k === "execute") return "$";
  if (k === "read" || k === "read_file") return "R";
  if (k === "search" || k === "list" || k === "list_files") return "S";
  if (k === "write" || k === "edit" || k === "apply_patch") return "W";
  return "·";
}

export function humanToolKind(kind: string): string {
  const k = (kind || "").toLowerCase();
  if (k === "execute") return "Run Command";
  if (k === "search") return "Search";
  if (k === "read" || k === "read_file") return "Read File";
  if (k === "edit" || k === "write" || k === "apply_patch") return "Edit File";
  if (k === "list" || k === "list_files") return "List Files";
  if (k === "fetch" || k === "http" || k === "curl") return "Fetch";
  if (k === "think") return "Think";
  if (k === "error") return "Error";
  return kind || "Tool";
}

export function formatToolInput(toolKind: string, input: any): string {
  const k = (toolKind || "").toLowerCase();
  if (k === "execute") {
    const cmd = Array.isArray(input?.command) ? input.command.join(" ") : input?.command;
    const cwd = input?.cwd;
    const out: string[] = [];
    if (cwd) out.push(`cwd: ${cwd}`);
    if (cmd) out.push(`cmd: ${cmd}`);
    return out.join("\n") || JSON.stringify(input, null, 2);
  }
  if (typeof input === "string") return input;
  return JSON.stringify(input, null, 2);
}

type ToolDiffStats = {
  added?: number;
  removed?: number;
  files?: number;
};

function extractToolDiffStats(input: any): ToolDiffStats | null {
  const raw = input?.diff_stats;
  if (!raw || typeof raw !== "object") return null;
  const added = Number((raw as any).added);
  const removed = Number((raw as any).removed);
  const files = Number((raw as any).files);
  const hasAny =
    Number.isFinite(added) || Number.isFinite(removed) || Number.isFinite(files);
  if (!hasAny) return null;
  return {
    added: Number.isFinite(added) ? added : undefined,
    removed: Number.isFinite(removed) ? removed : undefined,
    files: Number.isFinite(files) ? files : undefined,
  };
}

function formatToolDiffStats(input: any): string {
  const stats = extractToolDiffStats(input);
  if (!stats) return "";
  const parts: string[] = [];
  if (stats.added && stats.added > 0) parts.push(`+${stats.added}`);
  if (stats.removed && stats.removed > 0) parts.push(`-${stats.removed}`);
  if (!parts.length && stats.files && stats.files > 0) {
    parts.push(`${stats.files} files`);
  }
  return parts.length ? `(${parts.join(" ")})` : "";
}

function extractToolPaths(input: any): { paths: string[]; total?: number } {
  const paths: string[] = [];
  const push = (value: unknown) => {
    if (typeof value !== "string") return;
    const trimmed = value.trim();
    if (trimmed) paths.push(trimmed);
  };
  push(input?.path);
  push(input?.file);
  push(input?.filename);
  push(input?.file_path);
  push(input?.filePath);
  push(input?.filepath);
  push(input?.target);
  if (Array.isArray(input?.paths)) input.paths.forEach(push);
  if (Array.isArray(input?.files)) input.files.forEach(push);
  if (Array.isArray(input?.file_paths)) input.file_paths.forEach(push);
  if (Array.isArray(input?.filePaths)) input.filePaths.forEach(push);
  if (Array.isArray(input?.parsed_cmd)) {
    input.parsed_cmd.forEach((cmd: any) => push(cmd?.path));
  }
  const seen = new Set<string>();
  const unique = paths.filter((p) => {
    if (seen.has(p)) return false;
    seen.add(p);
    return true;
  });
  const total =
    typeof input?.paths_total === "number" ? input.paths_total : unique.length;
  return { paths: unique, total };
}

function formatToolPathSummary(input: any): string {
  const { paths, total } = extractToolPaths(input);
  if (!paths.length) return "";
  const more = Math.max(0, (total ?? paths.length) - 1);
  const head = truncateMiddle(paths[0], 120);
  return more > 0 ? `${head} +${more} more` : head;
}

export function toolSummaryLine(toolKind: string, input: any): string {
  const k = (toolKind || "").toLowerCase();
  if (k === "execute") {
    const cmd = Array.isArray(input?.command) ? input.command.join(" ") : input?.command;
    return cmd ? truncateMiddle(String(cmd), 120) : "";
  }
  if (k === "search") {
    const q = input?.query ?? input?.pattern ?? input?.regex ?? input?.text;
    const path = formatToolPathSummary(input);
    const query = q ? truncateMiddle(String(q), 120) : "";
    if (query && path) return truncateMiddle(`${query} in ${path}`, 120);
    return query || path;
  }
  if (k === "list" || k === "list_files") {
    return formatToolPathSummary(input);
  }
  if (k === "read" || k === "read_file") {
    return formatToolPathSummary(input);
  }
  if (k === "edit" || k === "write" || k === "apply_patch") {
    const path = formatToolPathSummary(input);
    const stats = formatToolDiffStats(input);
    if (path && stats) return `${path} ${stats}`;
    return path || stats;
  }
  if (k === "fetch" || k === "http" || k === "curl") {
    const method = String(input?.method ?? "GET").trim().toUpperCase();
    const url = input?.url ?? input?.uri ?? input?.href;
    if (url) return truncateMiddle(`${method} ${String(url)}`, 120);
    return method;
  }
  return "";
}

export function truncateMiddle(text: string, maxLen: number): string {
  const s = String(text ?? "");
  if (s.length <= maxLen) return s;
  const head = Math.max(10, Math.floor(maxLen * 0.6));
  const tail = Math.max(10, maxLen - head - 3);
  return `${s.slice(0, head)}...${s.slice(-tail)}`;
}

export function looksLikeMarkdown(text: string): boolean {
  const t = String(text ?? "");
  if (t.includes("```")) return true;
  if (/^#{1,6}\s/m.test(t)) return true;
  if (/^\s*[-*]\s+/m.test(t)) return true;
  if (/\[[^\]]+\]\([^)]+\)/.test(t)) return true;
  return false;
}
