import type { Artifact } from "../api/client";

const VIDEO_EXTENSIONS = new Set(["mp4", "mov", "webm", "m4v"]);

const artifactExtension = (value?: string | null): string => {
  if (!value) return "";
  const parts = value.split(".");
  if (parts.length <= 1) return "";
  return parts[parts.length - 1].toLowerCase();
};

export const isVideoArtifact = (artifact: Artifact): boolean => {
  const mime = (artifact.mime_type ?? "").toLowerCase();
  if (mime.startsWith("video/")) return true;
  const ext = artifactExtension(artifact.absolute_path || artifact.name || "");
  return VIDEO_EXTENSIONS.has(ext);
};

export const isImageArtifact = (artifact: Artifact): boolean => {
  const mime = (artifact.mime_type ?? "").toLowerCase();
  return mime.startsWith("image/");
};
