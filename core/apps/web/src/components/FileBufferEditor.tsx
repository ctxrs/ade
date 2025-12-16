import { useEffect, useMemo, useRef, useState } from "react";
import Editor from "@monaco-editor/react";
import { useSessionEntry } from "../state/sessionSupervisor";
import { daemonFetchRaw } from "../api/client";

type OpenResp = {
  buffer_id: string;
  path: string;
  version: number;
  text: string;
  last_disk_sha256: string;
};

type UpdateResp = {
  buffer_id: string;
  version: number;
  last_disk_sha256: string;
};

type ConflictResp = {
  error: string;
  disk_sha256: string;
  disk_text: string;
};

type SaveStatus = "loading" | "dirty" | "saving" | "saved" | "conflict" | "error";

export function FileBufferEditor({
  sessionId,
  path,
  heightPx = 420,
  onClose,
  onSaved,
}: {
  sessionId: string;
  path: string;
  heightPx?: number;
  onClose: () => void;
  onSaved?: () => void;
}) {
  const entry = useSessionEntry(sessionId);
  const diagnostics = entry?.diagnosticsByPath?.[path] ?? [];

  const editorRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);
  const saveTimer = useRef<number | null>(null);
  const syncTimer = useRef<number | null>(null);
  const pendingSave = useRef(false);
  const lastSentVersion = useRef<number>(0);
  const disposables = useRef<{ dispose: () => void }[]>([]);

  const [bufferId, setBufferId] = useState<string | null>(null);
  const [version, setVersion] = useState<number>(1);
  const [text, setText] = useState<string>("");
  const [status, setStatus] = useState<SaveStatus>("loading");
  const [conflict, setConflict] = useState<ConflictResp | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);

  const saveLabel = useMemo(() => {
    if (status === "saving") return "Saving…";
    if (status === "saved") return "Saved";
    if (status === "conflict") return "Conflict";
    if (status === "error") return "Error";
    if (status === "dirty") return "Unsaved…";
    return "Loading…";
  }, [status]);

  const applyMarkers = () => {
    const monaco = monacoRef.current;
    const editor = editorRef.current;
    if (!monaco || !editor) return;
    const model = editor.getModel?.();
    if (!model) return;

    const markers = Array.isArray(diagnostics)
      ? diagnostics.map((d: any) => {
          const sev = Number(d?.severity ?? 0);
          const severity =
            sev === 1
              ? monaco.MarkerSeverity.Error
              : sev === 2
                ? monaco.MarkerSeverity.Warning
                : sev === 3
                  ? monaco.MarkerSeverity.Info
                  : monaco.MarkerSeverity.Hint;
          const startLineNumber = Number(d?.range?.start?.line ?? 0) + 1;
          const startColumn = Number(d?.range?.start?.character ?? 0) + 1;
          const endLineNumber = Number(d?.range?.end?.line ?? 0) + 1;
          const endColumn = Number(d?.range?.end?.character ?? 0) + 1;
          return {
            severity,
            message: String(d?.message ?? "Diagnostic"),
            startLineNumber,
            startColumn,
            endLineNumber,
            endColumn,
          };
        })
      : [];
    monaco.editor.setModelMarkers(model, "context-lsp", markers);
  };

  useEffect(() => {
    applyMarkers();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [JSON.stringify(diagnostics)]);

  const openBuffer = async () => {
    setStatus("loading");
    setLastError(null);
    setConflict(null);
    try {
      const res = await daemonFetchRaw("/api/buffers/open", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ session_id: sessionId, path }),
      });
      if (res.status < 200 || res.status >= 300) throw new Error(res.body);
      const data = (res.body ? JSON.parse(res.body) : null) as OpenResp;
      setBufferId(data.buffer_id);
      setVersion(data.version);
      setText(data.text);
      lastSentVersion.current = data.version;
      setStatus("saved");
    } catch (e: any) {
      setLastError(String(e?.message ?? e));
      setStatus("error");
    }
  };

  const doSave = async (opts?: { force?: boolean; nextText?: string; persist?: boolean }) => {
    const bid = bufferId;
    if (!bid) return;
    const force = Boolean(opts?.force);
    const nextText = opts?.nextText ?? text;
    const persist = opts?.persist ?? true;

    // Ensure monotonic version even if multiple saves happen quickly.
    const nextVersion = Math.max(version + 1, lastSentVersion.current + 1);
    lastSentVersion.current = nextVersion;
    setVersion(nextVersion);
    if (persist) setStatus("saving");
    setLastError(null);
    setConflict(null);
    pendingSave.current = true;

    try {
      const res = await daemonFetchRaw("/api/buffers/update", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ buffer_id: bid, version: nextVersion, text: nextText, force, persist }),
      });
      if (res.status === 409) {
        const body = (res.body ? JSON.parse(res.body) : null) as ConflictResp;
        setConflict(body);
        setStatus("conflict");
        pendingSave.current = false;
        return;
      }
      if (res.status < 200 || res.status >= 300) throw new Error(res.body);
      const data = (res.body ? JSON.parse(res.body) : null) as UpdateResp;
      setVersion(data.version);
      if (persist) setStatus("saved");
      else setStatus("dirty");
      pendingSave.current = false;
      if (persist) onSaved?.();
    } catch (e: any) {
      setLastError(String(e?.message ?? e));
      setStatus("error");
      pendingSave.current = false;
    }
  };

  const scheduleSync = () => {
    if (syncTimer.current) window.clearTimeout(syncTimer.current);
    syncTimer.current = window.setTimeout(() => {
      syncTimer.current = null;
      doSave({ persist: false }).catch(() => {});
    }, 150);
  };

  const scheduleSave = () => {
    if (saveTimer.current) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => {
      saveTimer.current = null;
      doSave().catch(() => {});
    }, 500);
  };

  useEffect(() => {
    openBuffer().catch(() => {});
    return () => {
      if (saveTimer.current) window.clearTimeout(saveTimer.current);
      if (syncTimer.current) window.clearTimeout(syncTimer.current);
      for (const d of disposables.current) {
        try {
          d.dispose();
        } catch {
          // ignore
        }
      }
      disposables.current = [];
      const bid = bufferId;
      if (bid) {
        daemonFetchRaw("/api/buffers/close", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ session_id: sessionId, buffer_id: bid }),
        }).catch(() => {});
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, path]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ display: "flex", justifyContent: "space-between", gap: 8, alignItems: "center" }}>
        <div style={{ fontSize: 12, opacity: 0.8, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          Editing <code>{path}</code> · <span>{saveLabel}</span>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <button
            type="button"
            className="wb-small"
            onClick={() => {
              if (pendingSave.current) return;
              onClose();
            }}
          >
            Back
          </button>
        </div>
      </div>

      {status === "conflict" && conflict && (
        <div className="banner">
          <div style={{ marginBottom: 8 }}>File changed on disk while you were editing.</div>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              type="button"
              className="wb-small"
              onClick={() => {
                setText(conflict.disk_text);
                doSave({ force: true, nextText: conflict.disk_text }).catch(() => {});
              }}
            >
              Reload
            </button>
            <button
              type="button"
              className="wb-primary"
              onClick={() => {
                doSave({ force: true }).catch(() => {});
              }}
            >
              Overwrite
            </button>
          </div>
        </div>
      )}

      {status === "error" && lastError && (
        <div className="banner">
          <div style={{ marginBottom: 8 }}>Error: {lastError}</div>
          <button type="button" className="wb-small" onClick={() => openBuffer().catch(() => {})}>
            Retry
          </button>
        </div>
      )}

      <div style={{ border: "1px solid rgba(255,255,255,0.08)", borderRadius: 8, overflow: "hidden" }}>
        <Editor
          height={`${heightPx}px`}
          language={guessMonacoLanguage(path)}
          path={path}
          value={text}
          theme="vs-dark"
          options={{
            minimap: { enabled: false },
            fontSize: 12,
            scrollBeyondLastLine: false,
            wordWrap: "on",
          }}
          onMount={(editor, monaco) => {
            editorRef.current = editor;
            monacoRef.current = monaco;
            applyMarkers();
            editor.onDidBlurEditorWidget(() => {
              doSave().catch(() => {});
            });

            // Lightweight LSP-backed completion + hover for this editor only.
            const model = editor.getModel?.();
            const matchesThisModel = (m: any) => {
              const p = String(m?.uri?.path ?? "");
              return p.endsWith(path) || p === path || p.endsWith(`/${path}`);
            };
            const language = guessMonacoLanguage(path);
            const completion = monaco.languages.registerCompletionItemProvider(language, {
              triggerCharacters: [".", ":", "<", "\"", "'", "/", "@", "#"],
              provideCompletionItems: async (m: any, pos: any) => {
                if (!matchesThisModel(m)) return { suggestions: [] };
                const line = Math.max(0, Number(pos.lineNumber ?? 1) - 1);
                const character = Math.max(0, Number(pos.column ?? 1) - 1);
                const res = await daemonFetchRaw("/api/lsp/completion", {
                  method: "POST",
                  headers: { "content-type": "application/json" },
                  body: JSON.stringify({ session_id: sessionId, path, line, character }),
                });
                if (res.status < 200 || res.status >= 300) return { suggestions: [] };
                const v = res.body ? JSON.parse(res.body) : null;
                const items = Array.isArray(v?.items) ? v.items : Array.isArray(v) ? v : [];
                const suggestions = items.map((it: any) => ({
                  label: String(it?.label ?? ""),
                  kind: monaco.languages.CompletionItemKind.Text,
                  insertText: String(it?.insertText ?? it?.label ?? ""),
                  detail: it?.detail ? String(it.detail) : undefined,
                }));
                return { suggestions };
              },
            });
            const hover = monaco.languages.registerHoverProvider(language, {
              provideHover: async (m: any, pos: any) => {
                if (!matchesThisModel(m)) return null;
                const line = Math.max(0, Number(pos.lineNumber ?? 1) - 1);
                const character = Math.max(0, Number(pos.column ?? 1) - 1);
                const res = await daemonFetchRaw("/api/lsp/hover", {
                  method: "POST",
                  headers: { "content-type": "application/json" },
                  body: JSON.stringify({ session_id: sessionId, path, line, character }),
                });
                if (res.status < 200 || res.status >= 300) return null;
                const v = res.body ? JSON.parse(res.body) : null;
                const contents = v?.contents;
                const markdown =
                  typeof contents === "string"
                    ? contents
                    : typeof contents?.value === "string"
                      ? contents.value
                      : Array.isArray(contents)
                        ? contents
                            .map((c: any) => (typeof c === "string" ? c : typeof c?.value === "string" ? c.value : ""))
                            .filter(Boolean)
                            .join("\n\n")
                        : "";
                if (!markdown) return null;
                return { contents: [{ value: markdown }] };
              },
            });
            disposables.current.push(completion, hover);
            if (model) applyMarkers();
          }}
          onChange={(v) => {
            const next = String(v ?? "");
            setText(next);
            if (status !== "saving") setStatus("dirty");
            scheduleSync();
            scheduleSave();
          }}
        />
      </div>
    </div>
  );
}

function guessMonacoLanguage(path: string): string {
  const lower = String(path).toLowerCase();
  if (lower.endsWith(".ts") || lower.endsWith(".tsx")) return "typescript";
  if (lower.endsWith(".js") || lower.endsWith(".jsx") || lower.endsWith(".mjs") || lower.endsWith(".cjs")) return "javascript";
  if (lower.endsWith(".rs")) return "rust";
  if (lower.endsWith(".py")) return "python";
  if (lower.endsWith(".go")) return "go";
  if (lower.endsWith(".json")) return "json";
  if (lower.endsWith(".md")) return "markdown";
  return "plaintext";
}
