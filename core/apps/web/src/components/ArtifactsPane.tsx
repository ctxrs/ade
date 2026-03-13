import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Copy, Download, RotateCcw, X, ZoomIn, ZoomOut } from "lucide-react";
import { artifactUrl, idToString, type Artifact } from "../api/client";
import { MemoMarkdown } from "../pages/SessionPage.markdown";
import { getArtifactPreviewKind, isImageArtifact, isPreviewableArtifact, isVideoArtifact } from "../utils/artifacts";
import { errorMessage } from "../utils/errorMessage";
const DEFAULT_MIN_SCALE = 0.2;
const MAX_SCALE = 5;

type TextPreviewState =
  | { status: "idle" | "loading"; content: string; error: null }
  | { status: "ready"; content: string; error: null }
  | { status: "error"; content: string; error: string };

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

function downloadArtifact(artifact: Artifact, url: string) {
  const a = document.createElement("a");
  a.href = url;
  a.download = artifactFileName(artifact);
  a.rel = "noopener";
  a.click();
}

async function copyArtifactImage(artifact: Artifact, url: string) {
  if (!navigator.clipboard?.write || typeof window.ClipboardItem === "undefined") {
    throw new Error("Clipboard image copy is not supported in this browser.");
  }
  const resp = await fetch(url, { cache: "no-store" });
  if (!resp.ok) throw new Error("Failed to fetch image for clipboard.");
  const blob = await resp.blob();
  const type = blob.type || artifact.mime_type || "image/png";
  await navigator.clipboard.write([new window.ClipboardItem({ [type]: blob })]);
}

