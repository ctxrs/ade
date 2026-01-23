import { memo, useEffect, useRef, useState } from "react";
import Editor from "@monaco-editor/react";
import type { editor as MonacoEditor } from "monaco-editor";
import { ChevronDown, ChevronUp } from "lucide-react";
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

type PendingDiffRequest = {
  resolve: (files: DiffFile[]) => void;
  reject: (error: Error) => void;
};

let diffWorker: Worker | null = null;
let diffWorkerFailed = false;
let diffWorkerSeq = 0;
const diffWorkerPending = new Map<number, PendingDiffRequest>();

const failDiffWorker = (error: Error) => {
  diffWorkerFailed = true;
  if (diffWorker) {
    diffWorker.terminate();
    diffWorker = null;
  }
  for (const pending of diffWorkerPending.values()) {
    pending.reject(error);
  }
  diffWorkerPending.clear();
};

const ensureDiffWorker = (): Worker | null => {
  if (diffWorkerFailed) return null;
  if (diffWorker) return diffWorker;
  if (typeof Worker === "undefined") return null;
  try {
    diffWorker = new Worker(new URL("../workers/diffParserWorker.ts", import.meta.url), { type: "module" });
    diffWorker.onmessage = (event) => {
      const payload = event.data as { id?: number; files?: DiffFile[] };
      if (!payload || typeof payload.id !== "number") return;
      const pending = diffWorkerPending.get(payload.id);
      if (!pending) return;
      diffWorkerPending.delete(payload.id);
      pending.resolve(Array.isArray(payload.files) ? payload.files : []);
    };
    diffWorker.onerror = () => {
      failDiffWorker(new Error("Diff worker failed."));
    };
    diffWorker.onmessageerror = () => {
      failDiffWorker(new Error("Diff worker message error."));
    };
    return diffWorker;
  } catch {
    diffWorkerFailed = true;
    diffWorker = null;
    return null;
  }
};

const parseDiffInWorker = (diff: string): Promise<DiffFile[]> => {
  const worker = ensureDiffWorker();
  if (!worker) return Promise.resolve(parseUnifiedDiff(diff));
  return new Promise((resolve, reject) => {
    const id = diffWorkerSeq + 1;
    diffWorkerSeq = id;
    diffWorkerPending.set(id, { resolve, reject });
    worker.postMessage({ id, diff });
  });
};

const DiffReviewPane = memo(function DiffReviewPane({
  diff,
  labels,
}: {
  diff: string;
  labels?: Partial<{
    empty: string;
  }>;
}) {
  const [expandedFiles, setExpandedFiles] = useState<Record<string, boolean>>({});
  const [wrapLines, setWrapLines] = useState(true);
  const [files, setFiles] = useState<DiffFile[]>([]);
  const [parsing, setParsing] = useState(false);

  useEffect(() => {
    setExpandedFiles({});
  }, [diff]);

  useEffect(() => {
    const diffText = String(diff ?? "");
    if (!diffText.trim()) {
      setFiles([]);
      setParsing(false);
      return;
    }
    let cancelled = false;
    setParsing(true);
    setFiles([]);
    parseDiffInWorker(diffText)
      .then((next) => {
        if (cancelled) return;
        setFiles(next);
      })
      .catch(() => {
        if (cancelled) return;
        setFiles(parseUnifiedDiff(diffText));
      })
      .finally(() => {
        if (cancelled) return;
        setParsing(false);
      });
    return () => {
      cancelled = true;
    };
  }, [diff]);

  const toggleFile = (key: string) => setExpandedFiles((prev) => ({ ...prev, [key]: !(prev[key] ?? false) }));

  const hasChanges = diff.trim().length > 0;

  return (
    <div className="diff-pane">
      {!hasChanges && <div className="muted">{labels?.empty ?? "No changes."}</div>}

      {hasChanges && parsing && <div className="muted">Parsing diff...</div>}

      {hasChanges && !parsing && (
        <div className="cursor-diff">
          <div className="cursor-diff-toolbar">
            <button
              type="button"
              className={`cursor-diff-toggle ${wrapLines ? "cursor-diff-toggle-active" : ""}`}
              aria-pressed={wrapLines}
              title={wrapLines ? "Disable line wrap" : "Enable line wrap"}
              onClick={() => setWrapLines((prev) => !prev)}
            >
              Wrap lines
            </button>
          </div>
          <div className="cursor-diff-list">
            {files.length === 0 && <div className="muted">No parsed file diffs yet.</div>}
            {files.map((f) => {
              const isOpen = expandedFiles[f.key] ?? false;

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
                  className={`cursor-diff-file ${fileAccentClass(f)}`}
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
                    <span className="cursor-diff-file-path" title={f.filePath}>
                      {f.filePath}
                    </span>
                    {summary}
                    <div className="cursor-diff-spacer" />
                  </div>

                  {isOpen && (
                    <div className="cursor-diff-file-body" role="region" aria-label={`Diff for ${f.filePath}`}>
                      {f.isBinary ? (
                        <div className="muted" style={{ padding: 12 }}>
                          Binary or metadata-only diff.
                        </div>
                      ) : (
                        <>
                          <div className="cursor-diff-editor-shell">
                            <DecoratedDiffEditor file={f} wrapLines={wrapLines} />
                          </div>
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

const WRAP_BREAK_AFTER_CHARACTERS =
  "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_/-=+*.,:;|\\~!@#$%^&()[]{}<>?\"'";

function DecoratedDiffEditor({ file, wrapLines }: { file: DiffFile; wrapLines: boolean }) {
  const decorationIdsRef = useRef<string[]>([]);
  const [editorHeight, setEditorHeight] = useState(() => estimateDiffHeightPx(file));
  const [editorInstance, setEditorInstance] = useState<MonacoEditor.IStandaloneCodeEditor | null>(null);
  const modelPath = `inmemory://diff/${encodeURIComponent(file.key)}`;

  useEffect(() => {
    setEditorHeight(estimateDiffHeightPx(file));
  }, [file.key, file.renderText]);

  useEffect(() => {
    if (!editorInstance) return;
    const updateHeight = () => {
      const contentHeight = editorInstance.getContentHeight();
      const minHeight = estimateDiffHeightPx(file);
      setEditorHeight(Math.ceil(Math.max(minHeight, contentHeight)));
    };
    updateHeight();
    const disposable = editorInstance.onDidContentSizeChange(updateHeight);
    return () => disposable.dispose();
  }, [editorInstance, file.key, file.renderText]);

  return (
    <Editor
      key={`${file.key}:${wrapLines ? "wrap" : "nowrap"}`}
      height={`${editorHeight}px`}
      language="diff"
      path={modelPath}
      value={file.renderText}
      theme="vs-dark"
      options={{
        readOnly: true,
        minimap: { enabled: false },
        scrollbar: {
          vertical: "hidden",
          horizontal: wrapLines ? "hidden" : "auto",
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
        renderValidationDecorations: "off",
        fixedOverflowWidgets: true,
        padding: { top: 10, bottom: 10 },
        wordWrap: wrapLines ? "on" : "off",
        wrappingStrategy: wrapLines ? "advanced" : "simple",
        wordWrapBreakAfterCharacters: wrapLines ? WRAP_BREAK_AFTER_CHARACTERS : undefined,
      }}
      onMount={(editor, monaco) => {
        setEditorInstance(editor);
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
