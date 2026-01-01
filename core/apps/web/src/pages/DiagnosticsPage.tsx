import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import {
  appendDesktopLog,
  ApplyAppImageUpdateResp,
  checkUpdates,
  Diagnostics,
  DownloadAppImageUpdateResp,
  LspStatus,
  getDiagnostics,
  getLspStatus,
  UpdateCheck,
  applyAppImageUpdate,
  downloadAppImageUpdate,
  openLogsFolder,
} from "../api/client";

export default function DiagnosticsPage() {
  const [diagnostics, setDiagnostics] = useState<Diagnostics | null>(null);
  const [lspStatus, setLspStatus] = useState<LspStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [updateInfo, setUpdateInfo] = useState<UpdateCheck | null>(null);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [downloadResp, setDownloadResp] = useState<DownloadAppImageUpdateResp | null>(null);
  const [applyResp, setApplyResp] = useState<ApplyAppImageUpdateResp | null>(null);

  const refresh = () => {
    setError(null);
    return Promise.all([
      getDiagnostics(),
      getLspStatus().catch(() => null),
    ])
      .then(([d, lsp]) => {
        setDiagnostics(d);
        setLspStatus(lsp);
        setNotice(null);
      })
      .catch((e) => setError(e.message));
  };

  useEffect(() => {
    appendDesktopLog("ui: opened Diagnostics page").catch(() => {});
    refresh();
  }, []);

  const pretty = useMemo(
    () => (diagnostics ? JSON.stringify(diagnostics, null, 2) : ""),
    [diagnostics],
  );

  const lspMissing = useMemo(() => {
    const servers = lspStatus?.servers ?? [];
    return servers.filter((s) => !s.found);
  }, [lspStatus]);

  const onCopy = async () => {
    if (!diagnostics) return;
    try {
      await navigator.clipboard.writeText(pretty);
      setNotice("Copied diagnostics JSON to clipboard.");
    } catch {
      setNotice("Copy failed. Select the text and copy manually.");
    }
  };

  const onOpenLogs = async () => {
    setError(null);
    setNotice(null);
    try {
      await openLogsFolder();
      appendDesktopLog("ui: requested open logs folder").catch(() => {});
    } catch (e: any) {
      setError(e.message);
    }
  };

  const onCheckUpdates = async () => {
    setError(null);
    setNotice(null);
    setUpdateBusy(true);
    setDownloadResp(null);
    setApplyResp(null);
    try {
      const info = await checkUpdates();
      setUpdateInfo(info);
      if (info.update_available) {
        setNotice(`Update available: ${info.latest_version}`);
      } else {
        setNotice("No update available.");
      }
    } catch (e: any) {
      setError(e.message);
    } finally {
      setUpdateBusy(false);
    }
  };

  const onDownloadUpdate = async () => {
    setError(null);
    setNotice(null);
    setUpdateBusy(true);
    setApplyResp(null);
    try {
      const resp = await downloadAppImageUpdate();
      setDownloadResp(resp);
      setNotice(`Downloaded update to ${resp.downloaded_path}`);
    } catch (e: any) {
      setError(e.message);
    } finally {
      setUpdateBusy(false);
    }
  };

  const onApplyUpdate = async () => {
    setError(null);
    setNotice(null);
    setUpdateBusy(true);
    try {
      const resp = await applyAppImageUpdate();
      setApplyResp(resp);
      setNotice(resp.message);
    } catch (e: any) {
      setError(e.message);
    } finally {
      setUpdateBusy(false);
    }
  };

  return (
    <div className="page">
      <div className="row">
        <h1 style={{ marginRight: "auto" }}>Diagnostics</h1>
        <Link to="/workspaces">Workspaces</Link>
      </div>

      <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
        <button onClick={() => refresh()}>Refresh</button>
        <button onClick={onCopy} disabled={!diagnostics}>
          Copy diagnostics
        </button>
        <button onClick={onOpenLogs} disabled={!diagnostics}>
          Open logs folder
        </button>
      </div>

      {notice && <div className="banner">{notice}</div>}
      {error && <div className="error">{error}</div>}

      {lspStatus && (
        <div className="card">
          <h2 style={{ marginTop: 0 }}>Language Servers (LSP)</h2>
          <div className="muted" style={{ marginBottom: 8 }}>
            <div>
              <b>Enabled:</b> {lspStatus.enabled ? "Yes" : "No"}{" "}
              {!lspStatus.enabled && (
                <span className="muted">
                  (set <code>CTX_LSP_ENABLED=1</code> or{" "}
                  <code>CONTEXT_LSP_ENABLED=1</code>)
                </span>
              )}
            </div>
          </div>

          <ul className="list">
            {lspStatus.servers.map((s) => (
              <li key={s.language}>
                <div style={{ display: "flex", justifyContent: "space-between", gap: 12 }}>
                  <div>
                    <div>
                      <b>{s.language}</b>{" "}
                      <span className="muted">
                        {s.found ? "installed" : "missing"}
                      </span>
                    </div>
                    <div className="muted">
                      <code>{s.command}</code>
                      {s.args?.length ? (
                        <>
                          {" "}
                          <span className="muted">{s.args.join(" ")}</span>
                        </>
                      ) : null}
                    </div>
                    {s.resolved_path ? (
                      <div className="muted">
                        <span className="muted">Path:</span> {s.resolved_path}
                      </div>
                    ) : null}
                    {s.version ? (
                      <div className="muted">
                        <span className="muted">Version:</span> {s.version}
                      </div>
                    ) : null}
                    {!s.found && s.install_hints?.length ? (
                      <div className="muted" style={{ marginTop: 6 }}>
                        <div className="muted">Install:</div>
                        {s.install_hints.map((h, idx) => (
                          <div key={idx}>
                            <code>{h}</code>
                          </div>
                        ))}
                      </div>
                    ) : null}
                  </div>
                </div>
              </li>
            ))}
          </ul>

          {lspMissing.length > 0 && (
            <div className="muted" style={{ marginTop: 10 }}>
              Missing servers will cause LSP features to fail for those languages.
            </div>
          )}
        </div>
      )}

      <div className="card">
        <h2 style={{ marginTop: 0 }}>Updates</h2>
        <div className="muted">
          Uses <code>CTX_DOWNLOAD_BASE_URL</code> (or{" "}
          <code>CONTEXT_DOWNLOAD_BASE_URL</code>) to fetch{" "}
          <code>/releases/&lt;channel&gt;/latest.json</code>.
        </div>
        <div className="row" style={{ gap: 8, flexWrap: "wrap", marginTop: 8 }}>
          <button onClick={onCheckUpdates} disabled={updateBusy}>
            {updateBusy ? "Checking…" : "Check updates"}
          </button>
          <button
            onClick={onDownloadUpdate}
            disabled={updateBusy || !updateInfo?.update_available}
          >
            Download AppImage update
          </button>
          <button
            onClick={onApplyUpdate}
            disabled={updateBusy || !downloadResp?.can_apply_in_place}
          >
            Apply in place
          </button>
        </div>
        <div className="muted" style={{ marginTop: 8 }}>
          <div>
            <b>Current:</b>{" "}
            <span className="muted">{updateInfo?.current_version ?? "Unknown"}</span>
          </div>
          <div>
            <b>Latest:</b>{" "}
            <span className="muted">{updateInfo?.latest_version ?? "Unknown"}</span>
          </div>
          {downloadResp?.downloaded_path && (
            <div>
              <b>Downloaded:</b>{" "}
              <span className="muted">{downloadResp.downloaded_path}</span>
            </div>
          )}
          {applyResp?.target_path && (
            <div>
              <b>Applied to:</b>{" "}
              <span className="muted">{applyResp.target_path}</span>
            </div>
          )}
          {!downloadResp?.can_apply_in_place && downloadResp && (
            <div className="muted">
              Cannot apply in place (not running as AppImage / missing{" "}
              <code>CTX_APPIMAGE_PATH</code> (or{" "}
              <code>CONTEXT_APPIMAGE_PATH</code>). Downloaded file can be applied manually.
            </div>
          )}
        </div>
      </div>

      {diagnostics && (
        <div className="card">
          <div style={{ marginBottom: 8 }}>
            <div>
              <b>Daemon URL:</b>{" "}
              <span className="muted">{diagnostics.daemon.daemon_url}</span>
            </div>
            <div>
              <b>Data root:</b>{" "}
              <span className="muted">{diagnostics.daemon.data_root}</span>
            </div>
            <div>
              <b>Logs:</b> <span className="muted">{diagnostics.logs.dir}</span>
            </div>
          </div>

          <label>
            Diagnostics JSON
            <textarea
              readOnly
              value={pretty}
              style={{ width: "100%", height: 340, fontFamily: "monospace" }}
            />
          </label>
        </div>
      )}

      {diagnostics?.logs?.files?.length ? (
        <div className="card">
          <h2 style={{ marginTop: 0 }}>Log files</h2>
          <ul className="list">
            {diagnostics.logs.files.map((f) => (
              <li key={f.name}>
                <div>{f.name}</div>
                <div className="muted">
                  {f.bytes} bytes
                  {f.modified_utc ? ` • modified ${f.modified_utc}` : ""}
                </div>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