function ArtifactCard({
  artifact,
  onOpen,
}: {
  artifact: Artifact;
  onOpen: (next: Artifact) => void;
}) {
  const [copying, setCopying] = useState(false);
  const name = displayName(artifact);
  const missing = Boolean(artifact.missing);
  const artifactId = idToString(artifact.id);
  const url = artifactUrl(artifactId);
  const mimeLabel = artifact.mime_type || "application/octet-stream";
  const meta = `${mimeLabel} · ${formatBytes(artifact.bytes)}`;
  const title = artifact.absolute_path || name;
  const previewKind = getArtifactPreviewKind(artifact);
  const isVideo = previewKind === "video";
  const isImage = previewKind === "image";
  const canPreview = isPreviewableArtifact(artifact) && !missing;
  const canDownload = Boolean(artifactId) && !missing;
  const canCopy = Boolean(artifactId) && !missing && isImage && !copying;

  const onDownload = useCallback(
    (event?: React.MouseEvent) => {
      event?.stopPropagation();
    if (!canDownload) return;
    downloadArtifact(artifact, url);
    },
    [artifact, canDownload, url],
  );

  const onCopy = useCallback(async () => {
    if (!canCopy) return;
    setCopying(true);
    try {
      await copyArtifactImage(artifact, url);
    } catch (err: unknown) {
      window.alert(errorMessage(err) || "Failed to copy image.");
    } finally {
      setCopying(false);
    }
  }, [artifact, canCopy, url]);

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
    <div
      className={`wb-artifact-card ${canPreview ? "wb-artifact-card-previewable" : ""}`}
      title={title}
      onClick={canPreview ? () => onOpen(artifact) : undefined}
    >
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
                onClick={(event) => {
                  event.stopPropagation();
                  void onCopy();
                }}
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

function ArtifactViewer({
  artifact,
  onClose,
}: {
  artifact: Artifact;
  onClose: () => void;
}) {
  const previewKind = getArtifactPreviewKind(artifact);
  const isVideo = previewKind === "video";
  const isImage = previewKind === "image";
  const isMarkdown = previewKind === "markdown";
  const isTextPreview = previewKind === "markdown" || previewKind === "text";
  const name = displayName(artifact);
  const artifactId = idToString(artifact.id);
  const url = artifactUrl(artifactId);
  const meta = `${artifact.mime_type || "application/octet-stream"} · ${formatBytes(artifact.bytes)}`;
  const missing = Boolean(artifact.missing);
  const [copying, setCopying] = useState(false);
  const [scale, setScale] = useState(1);
  const [baseScale, setBaseScale] = useState(1);
  const [minScale, setMinScale] = useState(DEFAULT_MIN_SCALE);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const containerRef = useRef<HTMLDivElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const draggingRef = useRef(false);
  const lastPointRef = useRef({ x: 0, y: 0 });
  const [textPreview, setTextPreview] = useState<TextPreviewState>({
    status: "idle",
    content: "",
    error: null,
  });

  useEffect(() => {
    setScale(1);
    setBaseScale(1);
    setMinScale(DEFAULT_MIN_SCALE);
    setOffset({ x: 0, y: 0 });
    setTextPreview({ status: "idle", content: "", error: null });
  }, [artifactId]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  useEffect(() => {
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  }, []);

  useEffect(() => {
    if (!artifactId || missing || !isTextPreview) return;
    const controller = new AbortController();
    setTextPreview({ status: "loading", content: "", error: null });
    void fetch(url, {
      cache: "no-store",
      signal: controller.signal,
    })
      .then(async (resp) => {
        if (!resp.ok) {
          throw new Error(`Failed to load artifact (${resp.status}).`);
        }
        const content = await resp.text();
        setTextPreview({ status: "ready", content, error: null });
      })
      .catch((err: unknown) => {
        if (controller.signal.aborted) return;
        setTextPreview({
          status: "error",
          content: "",
          error: errorMessage(err) || "Failed to load artifact.",
        });
      });
    return () => controller.abort();
  }, [artifactId, isTextPreview, missing, url]);

  const clampScale = useCallback(
    (value: number) => Math.min(MAX_SCALE, Math.max(minScale, value)),
    [minScale],
  );

  const zoomBy = useCallback(
    (delta: number) => {
      setScale((current) => {
        const next = clampScale(current + delta);
        if (next === baseScale) {
          setOffset({ x: 0, y: 0 });
        }
        return next;
      });
    },
    [baseScale, clampScale],
  );

  const resetZoom = useCallback(() => {
    setScale(baseScale);
    setOffset({ x: 0, y: 0 });
  }, [baseScale]);

  const onImageLoad = useCallback(() => {
    if (!isImage) return;
    const container = containerRef.current;
    const img = imageRef.current;
    if (!container || !img) return;
    const { clientWidth, clientHeight } = container;
    const { naturalWidth, naturalHeight } = img;
    if (!clientWidth || !clientHeight || !naturalWidth || !naturalHeight) return;
    const fitScale = Math.min(1, clientWidth / naturalWidth, clientHeight / naturalHeight);
    setBaseScale(fitScale);
    setMinScale(Math.min(DEFAULT_MIN_SCALE, fitScale));
    setScale(fitScale);
    setOffset({ x: 0, y: 0 });
  }, [isImage]);

  const onWheel = useCallback(
    (event: React.WheelEvent<HTMLDivElement>) => {
      if (!isImage) return;
      event.preventDefault();
      const delta = event.deltaY;
      zoomBy(delta < 0 ? 0.2 : -0.2);
    },
    [isImage, zoomBy],
  );

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!isImage || scale <= baseScale) return;
      draggingRef.current = true;
      lastPointRef.current = { x: event.clientX, y: event.clientY };
    },
    [baseScale, isImage, scale],
  );

  const onPointerMove = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current) return;
    const dx = event.clientX - lastPointRef.current.x;
    const dy = event.clientY - lastPointRef.current.y;
    lastPointRef.current = { x: event.clientX, y: event.clientY };
    setOffset((prev) => ({ x: prev.x + dx, y: prev.y + dy }));
  }, []);

  const onPointerUp = useCallback(() => {
    draggingRef.current = false;
  }, []);

  const onDownload = useCallback(() => {
    if (!artifactId || missing) return;
    downloadArtifact(artifact, url);
  }, [artifact, artifactId, missing, url]);

  const onCopy = useCallback(async () => {
    if (!artifactId || missing || !isImage) return;
    setCopying(true);
    try {
      await copyArtifactImage(artifact, url);
    } catch (err: unknown) {
      window.alert(errorMessage(err) || "Failed to copy image.");
    } finally {
      setCopying(false);
    }
  }, [artifact, artifactId, isImage, missing, url]);

  return (
    <div className="wb-artifact-modal-overlay" onClick={onClose}>
      <div className="wb-artifact-modal" onClick={(event) => event.stopPropagation()}>
        <div className="wb-artifact-modal-header">
          <div className="wb-artifact-modal-title">
            <div className="wb-artifact-modal-name">{name}</div>
            <div className="wb-artifact-modal-sub">{missing ? "Missing" : meta}</div>
          </div>
          <div className="wb-artifact-modal-actions">
            <button
              type="button"
              className="wb-artifact-action"
              onClick={onDownload}
              disabled={!artifactId || missing}
              aria-label="Download artifact"
              title={missing ? "Missing" : "Download"}
            >
              <Download size={14} />
            </button>
            {isImage ? (
              <button
                type="button"
                className="wb-artifact-action"
                onClick={() => void onCopy()}
                disabled={!artifactId || missing || copying}
                aria-label="Copy image"
                title="Copy image"
              >
                <Copy size={14} />
              </button>
            ) : null}
            {isImage ? (
              <>
                <button
                  type="button"
                  className="wb-artifact-action"
                  onClick={() => zoomBy(-0.2)}
                  disabled={scale <= minScale}
                  aria-label="Zoom out"
                  title="Zoom out"
                >
                  <ZoomOut size={14} />
                </button>
                <button
                  type="button"
                  className="wb-artifact-action"
                  onClick={() => zoomBy(0.2)}
                  disabled={scale >= MAX_SCALE}
                  aria-label="Zoom in"
                  title="Zoom in"
                >
                  <ZoomIn size={14} />
                </button>
                <button
                  type="button"
                  className="wb-artifact-action"
                  onClick={resetZoom}
                  disabled={scale === baseScale && offset.x === 0 && offset.y === 0}
                  aria-label="Reset zoom"
                  title="Reset"
                >
                  <RotateCcw size={14} />
                </button>
              </>
            ) : null}
            <button
              type="button"
              className="wb-artifact-action"
              onClick={onClose}
              aria-label="Close"
              title="Close"
            >
              <X size={14} />
            </button>
          </div>
        </div>
        <div
          ref={containerRef}
          className={`wb-artifact-modal-body ${scale > baseScale ? "wb-artifact-zoomed" : ""} ${isTextPreview ? "wb-artifact-modal-body-text" : ""}`}
          onWheel={onWheel}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerLeave={onPointerUp}
        >
          {missing ? (
            <div className="wb-artifact-missing">Missing on disk</div>
          ) : isVideo ? (
            <video className="wb-artifact-modal-video" controls preload="metadata">
              <source src={url} type={artifact.mime_type || "video/mp4"} />
            </video>
          ) : isImage ? (
            <img
              className="wb-artifact-modal-image"
              ref={imageRef}
              src={url}
              alt={name}
              onLoad={onImageLoad}
              style={{ transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})` }}
            />
          ) : isTextPreview ? (
            textPreview.status === "loading" || textPreview.status === "idle" ? (
              <div className="wb-muted">Loading artifact…</div>
            ) : textPreview.status === "error" ? (
              <div className="wb-artifacts-error" role="alert">
                <div>{textPreview.error}</div>
              </div>
            ) : isMarkdown ? (
              <div className="wb-artifact-text-content wb-tool-markdown">
                <MemoMarkdown content={textPreview.content} />
              </div>
            ) : (
              <pre className="wb-artifact-text-content wb-artifact-text-pre">{textPreview.content}</pre>
            )
          ) : (
            <div className="wb-artifact-file">{name}</div>
          )}
        </div>
      </div>
    </div>
  );
}

export function ArtifactsPane({
  artifacts,
  loading,
  error,
  onRetry,
}: {
  artifacts: Artifact[];
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}) {
  const [viewerArtifact, setViewerArtifact] = useState<Artifact | null>(null);
  const rows = useMemo(() => {
    return artifacts.map((artifact) => {
      const artifactId = idToString(artifact.id);
      const key = artifactId || artifact.absolute_path || artifact.name || "artifact";
      return <ArtifactCard key={key} artifact={artifact} onOpen={setViewerArtifact} />;
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
        ) : error ? (
          <div className="wb-artifacts-error" role="alert">
            <div>{error}</div>
            {onRetry ? (
              <div>
                <button type="button" className="wb-session-load-issues-retry" onClick={onRetry}>
                  Retry
                </button>
              </div>
            ) : null}
          </div>
        ) : artifacts.length === 0 ? (
          <div className="wb-muted">No artifacts yet.</div>
        ) : (
          <div className="wb-artifacts-grid">{rows}</div>
        )}
      </div>
      {viewerArtifact ? (
        <ArtifactViewer artifact={viewerArtifact} onClose={() => setViewerArtifact(null)} />
      ) : null}
    </div>
  );
}
