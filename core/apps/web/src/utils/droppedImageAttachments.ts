import type { MessageAttachment } from "../api/client";
import { imageFilesToInlineAttachments, isImageFile } from "./messageAttachments";
import { desktopReadBinaryFile, isDesktopApp } from "./desktop";

const WINDOWS_DRIVE_PATH_RE = /^\/[A-Za-z]:\//;
const IMAGE_EXTENSION_FALLBACKS: Record<string, string> = {
  avif: "image/avif",
  bmp: "image/bmp",
  gif: "image/gif",
  jpeg: "image/jpeg",
  jpg: "image/jpeg",
  png: "image/png",
  svg: "image/svg+xml",
  tif: "image/tiff",
  tiff: "image/tiff",
  webp: "image/webp",
};

function basename(input: string): string {
  const normalized = input.replace(/\\/g, "/");
  const value = normalized.split("/").filter(Boolean).pop() ?? "";
  return value.trim();
}

function fileNameFromUrl(url: string): string {
  try {
    const parsed = new URL(url, window.location.href);
    const value = basename(parsed.pathname);
    return value || "image";
  } catch {
    return "image";
  }
}

function extensionMimeType(name: string): string {
  const ext = name.trim().toLowerCase().split(".").pop() ?? "";
  return IMAGE_EXTENSION_FALLBACKS[ext] ?? "";
}

function pathLooksLikeImage(path: string): boolean {
  const name = basename(path);
  return Boolean(name) && Boolean(extensionMimeType(name));
}

function normalizeFileUrlToPath(url: string): string | null {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "file:") return null;
    let pathname = decodeURIComponent(parsed.pathname || "");
    if (WINDOWS_DRIVE_PATH_RE.test(pathname)) pathname = pathname.slice(1);
    if (!pathname.trim()) return null;
    return pathname;
  } catch {
    return null;
  }
}

async function readImageFileFromDesktopPath(path: string): Promise<File | null> {
  if (!isDesktopApp()) return null;
  if (!pathLooksLikeImage(path)) return null;
  try {
    const response = await desktopReadBinaryFile({ path });
    const name = basename(response.path) || basename(path) || "image";
    const file = new File([Uint8Array.from(response.bytes)], name, {
      type: extensionMimeType(name) || "",
    });
    return isImageFile(file) ? file : null;
  } catch {
    return null;
  }
}

async function readImageFileFromUrl(url: string, suggestedName?: string | null): Promise<File | null> {
  try {
    const response = await fetch(url);
    if (!response.ok) return null;
    const blob = await response.blob();
    const fallbackName = (suggestedName ?? fileNameFromUrl(url)).trim() || "image";
    const mimeType = blob.type || extensionMimeType(fallbackName) || "";
    const file = new File([blob], fallbackName, { type: mimeType });
    return isImageFile(file) ? file : null;
  } catch {
    return null;
  }
}

export function extractFilesFromTransfer(transfer: DataTransfer | null): File[] {
  if (!transfer) return [];
  const out: File[] = [];
  const files = transfer.files ? Array.from(transfer.files) : [];
  out.push(...files);
  const items = transfer.items;
  if (out.length === 0 && items && items.length > 0) {
    for (const item of Array.from(items)) {
      if (item.kind !== "file") continue;
      const file = item.getAsFile?.();
      if (file) out.push(file);
    }
  }
  return out;
}

export function extractFirstUrlFromTransfer(transfer: DataTransfer | null): string | null {
  if (!transfer) return null;
  const uriRaw = (transfer.getData?.("text/uri-list") ?? "").trim();
  if (uriRaw) {
    for (const line of uriRaw.split("\n")) {
      const value = line.trim();
      if (!value || value.startsWith("#")) continue;
      return value;
    }
  }
  const html = (transfer.getData?.("text/html") ?? "").trim();
  if (html) {
    const match = html.match(/<img[^>]*\ssrc=("([^"]+)"|'([^']+)'|([^\s>]+))/i);
    const src = (match?.[2] ?? match?.[3] ?? match?.[4] ?? "").trim();
    if (src) return src;
  }
  const text = (transfer.getData?.("text/plain") ?? "").trim();
  if (text && /^(https?:|data:image\/|blob:|file:)/i.test(text)) return text;
  return null;
}

export async function imageAttachmentsFromTransfer(transfer: DataTransfer | null): Promise<MessageAttachment[]> {
  const files = extractFilesFromTransfer(transfer);
  if (files.length > 0) return imageFilesToInlineAttachments(files);

  const url = extractFirstUrlFromTransfer(transfer);
  if (!url) return [];
  const fileUrlPath = normalizeFileUrlToPath(url);
  if (fileUrlPath) return imageAttachmentsFromPaths([fileUrlPath]);

  const file = await readImageFileFromUrl(url);
  if (!file) return [];
  return imageFilesToInlineAttachments([file]);
}

export async function imageAttachmentsFromPaths(paths: string[]): Promise<MessageAttachment[]> {
  const diagnostics = (
    globalThis as typeof globalThis & {
      __ctxDroppedImagePathsCalls?: Array<{
        paths: string[];
        resolvedPaths: string[];
        fileCount: number;
      }>;
    }
  ).__ctxDroppedImagePathsCalls ?? [];
  const files: File[] = [];
  const resolvedPaths: string[] = [];
  for (const path of paths) {
    const trimmed = path.trim();
    if (!trimmed) continue;
    resolvedPaths.push(trimmed);
    const file = await readImageFileFromDesktopPath(trimmed);
    if (file) files.push(file);
  }
  diagnostics.push({ paths: [...paths], resolvedPaths, fileCount: files.length });
  (
    globalThis as typeof globalThis & {
      __ctxDroppedImagePathsCalls?: Array<{
        paths: string[];
        resolvedPaths: string[];
        fileCount: number;
      }>;
    }
  ).__ctxDroppedImagePathsCalls = diagnostics;
  if (files.length === 0) return [];
  return imageFilesToInlineAttachments(files);
}
