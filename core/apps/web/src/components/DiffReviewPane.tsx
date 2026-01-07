import { memo, useEffect, useMemo, useRef, useState } from "react";
import Editor from "@monaco-editor/react";
import { Check, ChevronDown, ChevronUp, MessageSquare, X } from "lucide-react";
import { applyTrackDiffPatch } from "../api/client";
import { guessMonacoLanguage } from "../utils/monacoLanguage";
import { FileBufferEditor } from "./FileBufferEditor";
import { FileIcon } from "./FileIcon";

type DiffFile = {
  key: string;
  oldPath: string;
  newPath: string;
  filePath: string;
  sectionLines: string[];
  headerLines: string[];
  hunks: DiffHunk[];
  isNew: boolean;
  isDeleted: boolean;
  isBinary: boolean;
  addedLines: number;
  deletedLines: number;
  renderText: string;
  renderLineKinds: Array<"add" | "del" | "ctx">;
};

type DiffHunk = {
  key: string;
  headerLine: string;
  lines: string[];
};

const DiffReviewPane = memo(function DiffReviewPane({
  diff,
  trackId,
  sessionId,
  onDiffUpdated,
  onFileSaved,
  labels,
}: {
  diff: string;
  trackId: string;
  sessionId?: string;
  onDiffUpdated: (diff: string) => void;
  onFileSaved?: () => void;
  labels?: Partial<{
    title: string;
    empty: string;
    acceptAll: string;
    rejectAll: string;
    accept: string;
    reject: string;
  }>;
}) {
  const [editingPath, setEditingPath] = useState<string | null>(null);
  const [expandedFiles, setExpandedFiles] = useState<Record<string, boolean>>({});
  const [activeFileKey, setActiveFileKey] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hoverKeepKey, setHoverKeepKey] = useState<string | null>(null);
  const tooltipTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    setEditingPath(null);
    setExpandedFiles({});
    setActiveFileKey(null);
    setBusyKey(null);
    setError(null);
    setHoverKeepKey(null);
  }, [trackId, sessionId]);

  const files = useMemo(() => parseUnifiedDiff(diff), [diff]);

  const doApply = async (key: string, action: "accept" | "reject", patch: string) => {
    if (!trackId) return;
    setBusyKey(key);
    setError(null);
    try {
      const resp = await applyTrackDiffPatch(trackId, action, patch);
      onDiffUpdated(resp.diff ?? "");
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusyKey(null);
    }
  };

  const toggleFile = (key: string) => setExpandedFiles((prev) => ({ ...prev, [key]: !(prev[key] ?? true) }));

  const hasChanges = diff.trim().length > 0;

  if (editingPath && sessionId) {
    return (
      <div className="diff-pane">
        <div style={{ padding: 12 }}>
          <FileBufferEditor
            sessionId={sessionId}
            path={editingPath}
            onClose={() => setEditingPath(null)}
            onSaved={onFileSaved}
          />
        </div>
      </div>
    );
  }

  return (
    <div className="diff-pane">
      {error && <div className="banner">{error}</div>}

      {!hasChanges && <div className="muted">{labels?.empty ?? "No changes."}</div>}

      {hasChanges && (
        <div className="cursor-diff">
          <div className="cursor-diff-list">
            {files.map((f) => {
              const isOpen = expandedFiles[f.key] ?? true;
              const filePatch = f.sectionLines.join("\n") + "\n";
              const fileBusy = busyKey === `file:${f.key}`;
              const canEdit = Boolean(sessionId) && f.filePath !== "(unknown)";

              const summary = (
                <span className="cursor-diff-summary" aria-label="Diff summary">
                  {f.isNew ? (
                    <>
                      <span className="cursor-diff-new">(New)</span>{" "}
                      <span className="cursor-diff-plus">+{f.addedLines}</span>
                    </>
                  ) : f.isDeleted ? (
                    <>
                      <span className="cursor-diff-deleted">(Deleted)</span>{" "}
                      <span className="cursor-diff-minus">-{f.deletedLines}</span>
                    </>
                  ) : (
                    <>
                      <span className="cursor-diff-plus">+{f.addedLines}</span>{" "}
                      <span className="cursor-diff-minus">-{f.deletedLines}</span>
                    </>
                  )}
                </span>
              );

              return (
                <div
                  key={f.key}
                  className={`cursor-diff-file ${fileAccentClass(f)} ${activeFileKey === f.key ? "cursor-diff-file-active" : ""}`}
                >
                  <div className="cursor-diff-file-header">
                    <button
                      type="button"
                      className="cursor-diff-chevron"
                      onClick={() => toggleFile(f.key)}
                      aria-label={isOpen ? "Collapse file diff" : "Expand file diff"}
                      aria-expanded={isOpen}
                    >
                      {isOpen ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
                    </button>
                    <FileIcon path={f.filePath} size={14} className="cursor-diff-file-icon" />
                    <span
                      className="cursor-diff-file-path"
                      title={f.filePath}
                      onDoubleClick={() => {
                        if (canEdit) setEditingPath(f.filePath);
                      }}
                    >
                      {f.filePath}
                    </span>
                    {summary}
                    <div className="cursor-diff-spacer" />

                    <div className="cursor-diff-file-actions" aria-label="File actions">
                      <button
                        type="button"
                        className="cursor-diff-icon-btn"
                        aria-label="Comment"
                        disabled
                        title="Comment"
                      >
                        <MessageSquare size={14} />
                      </button>
                      <button
                        type="button"
                        className="cursor-diff-icon-btn"
                        aria-label={isOpen ? "Collapse" : "Expand"}
                        title={isOpen ? "Collapse" : "Expand"}
                        onClick={() => toggleFile(f.key)}
                      >
                        {isOpen ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
                      </button>
                      <button
                        type="button"
                        className="cursor-diff-icon-btn cursor-diff-icon-btn-danger"
                        aria-label="Undo"
                        title="Undo"
                        disabled={!trackId || fileBusy || busyKey !== null}
                        onClick={() => doApply(`file:${f.key}`, "reject", filePatch)}
                      >
                        <X size={14} />
                      </button>
                      <button
                        type="button"
                        className="cursor-diff-icon-btn cursor-diff-icon-btn-keep"
                        aria-label="Keep"
                        onMouseEnter={() => {
                          if (tooltipTimeoutRef.current) window.clearTimeout(tooltipTimeoutRef.current);
                          setHoverKeepKey(f.key);
                        }}
                        onMouseLeave={() => {
                          tooltipTimeoutRef.current = window.setTimeout(() => setHoverKeepKey(null), 80);
                        }}
                        title="Keep"
                        disabled={!trackId || fileBusy || busyKey !== null}
                        onClick={() => doApply(`file:${f.key}`, "accept", filePatch)}
                      >
                        <Check size={14} />
                      </button>
                      {hoverKeepKey === f.key && (
                        <div className="cursor-diff-tooltip" role="tooltip">
                          Keep changes in this file
                        </div>
                      )}
                    </div>
                  </div>

                  {isOpen && (
                    <div
                      className="cursor-diff-file-body"
                      onMouseDown={() => setActiveFileKey(f.key)}
                      role="region"
                      aria-label={`Diff for ${f.filePath}`}
                    >
                      {f.isBinary ? (
                        <div className="muted" style={{ padding: 12 }}>
                          Binary or metadata-only diff.
                        </div>
                      ) : (
                        <>
                          <div className="cursor-diff-editor-shell">
                            <DecoratedDiffEditor file={f} />
                          </div>

                          {activeFileKey === f.key && (
                            <div className="cursor-diff-overlay" aria-label="Quick actions">
                              <button
                                type="button"
                                className="cursor-diff-overlay-btn"
                                disabled={!trackId || fileBusy || busyKey !== null}
                                onClick={() => doApply(`file:${f.key}`, "reject", filePatch)}
                              >
                                Undo <span className="cursor-diff-kbd">⌘N</span>
                              </button>
                              <button
                                type="button"
                                className="cursor-diff-overlay-btn cursor-diff-overlay-btn-keep"
                                disabled={!trackId || fileBusy || busyKey !== null}
                                onClick={() => doApply(`file:${f.key}`, "accept", filePatch)}
                              >
                                Keep <span className="cursor-diff-kbd">⌘Y</span>
                              </button>
                            </div>
                          )}
                        </>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
});

export { DiffReviewPane };

function parseUnifiedDiff(diffText: string): DiffFile[] {
  const lines = String(diffText ?? "").split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  const files: DiffFile[] = [];
  let current:
    | Omit<
        DiffFile,
        "filePath" | "isNew" | "isDeleted" | "isBinary" | "addedLines" | "deletedLines" | "renderText" | "renderLineKinds"
      >
    | null = null;
  let inHeader = false;
  let currentHunk: DiffHunk | null = null;

  const pushCurrent = () => {
    if (!current) return;
    if (currentHunk) {
      current.hunks.push(currentHunk);
      currentHunk = null;
    }
    const file: DiffFile = finalizeFile(current);
    files.push(file);
    current = null;
    inHeader = false;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.startsWith("diff --git ")) {
      pushCurrent();
      const m = /^diff --git a\/(.+?) b\/(.+)$/.exec(line);
      const oldPath = m?.[1] ?? "";
      const newPath = m?.[2] ?? "";
      const key = `${oldPath}=>${newPath}:${i}`;
      current = {
        key,
        oldPath,
        newPath,
        sectionLines: [line],
        headerLines: [line],
        hunks: [],
      };
      inHeader = true;
      continue;
    }

    if (!current) continue;
    current.sectionLines.push(line);

    if (line.startsWith("@@ ")) {
      if (currentHunk) current.hunks.push(currentHunk);
      currentHunk = { key: `${current.key}:h${current.hunks.length}:${i}`, headerLine: line, lines: [] };
      inHeader = false;
      continue;
    }

    if (inHeader) {
      current.headerLines.push(line);
    } else if (currentHunk) {
      currentHunk.lines.push(line);
    }
  }

  pushCurrent();
  return files.filter((f) => f.sectionLines.some((l) => l.trim().length > 0));
}

function finalizeFile(
  raw: Omit<
    DiffFile,
    "filePath" | "isNew" | "isDeleted" | "isBinary" | "addedLines" | "deletedLines" | "renderText" | "renderLineKinds"
  >,
): DiffFile {
  const filePath =
    raw.newPath && raw.newPath !== "dev/null"
      ? raw.newPath
      : raw.oldPath && raw.oldPath !== "dev/null"
        ? raw.oldPath
        : "(unknown)";
  const headerText = raw.headerLines.join("\n");
  const isNew = raw.oldPath === "dev/null" || headerText.includes("new file mode") || headerText.includes("--- /dev/null");
  const isDeleted = raw.newPath === "dev/null" || headerText.includes("deleted file mode") || headerText.includes("+++ /dev/null");
  const patchText = raw.sectionLines.join("\n");
  const isBinary = patchText.includes("GIT binary patch") || patchText.includes("Binary files");

  let addedLines = 0;
  let deletedLines = 0;
  const renderLines: string[] = [];
  const renderLineKinds: Array<"add" | "del" | "ctx"> = [];

  for (const h of raw.hunks) {
    for (const l of h.lines) {
      if (!l) continue;
      const prefix = l[0];
      if (prefix === "+") {
        if (!l.startsWith("+++")) {
          addedLines += 1;
          renderLines.push(l.slice(1));
          renderLineKinds.push("add");
        }
        continue;
      }
      if (prefix === "-") {
        if (!l.startsWith("---")) {
          deletedLines += 1;
          renderLines.push(l.slice(1));
          renderLineKinds.push("del");
        }
        continue;
      }
      if (prefix === " ") {
        renderLines.push(l.slice(1));
        renderLineKinds.push("ctx");
        continue;
      }
      // \ No newline at end of file
    }
  }

  return {
    ...raw,
    filePath,
    isNew,
    isDeleted,
    isBinary: isBinary || raw.hunks.length === 0,
    addedLines,
    deletedLines,
    renderText: renderLines.join("\n"),
    renderLineKinds,
  };
}

function estimateDiffHeightPx(file: DiffFile): number {
  const visibleLines = Math.max(3, file.renderText ? file.renderText.split("\n").length : 0);
  const lineHeight = 22;
  // Expand the editor to the full diff height so scrolling happens at the page/container level,
  // not inside the diff editor.
  const paddingTopBottom = 20; // matches Monaco options padding { top: 10, bottom: 10 }
  const safetyLines = 2; // avoid off-by-one vertical scrolling inside Monaco
  return (visibleLines + safetyLines) * lineHeight + paddingTopBottom;
}

function fileAccentClass(file: DiffFile): string {
  if (file.isDeleted && !file.isNew) return "cursor-diff-file-deleted";
  if (file.isNew) return "cursor-diff-file-new";
  if (file.deletedLines > 0 && file.addedLines === 0) return "cursor-diff-file-deleted";
  return "cursor-diff-file-modified";
}

function DecoratedDiffEditor({ file }: { file: DiffFile }) {
  const decorationIdsRef = useRef<string[]>([]);
  const modelPath = `inmemory://diff/${encodeURIComponent(file.key)}`;

  return (
    <Editor
      key={file.key}
      height={`${estimateDiffHeightPx(file)}px`}
      language={guessMonacoLanguage(file.filePath)}
      path={modelPath}
      value={file.renderText}
      theme="vs-dark"
      options={{
        readOnly: true,
        minimap: { enabled: false },
        scrollbar: {
          vertical: "hidden",
          horizontal: "hidden",
          handleMouseWheel: false,
          alwaysConsumeMouseWheel: false,
        },
        scrollBeyondLastLine: false,
        overviewRulerLanes: 0,
        hideCursorInOverviewRuler: true,
        glyphMargin: false,
        folding: false,
        lineNumbersMinChars: 2,
        fontSize: 12,
        lineHeight: 22,
        renderLineHighlight: "none",
        fixedOverflowWidgets: true,
        padding: { top: 10, bottom: 10 },
        wordWrap: "off",
      }}
      onMount={(editor, monaco) => {
        const applyDecorations = () => {
          const decs = file.renderLineKinds
            .map((kind, idx) => {
              if (kind === "ctx") return null;
              const range = new monaco.Range(idx + 1, 1, idx + 1, 1);
              return {
                range,
                options: {
                  isWholeLine: true,
                  className: kind === "add" ? "cursor-diff-line-add" : "cursor-diff-line-del",
                },
              };
            })
            .filter(Boolean) as any[];
          decorationIdsRef.current = editor.deltaDecorations(decorationIdsRef.current, decs);
        };

        applyDecorations();
      }}
    />
  );
}
