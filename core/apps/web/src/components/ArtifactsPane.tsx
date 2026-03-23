import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Copy, Download, RotateCcw, X, ZoomIn, ZoomOut } from "lucide-react";
import { artifactUrl, idToString, type Artifact } from "../api/client";
import { MemoMarkdown } from "../pages/SessionPage.markdown";
import {
  getArtifactPreviewKind,
  isImageArtifact,
  isPreviewableArtifact,
  isVideoArtifact,
  type ArtifactPreviewKind,
} from "../utils/artifacts";
import { errorMessage } from "../utils/errorMessage";

const MIN_ZOOM = 1;
const MAX_ZOOM = 6;
const BUTTON_ZOOM_FACTOR = 1.2;
const WHEEL_ZOOM_SENSITIVITY = 0.0015;
const MAX_WHEEL_DELTA = 80;

type Point = {
  x: number;
  y: number;
};

type Size = {
  width: number;
  height: number;
};

const ZERO_POINT: Point = { x: 0, y: 0 };

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

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

function ArtifactInlineTextPreview({
  sessionId,
  artifact,
  previewKind,
}: {
  sessionId: string;
  artifact: Artifact;
  previewKind: Extract<ArtifactPreviewKind, "markdown" | "text">;
}) {
  const artifactId = idToString(artifact.id);
  const url = artifactUrl(sessionId, artifactId);
  const missing = Boolean(artifact.missing);
  const [textPreview, setTextPreview] = useState<TextPreviewState>({
    status: "idle",
    content: "",
    error: null,
  });

  useEffect(() => {
    if (!artifactId || missing) return;
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
  }, [artifactId, missing, url]);

  if (textPreview.status === "loading" || textPreview.status === "idle") {
    return <div className="wb-artifact-inline-status">Loading preview…</div>;
  }

  if (textPreview.status === "error") {
    return <div className="wb-artifact-inline-status">{textPreview.error}</div>;
  }

  if (previewKind === "markdown") {
    return (
      <div className="wb-artifact-inline-markdown wb-tool-markdown">
        <MemoMarkdown content={textPreview.content} />
      </div>
    );
  }

  return <pre className="wb-artifact-inline-text">{textPreview.content}</pre>;
}

