import { useMemo, useState } from "react";
import { applyTrackDiffPatch } from "../api/client";

type DiffFile = {
  key: string;
  oldPath: string;
  newPath: string;
  sectionLines: string[];
  headerLines: string[];
  hunks: DiffHunk[];
};

type DiffHunk = {
  key: string;
  headerLine: string;
  lines: string[];
};

export function DiffReviewPane({
  diff,
  trackId,
  onDiffUpdated,
  labels,
}: {
  diff: string;
  trackId: string;
  onDiffUpdated: (diff: string) => void;
  labels?: Partial<{
    title: string;
    rawToggleShow: string;
    rawToggleHide: string;
    empty: string;
    acceptAll: string;
    rejectAll: string;
    accept: string;
    reject: string;
  }>;
}) {
  const [showRaw, setShowRaw] = useState(false);
  const [expandedFiles, setExpandedFiles] = useState<Record<string, boolean>>({});
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  const toggleFile = (key: string) =>
    setExpandedFiles((prev) => ({ ...prev, [key]: !(prev[key] ?? true) }));

  const hasChanges = diff.trim().length > 0;

  return (
    <div className="diff-pane">
      <div className="diff-header">
        <h2>{labels?.title ?? "Review"}</h2>
        <div className="row" style={{ alignItems: "center" }}>
          <button type="button" onClick={() => setShowRaw((v) => !v)}>
            {showRaw ? labels?.rawToggleHide ?? "Hide raw" : labels?.rawToggleShow ?? "Raw diff"}
          </button>
        </div>
      </div>

      {error && <div className="banner">{error}</div>}

      {!hasChanges && <div className="muted">{labels?.empty ?? "No changes."}</div>}

      {hasChanges && showRaw && <pre className="diff">{diff}</pre>}

      {hasChanges && !showRaw && (
        <div className="diff-review">
          {files.map((f) => {
            const isOpen = expandedFiles[f.key] ?? true;
            const fileLabel = f.newPath || f.oldPath || "(unknown)";
            const filePatch = f.sectionLines.join("\n") + "\n";
            const fileBusy = busyKey === `file:${f.key}`;

            return (
              <div key={f.key} className="diff-file">
                <button type="button" className="diff-file-header" onClick={() => toggleFile(f.key)}>
                  <span className="diff-file-path">{fileLabel}</span>
                  <span className="muted">
                    {f.hunks.length} hunk{f.hunks.length === 1 ? "" : "s"}
                  </span>
                  <span className="thinking-chev">{isOpen ? "▴" : "▾"}</span>
                </button>
                {isOpen && (
                  <div className="diff-file-body">
                    <div className="diff-file-actions">
                      <button
                        type="button"
                        disabled={!trackId || fileBusy || busyKey !== null}
                        onClick={() => doApply(`file:${f.key}`, "accept", filePatch)}
                      >
                        {fileBusy ? "Working…" : labels?.acceptAll ?? "Accept all"}
                      </button>
                      <button
                        type="button"
                        disabled={!trackId || fileBusy || busyKey !== null}
                        onClick={() => doApply(`file:${f.key}`, "reject", filePatch)}
                      >
                        {fileBusy ? "Working…" : labels?.rejectAll ?? "Reject all"}
                      </button>
                    </div>

                    {f.hunks.length > 0 ? (
                      <div className="diff-hunks">
                        {f.hunks.map((h, idx) => {
                          const hunkBusy = busyKey === `hunk:${h.key}`;
                          const patch =
                            f.headerLines.join("\n") + "\n" + h.headerLine + "\n" + h.lines.join("\n") + "\n";
                          return (
                            <div key={h.key} className="diff-hunk">
                              <div className="diff-hunk-top">
                                <div className="muted">
                                  Hunk {idx + 1}: <code>{h.headerLine}</code>
                                </div>
                                <div className="row" style={{ gap: 6 }}>
                                  <button
                                    type="button"
                                    disabled={!trackId || hunkBusy || busyKey !== null}
                                    onClick={() => doApply(`hunk:${h.key}`, "accept", patch)}
                                  >
                                    {hunkBusy ? "Working…" : labels?.accept ?? "Accept"}
                                  </button>
                                  <button
                                    type="button"
                                    disabled={!trackId || hunkBusy || busyKey !== null}
                                    onClick={() => doApply(`hunk:${h.key}`, "reject", patch)}
                                  >
                                    {hunkBusy ? "Working…" : labels?.reject ?? "Reject"}
                                  </button>
                                </div>
                              </div>
                              <HunkPreview lines={h.lines} />
                            </div>
                          );
                        })}
                      </div>
                    ) : (
                      <div className="muted">Binary or metadata-only diff.</div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function parseUnifiedDiff(diffText: string): DiffFile[] {
  const lines = String(diffText ?? "").split("\n");
  const files: DiffFile[] = [];
  let current: DiffFile | null = null;
  let inHeader = false;
  let currentHunk: DiffHunk | null = null;

  const pushCurrent = () => {
    if (!current) return;
    if (currentHunk) {
      current.hunks.push(currentHunk);
      currentHunk = null;
    }
    files.push(current);
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

function HunkPreview({ lines }: { lines: string[] }) {
  const maxLines = 260;
  const shown = lines.length > maxLines ? lines.slice(0, maxLines) : lines;
  return (
    <div className="diff-hunk-pre" role="region" aria-label="Diff hunk">
      {shown.map((l, idx) => {
        const cls = l.startsWith("+") ? "add" : l.startsWith("-") ? "del" : l.startsWith("@@") ? "h" : "ctx";
        return (
          <div key={idx} className={`diff-line ${cls}`}>
            {l === "" ? "\u00A0" : l}
          </div>
        );
      })}
      {lines.length > maxLines && <div className="diff-line ctx">…(truncated)…</div>}
    </div>
  );
}

