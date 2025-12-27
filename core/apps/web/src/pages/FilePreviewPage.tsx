import { useEffect, useMemo, useState } from "react";
import { useLocation } from "react-router-dom";
import { getWorktreeFile } from "../api/client";
import { desktopReadFile, isDesktopApp } from "../utils/desktop";

type FilePreviewState = {
  path: string;
  text: string;
};

const parsePositiveInt = (value: string | null): number | null => {
  if (!value) return null;
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return null;
  return parsed;
};

export default function FilePreviewPage() {
  const location = useLocation();
  const [data, setData] = useState<FilePreviewState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const params = useMemo(() => new URLSearchParams(location.search), [location.search]);
  const worktreeId = params.get("worktreeId");
  const file = params.get("file");
  const path = params.get("path");
  const line = parsePositiveInt(params.get("line"));
  const col = parsePositiveInt(params.get("col"));

  useEffect(() => {
    setError(null);
    setData(null);
    const load = async () => {
      if (worktreeId && file) {
        const resp = await getWorktreeFile(worktreeId, file);
        return { path: resp.path, text: resp.text };
      }
      if (path) {
        if (!isDesktopApp()) {
          throw new Error("Absolute path previews are only available in the desktop app.");
        }
        const resp = await desktopReadFile({ path });
        return { path: resp.path, text: resp.text };
      }
      throw new Error("Missing file reference.");
    };

    setLoading(true);
    load()
      .then((resp) => setData(resp))
      .catch((err: any) => setError(err?.message ?? String(err)))
      .finally(() => setLoading(false));
  }, [worktreeId, file, path]);

  useEffect(() => {
    if (!line) return;
    const id = `file-line-${line}`;
    const el = document.getElementById(id);
    if (!el) return;
    el.scrollIntoView({ block: "center" });
  }, [line, data?.text]);

  const lines = data?.text?.split("\n") ?? [];

  return (
    <div className="page file-preview-page">
      <div className="row">
        <h1 style={{ marginRight: "auto" }}>File Preview</h1>
        {data?.path && <div className="muted">{data.path}</div>}
      </div>
      {line && (
        <div className="muted" style={{ marginBottom: 12 }}>
          Line {line}{col ? `, Column ${col}` : ""}
        </div>
      )}
      {loading && <div className="muted">Loading…</div>}
      {error && <div className="error">{error}</div>}
      {!loading && !error && (
        <div className="file-preview">
          {lines.map((text, idx) => {
            const lineNo = idx + 1;
            const isActive = lineNo === line;
            return (
              <div
                key={lineNo}
                id={`file-line-${lineNo}`}
                className={`file-preview-line${isActive ? " active" : ""}`}
              >
                <span className="file-preview-gutter">{lineNo}</span>
                <span className="file-preview-text">{text || "\u00a0"}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
