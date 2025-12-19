import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import {
  InstallInfo,
  InstallProgressEvent,
  ProviderStatus,
  ProviderOptions,
  Workspace,
  authenticateProviderForWorkspace,
  getInstall,
  getProviderOptions,
  idToString,
  installAllProviders,
  installProvider,
  installStreamUrl,
  listProviders,
  listInstallEvents,
  listWorkspaces,
  verifyProviderForWorkspace,
} from "../api/client";
import { isDesktopApp } from "../utils/desktop";

type InstallSession = {
  installId: string;
  state: InstallInfo["state"];
  events: InstallProgressEvent[];
  streamError?: string;
  error?: string;
};

const fmtBytes = (n: number): string => {
  if (!Number.isFinite(n)) return "";
  const units = ["B", "KB", "MB", "GB"];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u += 1;
  }
  return `${v.toFixed(u === 0 ? 0 : 1)} ${units[u]}`;
};

const safeCopy = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // ignore
  }
};

export default function ProvidersPage() {
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [workspaceId, setWorkspaceId] = useState<string | null>(null);
  const [providerOptions, setProviderOptions] = useState<Record<string, ProviderOptions | undefined>>({});
  const [optsBusy, setOptsBusy] = useState<Record<string, boolean>>({});
  const [authBusy, setAuthBusy] = useState<Record<string, boolean>>({});
  const [verifyBusy, setVerifyBusy] = useState<Record<string, boolean>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [installs, setInstalls] = useState<Record<string, InstallSession>>({});

  const eventSourcesRef = useRef<Record<string, EventSource>>({});
  const pollTimeoutsRef = useRef<Record<string, number>>({});

  const refresh = () =>
    listProviders()
      .then(setProviders)
      .catch((e) => setError(e.message));

  useEffect(() => {
    refresh();
    listWorkspaces()
      .then((ws) => {
        setWorkspaces(ws);
        if (!workspaceId && ws.length > 0) {
          setWorkspaceId(idToString((ws[0] as any).id));
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    setProviderOptions({});
  }, [workspaceId]);

  useEffect(() => {
    for (const p of providers) {
      const installId = p.details?.install_id;
      const running = p.details?.install_running === "true";
      if (running && installId && !installs[p.provider_id]) {
        attachInstall(p.provider_id, installId);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providers]);

  useEffect(() => {
    return () => {
      for (const key of Object.keys(eventSourcesRef.current)) {
        eventSourcesRef.current[key].close();
      }
      for (const key of Object.keys(pollTimeoutsRef.current)) {
        window.clearTimeout(pollTimeoutsRef.current[key]);
      }
      eventSourcesRef.current = {};
      pollTimeoutsRef.current = {};
    };
  }, []);

  const attachInstall = async (providerId: string, installId: string) => {
    if (eventSourcesRef.current[providerId] || pollTimeoutsRef.current[providerId]) {
      return;
    }

    setInstalls((prev) => ({
      ...prev,
      [providerId]: {
        installId,
        state: "running",
        events: prev[providerId]?.events ?? [],
        error: prev[providerId]?.error,
      },
    }));

    try {
      const history = await listInstallEvents(installId);
      setInstalls((prev) => ({
        ...prev,
        [providerId]: {
          installId,
          state: prev[providerId]?.state ?? "running",
          events: history,
          streamError: prev[providerId]?.streamError,
          error: prev[providerId]?.error,
        },
      }));
    } catch {
      // ignore
    }

    let eventSource: EventSource | null = null;
    try {
      if (isDesktopApp()) {
        throw new Error("desktop mode uses polling (no EventSource)");
      }
      eventSource = new EventSource(installStreamUrl(installId));
      eventSourcesRef.current[providerId] = eventSource;
      eventSource.addEventListener("progress", (evt: any) => {
        const data = (evt as MessageEvent).data;
        try {
          const ev = JSON.parse(data) as InstallProgressEvent;
          setInstalls((prev) => {
            const existing = prev[providerId];
            const events = [...(existing?.events ?? [])];
            if (!events.find((e) => e.at === ev.at && e.stage === ev.stage && e.message === ev.message)) {
              events.push(ev);
              if (events.length > 200) events.splice(0, events.length - 200);
            }
            return {
              ...prev,
              [providerId]: {
                installId,
                state: existing?.state ?? "running",
                events,
                streamError: existing?.streamError,
                error: existing?.error,
              },
            };
          });
        } catch {
          // ignore
        }
      });
      eventSource.onerror = () => {
        setInstalls((prev) => ({
          ...prev,
          [providerId]: {
            installId,
            state: prev[providerId]?.state ?? "running",
            events: prev[providerId]?.events ?? [],
            streamError: "Lost connection to install stream; polling status…",
            error: prev[providerId]?.error,
          },
        }));
        eventSource?.close();
        delete eventSourcesRef.current[providerId];
      };
    } catch {
      setInstalls((prev) => ({
        ...prev,
        [providerId]: {
          installId,
          state: prev[providerId]?.state ?? "running",
          events: prev[providerId]?.events ?? [],
          streamError: "Failed to open install stream; polling status…",
          error: prev[providerId]?.error,
        },
      }));
    }

    const poll = async () => {
      try {
        const info = await getInstall(installId);
        setInstalls((prev) => ({
          ...prev,
          [providerId]: {
            installId,
            state: info.state,
            events: prev[providerId]?.events ?? [],
            streamError: prev[providerId]?.streamError,
            error: info.error,
          },
        }));
        if (info.state !== "running") {
          eventSource?.close();
          delete eventSourcesRef.current[providerId];
          const t = pollTimeoutsRef.current[providerId];
          if (t) {
            window.clearTimeout(t);
            delete pollTimeoutsRef.current[providerId];
          }
          await refresh();
          return;
        }
      } catch {
        // ignore
      }
      pollTimeoutsRef.current[providerId] = window.setTimeout(poll, 900);
    };
    poll();
  };

  const onInstall = async (id: string) => {
    setBusy(id);
    setError(null);
    try {
      const { install_id } = await installProvider(id);
      await attachInstall(id, install_id);
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(null);
    }
  };

  const onInstallAll = async () => {
    setBusy("all");
    setError(null);
    try {
      const installs = await installAllProviders();
      for (const i of installs) attachInstall(i.provider_id, i.install_id);
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(null);
    }
  };

  const ensureProviderOpts = async (providerId: string) => {
    if (!workspaceId) return;
    if (optsBusy[providerId]) return;
    setOptsBusy((prev) => ({ ...prev, [providerId]: true }));
    try {
      const opts = await getProviderOptions(workspaceId, providerId);
      setProviderOptions((prev) => ({ ...prev, [providerId]: opts }));
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setOptsBusy((prev) => ({ ...prev, [providerId]: false }));
    }
  };

  const onAuthenticate = async (providerId: string) => {
    if (!workspaceId) return;
    setAuthBusy((prev) => ({ ...prev, [providerId]: true }));
    setError(null);
    try {
      await authenticateProviderForWorkspace(workspaceId, providerId);
      await ensureProviderOpts(providerId);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setAuthBusy((prev) => ({ ...prev, [providerId]: false }));
    }
  };

  const onVerify = async (providerId: string) => {
    if (!workspaceId) return;
    setVerifyBusy((prev) => ({ ...prev, [providerId]: true }));
    setError(null);
    try {
      await verifyProviderForWorkspace(workspaceId, providerId);
      await ensureProviderOpts(providerId);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setVerifyBusy((prev) => ({ ...prev, [providerId]: false }));
    }
  };

  return (
    <div className="page">
      <div className="header">
        <Link to="/workspaces">← Workspaces</Link>
      </div>

      <h1>Providers</h1>

      <div className="card">
        <div className="row">
          <strong>Managed installs</strong>
          <button type="button" onClick={onInstallAll} disabled={busy !== null}>
            {busy === "all" ? "Installing…" : "Install all"}
          </button>
        </div>
        <div className="muted">
          Installs ACP agent servers under <code>~/.context/providers/agent-servers</code>.
        </div>
        {error && <div className="error">{error}</div>}
      </div>

      <div className="card">
        <div className="row">
          <strong>Auth status</strong>
          <select
            value={workspaceId ?? ""}
            onChange={(e) => setWorkspaceId(e.target.value || null)}
            disabled={workspaces.length === 0}
          >
            {workspaces.map((ws) => {
              const id = idToString((ws as any).id);
              return (
                <option key={id} value={id}>
                  {ws.name}
                </option>
              );
            })}
          </select>
        </div>
        <div className="muted">Provider auth checks run against the selected workspace root.</div>
      </div>

      <ul className="list">
        {providers.map((p) => (
          <li key={p.provider_id} className="card">
            {workspaceId && (
              <>
                {(() => {
                  const opts = providerOptions[p.provider_id];
                  const verifyStatus = String((opts as any)?.verify?.status ?? "");
                  const statusLine = opts?.auth_required
                    ? "Auth required"
                    : verifyStatus === "ok"
                      ? "Verified"
                      : verifyStatus === "network_error"
                        ? "Offline/unreachable"
                        : verifyStatus === "error"
                          ? "Verify error"
                          : "Unknown";
                  return <div className="muted">Auth: {statusLine}</div>;
                })()}
                <div className="row" style={{ flexWrap: "wrap" }}>
                  <button
                    type="button"
                    onClick={() => ensureProviderOpts(p.provider_id)}
                    disabled={!workspaceId || optsBusy[p.provider_id]}
                    title="Probe session/new (no prompt) to detect auth_required and list models/modes"
                  >
                    {optsBusy[p.provider_id] ? "Checking…" : "Check"}
                  </button>
                  <button
                    type="button"
                    onClick={() => onAuthenticate(p.provider_id)}
                    disabled={!workspaceId || authBusy[p.provider_id] || !providerOptions[p.provider_id]?.auth_required}
                    title="Attempt ACP authenticate for this provider"
                  >
                    {authBusy[p.provider_id] ? "Authenticating…" : "Authenticate"}
                  </button>
                  <button
                    type="button"
                    onClick={() => onVerify(p.provider_id)}
                    disabled={!workspaceId || verifyBusy[p.provider_id]}
                    title="Send a tiny prompt to confirm credentials and connectivity"
                  >
                    {verifyBusy[p.provider_id] ? "Verifying…" : "Verify"}
                  </button>
                </div>
              </>
            )}

            {installs[p.provider_id] && (
              <div className="muted">
                Install: {installs[p.provider_id].state}
                {installs[p.provider_id].streamError ? ` · ${installs[p.provider_id].streamError}` : ""}
              </div>
            )}
            <div className="row">
              <strong>{p.provider_id}</strong>
              <span className="muted">{p.health}</span>
            </div>
            <div className="muted">
              {p.installed ? "Installed" : "Not installed"}
              {p.detected_path ? ` · ${p.detected_path}` : ""}
            </div>

            {(p.details?.managed_package || p.details?.managed_version || p.details?.managed_install_dir) && (
              <div className="muted">
                Managed: {p.details?.managed_package ?? "provider"}
                {p.details?.managed_version ? `@${p.details.managed_version}` : ""}
                {p.details?.managed_install_dir ? ` · ${p.details.managed_install_dir}` : ""}
              </div>
            )}

            {(p.details?.managed_last_success_at || p.details?.managed_last_error) && (
              <div className="muted">
                {p.details?.managed_last_success_at ? `Last success: ${p.details.managed_last_success_at}` : ""}
                {p.details?.managed_last_error
                  ? `${p.details?.managed_last_success_at ? " · " : ""}Last error: ${p.details.managed_last_error}`
                  : ""}
              </div>
            )}

            {(p.version ||
              p.details?.matrix_recommended_version ||
              p.details?.matrix_latest_version ||
              p.details?.matrix_update_available === "true" ||
              p.details?.matrix_update_requires_context === "true") && (
              <div className="muted">
                {p.version ? `Detected: ${p.version}` : "Detected: unknown"}
                {p.details?.matrix_recommended_version
                  ? ` · Recommended: ${p.details.matrix_recommended_version}`
                  : ""}
                {p.details?.matrix_update_available === "true" ? " · Update available" : ""}
                {p.details?.matrix_update_requires_context === "true" ? " · Requires Context update" : ""}
              </div>
            )}

            {p.diagnostics?.length > 0 && (
              <ul className="sublist">
                {p.diagnostics.map((d, i) => (
                  <li key={i} className="muted">
                    {d}
                  </li>
                ))}
              </ul>
            )}

            {installs[p.provider_id]?.events?.length > 0 && (
              <div className="card" style={{ marginTop: 12 }}>
                {(() => {
                  const ev = installs[p.provider_id].events[installs[p.provider_id].events.length - 1];
                  const pct =
                    typeof ev.bytes === "number" && typeof ev.total_bytes === "number" && ev.total_bytes > 0
                      ? Math.round((ev.bytes / ev.total_bytes) * 100)
                      : null;
                  return (
                    <>
                      <div className="muted">
                        {ev.stage}: {ev.message}
                        {pct !== null ? ` · ${pct}% (${fmtBytes(ev.bytes!)} / ${fmtBytes(ev.total_bytes!)})` : ""}
                      </div>
                      {pct !== null && (
                        <progress value={pct} max={100} style={{ width: "100%", marginTop: 8 }} />
                      )}
                    </>
                  );
                })()}
                {installs[p.provider_id].error && (
                  <div className="error">
                    <div className="row">
                      <strong>Install error</strong>
                      <button
                        type="button"
                        onClick={() => safeCopy(installs[p.provider_id].error ?? "")}
                      >
                        Copy
                      </button>
                    </div>
                    <pre style={{ whiteSpace: "pre-wrap", margin: "8px 0 0" }}>
                      {installs[p.provider_id].error}
                    </pre>
                  </div>
                )}
                <ul className="sublist">
                  {installs[p.provider_id].events.slice(-6).map((ev, i) => (
                    <li key={i} className="muted">
                      {ev.stage}: {ev.message}
                    </li>
                  ))}
                </ul>
                {(() => {
                  if (installs[p.provider_id].error) return null;
                  const lastErr = [...installs[p.provider_id].events]
                    .reverse()
                    .find((e) => e.level === "error");
                  if (!lastErr) return null;
                  const text = `[${lastErr.at}] ${p.provider_id} ${lastErr.stage}: ${lastErr.message}`;
                  return (
                    <div className="row">
                      <button type="button" onClick={() => safeCopy(text)}>
                        Copy error
                      </button>
                    </div>
                  );
                })()}
              </div>
            )}

            <div className="row">
              <button
                type="button"
                onClick={() => onInstall(p.provider_id)}
                disabled={busy !== null || installs[p.provider_id]?.state === "running"}
              >
                {busy === p.provider_id || installs[p.provider_id]?.state === "running"
                  ? "Installing…"
                  : p.installed && p.health === "unsupported_version"
                    ? "Update"
                    : p.installed
                      ? "Reinstall"
                    : "Install"}
              </button>
            </div>
          </li>
        ))}
        {providers.length === 0 && <li className="muted">No providers.</li>}
      </ul>
    </div>
  );
}
