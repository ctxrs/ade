import path from "node:path";

const VIDEO_MIME_BY_EXTENSION = new Map([
  [".mp4", "video/mp4"],
  [".mov", "video/quicktime"],
  [".webm", "video/webm"],
  [".m4v", "video/x-m4v"],
]);

export function inferVideoArtifactMimeType(artifactPath) {
  const extension = path.extname(String(artifactPath ?? "")).toLowerCase();
  return VIDEO_MIME_BY_EXTENSION.get(extension) ?? "application/octet-stream";
}