function ArtifactCard({
  sessionId,
  artifact,
  onOpen,
}: {
  sessionId: string;
  artifact: Artifact;
  onOpen: (next: Artifact) => void;
}) {
  const [copying, setCopying] = useState(false);
  const name = displayName(artifact);
  const missing = Boolean(artifact.missing);
  const artifactId = idToString(artifact.id);
  const url = artifactUrl(sessionId, artifactId);
  const mimeLabel = artifact.mime_type || "application/octet-stream";
  const meta = `${mimeLabel} · ${formatBytes(artifact.bytes)}`;
  const title = artifact.absolute_path || name;
  const previewKind = getArtifactPreviewKind(artifact);
  const isVideo = previewKind === "video";
  const isImage = previewKind === "image";
  const isInlineTextPreview = previewKind === "markdown" || previewKind === "text";
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
      <video className="wb-artifact-video" autoPlay controls loop muted playsInline preload="metadata">
        <source src={url} type={artifact.mime_type || "video/mp4"} />
      </video>
    );
  } else if (isImage) {
    preview = <img className="wb-artifact-image" src={url} alt={name} />;
  } else if (isInlineTextPreview) {
    preview = (
      <ArtifactInlineTextPreview
        sessionId={sessionId}
        artifact={artifact}
        previewKind={previewKind}
      />
    );
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
  sessionId,
  artifact,
  onClose,
}: {
  sessionId: string;
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
  const url = artifactUrl(sessionId, artifactId);
  const meta = `${artifact.mime_type || "application/octet-stream"} · ${formatBytes(artifact.bytes)}`;
  const missing = Boolean(artifact.missing);
  const [copying, setCopying] = useState(false);
  const [zoom, setZoom] = useState(MIN_ZOOM);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const baseSizeRef = useRef<Size | null>(null);
  const draggingRef = useRef(false);
  const lastPointRef = useRef({ x: 0, y: 0 });
  const [textPreview, setTextPreview] = useState<TextPreviewState>({
    status: "idle",
    content: "",
    error: null,
  });
  const pointerIdRef = useRef<number | null>(null);
  const zoomRef = useRef(MIN_ZOOM);

  useEffect(() => {
    setZoom(MIN_ZOOM);
    zoomRef.current = MIN_ZOOM;
    setOffset(ZERO_POINT);
    setDragging(false);
    draggingRef.current = false;
    pointerIdRef.current = null;
    baseSizeRef.current = null;
    setTextPreview({ status: "idle", content: "", error: null });
  }, [artifactId]);

  useEffect(() => {
    zoomRef.current = zoom;
  }, [zoom]);

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
  const updateBaseSize = useCallback(() => {
    const img = imageRef.current;
    if (!img) return;
    const width = img.clientWidth;
    const height = img.clientHeight;
    if (!width || !height) return;
    baseSizeRef.current = { width, height };
  }, []);

  const clampOffset = useCallback((next: Point, zoomValue: number): Point => {
    const container = containerRef.current;
    const baseSize = baseSizeRef.current;
    if (!container || !baseSize || zoomValue <= MIN_ZOOM) {
      return { x: 0, y: 0 };
    }
    const baseWidth = Number(baseSize.width);
    const baseHeight = Number(baseSize.height);
    const containerWidth = Number(container.clientWidth);
    const containerHeight = Number(container.clientHeight);
    const nextX = Number(next.x);
    const nextY = Number(next.y);
    if (
      !Number.isFinite(baseWidth)
      || !Number.isFinite(baseHeight)
      || !Number.isFinite(containerWidth)
      || !Number.isFinite(containerHeight)
      || !Number.isFinite(nextX)
      || !Number.isFinite(nextY)
      || !Number.isFinite(zoomValue)
    ) {
      return { x: 0, y: 0 };
    }
    const maxX = Math.max(0, (baseWidth * zoomValue - containerWidth) / 2);
    const maxY = Math.max(0, (baseHeight * zoomValue - containerHeight) / 2);
    return {
      x: clamp(nextX, -maxX, maxX),
      y: clamp(nextY, -maxY, maxY),
    };
  }, []);

  const adjustZoom = useCallback(
    (factor: number, focusPoint: Point) => {
      setZoom((currentZoom) => {
        const nextZoom = clamp(currentZoom * factor, MIN_ZOOM, MAX_ZOOM);
        setOffset((currentOffset) => {
          if (nextZoom <= MIN_ZOOM) {
            return { x: 0, y: 0 };
          }
          const ratio = nextZoom / currentZoom;
          const nextOffset = {
            x: focusPoint.x - ratio * (focusPoint.x - currentOffset.x),
            y: focusPoint.y - ratio * (focusPoint.y - currentOffset.y),
          };
          return clampOffset(nextOffset, nextZoom);
        });
        return nextZoom;
      });
    },
    [clampOffset],
  );

  const resetZoom = useCallback(() => {
    setZoom(MIN_ZOOM);
    setOffset({ x: 0, y: 0 });
  }, []);

  const onImageLoad = useCallback(() => {
    if (!isImage) return;
    updateBaseSize();
    setZoom(MIN_ZOOM);
    setOffset({ x: 0, y: 0 });
  }, [isImage, updateBaseSize]);

  useEffect(() => {
    if (!isImage) return;
    const onResize = () => {
      updateBaseSize();
      setOffset((current) => clampOffset(current, zoomRef.current));
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [clampOffset, isImage, updateBaseSize]);

  const onWheel = useCallback(
    (event: React.WheelEvent<HTMLDivElement>) => {
      if (!isImage) return;
      event.preventDefault();
      const container = containerRef.current;
      if (!container) return;
      const unit =
        event.deltaMode === WheelEvent.DOM_DELTA_LINE
          ? 16
          : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
            ? container.clientHeight || 800
            : 1;
      const normalizedDelta = clamp(event.deltaY * unit, -MAX_WHEEL_DELTA, MAX_WHEEL_DELTA);
      const rect = container.getBoundingClientRect();
      const focusPoint = {
        x: event.clientX - rect.left - rect.width / 2,
        y: event.clientY - rect.top - rect.height / 2,
      };
      const factor = Math.exp(-normalizedDelta * WHEEL_ZOOM_SENSITIVITY);
      adjustZoom(factor, focusPoint);
    },
    [adjustZoom, isImage],
  );

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!isImage || zoom <= MIN_ZOOM) return;
      event.preventDefault();
      draggingRef.current = true;
      pointerIdRef.current = event.pointerId;
      lastPointRef.current = { x: event.clientX, y: event.clientY };
      setDragging(true);
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [isImage, zoom],
  );

  const onPointerMove = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current) return;
    event.preventDefault();
    const dx = event.clientX - lastPointRef.current.x;
    const dy = event.clientY - lastPointRef.current.y;
    if (!Number.isFinite(dx) || !Number.isFinite(dy)) return;
    lastPointRef.current = { x: event.clientX, y: event.clientY };
    setOffset((prev) => clampOffset({ x: prev.x + dx, y: prev.y + dy }, zoomRef.current));
  }, [clampOffset]);

  const onPointerUp = useCallback((event?: React.PointerEvent<HTMLDivElement>) => {
    draggingRef.current = false;
    setDragging(false);
    const pointerId = event?.pointerId ?? pointerIdRef.current;
    if (event && pointerId !== null && event.currentTarget.hasPointerCapture(pointerId)) {
      event.currentTarget.releasePointerCapture(pointerId);
    }
    pointerIdRef.current = null;
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
                  onClick={() => adjustZoom(1 / BUTTON_ZOOM_FACTOR, ZERO_POINT)}
                  disabled={zoom <= MIN_ZOOM}
                  aria-label="Zoom out"
                  title="Zoom out"
                >
                  <ZoomOut size={14} />
                </button>
                <button
                  type="button"
                  className="wb-artifact-action"
                  onClick={() => adjustZoom(BUTTON_ZOOM_FACTOR, ZERO_POINT)}
                  disabled={zoom >= MAX_ZOOM}
                  aria-label="Zoom in"
                  title="Zoom in"
                >
                  <ZoomIn size={14} />
                </button>
                <button
                  type="button"
                  className="wb-artifact-action"
                  onClick={resetZoom}
                  disabled={zoom === MIN_ZOOM && offset.x === 0 && offset.y === 0}
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
          className={`wb-artifact-modal-body ${zoom > MIN_ZOOM ? "wb-artifact-zoomed" : ""} ${dragging ? "wb-artifact-dragging" : ""} ${isTextPreview ? "wb-artifact-modal-body-text" : ""}`}
          onWheel={onWheel}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onPointerLeave={onPointerUp}
        >
          {missing ? (
            <div className="wb-artifact-missing">Missing on disk</div>
          ) : isVideo ? (
            <video className="wb-artifact-modal-video" autoPlay controls loop muted playsInline preload="metadata">
              <source src={url} type={artifact.mime_type || "video/mp4"} />
            </video>
          ) : isImage ? (
            <img
              className="wb-artifact-modal-image"
              ref={imageRef}
              src={url}
              alt={name}
              onLoad={onImageLoad}
              draggable={false}
              style={{ transform: `translate(${offset.x}px, ${offset.y}px) scale(${zoom})` }}
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
  sessionId,
  artifacts,
  loading,
  error,
  onRetry,
}: {
  sessionId: string;
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
      return (
        <ArtifactCard
          key={key}
          sessionId={sessionId}
          artifact={artifact}
          onOpen={setViewerArtifact}
        />
      );
    });
  }, [artifacts, sessionId]);

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
        <ArtifactViewer
          sessionId={sessionId}
          artifact={viewerArtifact}
          onClose={() => setViewerArtifact(null)}
        />
      ) : null}
    </div>
  );
}
