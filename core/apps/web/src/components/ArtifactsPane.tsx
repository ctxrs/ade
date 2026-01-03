import React, { useCallback, useMemo, useState } from "react";
import { Copy, Download } from "lucide-react";
import { artifactUrl, idToString, type Artifact } from "../api/client";

const VIDEO_EXTENSIONS = new Set(["mp4", "mov", "webm", "m4v"]);

function formatBytes(bytes: number | null | undefined): string {
  if (!bytes || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let idx = 0;
  let value = bytes;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  return `${value.toFixed(value >= 10 || idx === 0 ? 0 : 1)} ${units[idx]}`;
}

function displayName(artifact: Artifact): string {
  const name = (artifact.name ?? "").trim();
  if (name) return name;
  const parts = artifact.absolute_path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? "artifact";
}

function sanitizeFileName(name: string): string {
  const raw = String(name ?? "").trim() || "artifact";
  const noBadChars = raw.replace(/[<>:"/\\|?*\u0000-\u001F]/g, "");
  const collapsed = noBadChars.replace(/\s+/g, "-").replace(/-+/g, "-").replace(/^-+|-+$/g, "");
  return (collapsed || "artifact").slice(0, 80);
}

function artifactFileName(artifact: Artifact): string {
  const name = (artifact.name ?? "").trim();
  const pathParts = artifact.absolute_path.split(/[\\/]/).filter(Boolean);
  const pathBase = pathParts[pathParts.length - 1] ?? "";
  const pathExt = pathBase.includes(".") ? pathBase.split(".").pop() ?? "" : "";
  if (name && pathExt && !name.includes(".")) {
    return sanitizeFileName(`${name}.${pathExt}`);
  }
  return sanitizeFileName(name || pathBase || "artifact");
}

function isVideoArtifact(artifact: Artifact): boolean {
  const mime = (artifact.mime_type ?? "").toLowerCase();
  if (mime.startsWith("video/")) return true;
  const parts = artifact.absolute_path.split(".");
  const ext = parts.length > 1 ? parts[parts.length - 1].toLowerCase() : "";
  return VIDEO_EXTENSIONS.has(ext);
}

function isImageArtifact(artifact: Artifact): boolean {
  const mime = (artifact.mime_type ?? "").toLowerCase();
  return mime.startsWith("image/");
}

function ArtifactCard({ artifact }: { artifact: Artifact }) {
  const [copying, setCopying] = useState(false);
  const name = displayName(artifact);
  const missing = Boolean(artifact.missing);
  const artifactId = idToString(artifact.id);
  const url = artifactUrl(artifactId);
  const mimeLabel = artifact.mime_type || "application/octet-stream";
  const meta = `${mimeLabel} · ${formatBytes(artifact.bytes)}`;
  const title = artifact.absolute_path || name;
  const isVideo = isVideoArtifact(artifact);
  const isImage = isImageArtifact(artifact);
  const canDownload = Boolean(artifactId) && !missing;
  const canCopy = Boolean(artifactId) && !missing && isImage && !copying;

  const onDownload = useCallback(() => {
    if (!canDownload) return;
    const a = document.createElement("a");
    a.href = url;
    a.download = artifactFileName(artifact);
    a.rel = "noopener";
    a.click();
  }, [artifact, canDownload, url]);

  const onCopy = useCallback(async () => {
    if (!canCopy) return;
    if (!navigator.clipboard?.write || typeof window.ClipboardItem === "undefined") {
      window.alert("Clipboard image copy is not supported in this browser.");
      return;
    }
    setCopying(true);
    try {
      const resp = await fetch(url, { cache: "no-store" });
      if (!resp.ok) throw new Error("Failed to fetch image for clipboard.");
      const blob = await resp.blob();
      const type = blob.type || artifact.mime_type || "image/png";
      await navigator.clipboard.write([new window.ClipboardItem({ [type]: blob })]);
    } catch (err: any) {
      window.alert(err?.message ?? "Failed to copy image.");
    } finally {
      setCopying(false);
    }
  }, [artifact.mime_type, canCopy, url]);

  let preview: React.ReactNode = null;
  if (missing) {
    preview = <div className="wb-artifact-missing">Missing on disk</div>;
  } else if (isVideo) {
    preview = (
      <video className="wb-artifact-video" controls preload="metadata">
        <source src={url} type={artifact.mime_type || "video/mp4"} />
      </video>
    );
  } else if (isImage) {
    preview = <img className="wb-artifact-image" src={url} alt={name} />;
  } else {
    preview = <div className="wb-artifact-file">{name}</div>;
  }

  return (
    <div className="wb-artifact-card" title={title}>
      <div className="wb-artifact-preview">{preview}</div>
      <div className="wb-artifact-meta">
        <div className="wb-artifact-name-row">
          <div className="wb-artifact-name">{name}</div>
          <div className="wb-artifact-actions">
            <button
              type="button"
              className="wb-artifact-action"
              onClick={onDownload}
              disabled={!canDownload}
              aria-label="Download artifact"
              title={canDownload ? "Download" : "Missing"}
            >
              <Download size={14} />
            </button>
            {isImage ? (
              <button
                type="button"
                className="wb-artifact-action"
                onClick={onCopy}
                disabled={!canCopy}
                aria-label="Copy image"
                title={canCopy ? "Copy image" : "Copy unavailable"}
              >
                <Copy size={14} />
              </button>
            ) : null}
          </div>
        </div>
        <div className="wb-artifact-sub">{missing ? "Missing" : meta}</div>
      </div>
    </div>
  );
}

export function ArtifactsPane({ artifacts, loading }: { artifacts: Artifact[]; loading?: boolean }) {
  const rows = useMemo(() => {
    return artifacts.map((artifact) => {
      const artifactId = idToString(artifact.id);
      const key = artifactId || artifact.absolute_path || artifact.name || "artifact";
      return <ArtifactCard key={key} artifact={artifact} />;
    });
  }, [artifacts]);

  return (
    <div className="wb-artifacts">
      <div className="wb-artifacts-top">
        <div className="wb-artifacts-title">Artifacts</div>
        <div className="wb-artifacts-count">{artifacts.length}</div>
      </div>
      <div className="wb-artifacts-body">
        {loading ? (
          <div className="wb-muted">Loading artifacts…</div>
        ) : artifacts.length === 0 ? (
          <div className="wb-muted">No artifacts yet.</div>
        ) : (
          <div className="wb-artifacts-grid">{rows}</div>
        )}
      </div>
    </div>
  );
}
