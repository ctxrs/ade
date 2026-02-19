import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import {
  InstallInfo,
  InstallProgressEvent,
  ProviderStatus,
  ProviderOptions,
  Workspace,
  getInstall,
  getProviderOptions,
  idToString,
  installAllProviders,
  installProvider,
  installStreamUrl,
  listProviders,
  listInstallEvents,
  listWorkspaces,
} from "../api/client";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../components/ui/select";
import { copyTextToClipboard } from "../utils/clipboard";
import { isDesktopApp } from "../utils/desktop";
import { errorMessage } from "../utils/errorMessage";
import { PROVIDER_INSTALLS_ENABLED } from "../utils/providerInstallGate";
import { formatProviderVersionDisplay, getMatrixVersionDisplay } from "../utils/providerVersionLabel";

type InstallSession = {
  installId: string;
  state: InstallInfo["state"];
  events: InstallProgressEvent[];
  streamError?: string;
  error?: string;
};

const nextInstallState = (prev: InstallInfo["state"] | undefined, ev?: InstallProgressEvent) => {
  if (prev && prev !== "running") return prev;
  if (!ev) return prev ?? "running";
  if (ev.level === "error") return "failed";
  if (ev.stage === "done" || ev.level === "success") return "succeeded";
  return prev ?? "running";
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

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

export default function ProvidersPage() {
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [workspaceId, setWorkspaceId] = useState<string | null>(null);
  const [providerOptions, setProviderOptions] = useState<Record<string, ProviderOptions | undefined>>({});
  const [optsBusy, setOptsBusy] = useState<Record<string, boolean>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [installs, setInstalls] = useState<Record<string, InstallSession>>({});
  const installControlsEnabled = PROVIDER_INSTALLS_ENABLED;

  const eventSourcesRef = useRef<Record<string, EventSource>>({});
  const pollTimeoutsRef = useRef<Record<string, number>>({});

  const refresh = () =>
    listProviders()
      .then(setProviders)
      .catch((e: unknown) => setError(errorMessage(e)));

  useEffect(() => {
    refresh();
    listWorkspaces()
      .then((ws) => {
        setWorkspaces(ws);
        if (!workspaceId && ws.length > 0) {
          setWorkspaceId(idToString(ws[0]?.id ?? ""));
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

    void (async () => {
      try {
        const history = await listInstallEvents(installId);
        const last = history[history.length - 1];
        setInstalls((prev) => ({
          ...prev,
          [providerId]: {
            installId,
            state: nextInstallState(prev[providerId]?.state, last),
            events: history,
            streamError: prev[providerId]?.streamError,
            error: prev[providerId]?.error,
          },
        }));
      } catch {
        // ignore
      }
    })();

    let eventSource: EventSource | null = null;
    try {
      if (isDesktopApp()) {
        throw new Error("desktop mode uses polling (no EventSource)");
      }
      eventSource = new EventSource(installStreamUrl(installId));
      eventSourcesRef.current[providerId] = eventSource;
      eventSource.addEventListener("progress", (evt: Event) => {
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
                state: nextInstallState(existing?.state, ev),
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
    } catch (e: unknown) {
      setError(errorMessage(e));
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
    } catch (e: unknown) {
      setError(errorMessage(e));
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
    } catch (e: unknown) {
      setError(errorMessage(e));
    } finally {
      setOptsBusy((prev) => ({ ...prev, [providerId]: false }));
    }
  };

  return (
    <div className="page">
      <div className="header">
        <Link to="/">← Launcher</Link>
      </div>

      <h1>Providers</h1>

      <div className="card">
        <div className="row">
          <strong>Managed installs</strong>
          {installControlsEnabled ? (
            <button type="button" onClick={onInstallAll} disabled={busy !== null}>
              {busy === "all" ? "Installing…" : "Install all"}
            </button>
          ) : null}
        </div>
        <div className="muted">
          Installs ACP agent servers under <code>~/.ctx/providers/agent-servers</code>.
        </div>
        {error && <div className="error">{error}</div>}
      </div>

      <div className="card">
        <div className="row">
          <strong>Provider status</strong>
          <Select
            value={workspaceId ?? undefined}
            onValueChange={(value) => setWorkspaceId(value || null)}
            disabled={workspaces.length === 0}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {workspaces.map((ws) => {
                const id = idToString(ws.id ?? "");
                return (
                  <SelectItem key={id} value={id}>
                    {ws.name}
                  </SelectItem>
                );
              })}
            </SelectContent>
          </Select>
        </div>
        <div className="muted">Provider probes run against the selected workspace root.</div>
      </div>

      <ul className="list">
        {providers.map((p) => {
          const installSupported = p.details?.install_supported === "true";
          const installRunning = installs[p.provider_id]?.state === "running";
          const installDisabled = busy !== null || installRunning || !installSupported;
          const detectedVersionLabel = formatProviderVersionDisplay(p);
          const recommendedVersionLabel = getMatrixVersionDisplay(p.details, "recommended");
          const latestVersionLabel = getMatrixVersionDisplay(p.details, "latest");

          return (
            <li key={p.provider_id} className="card">
            {workspaceId && (
              <>
                {(() => {
                  const opts = providerOptions[p.provider_id];
                  const verifyStatus = String(asRecord(opts?.verify).status ?? "");
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

            {(detectedVersionLabel ||
              recommendedVersionLabel ||
              latestVersionLabel ||
              p.details?.matrix_update_available === "true" ||
              p.details?.matrix_update_requires_context === "true") && (
              <div className="muted">
                {detectedVersionLabel ? `Detected: ${detectedVersionLabel}` : "Detected: unknown"}
                {recommendedVersionLabel ? ` · Recommended: ${recommendedVersionLabel}` : ""}
                {p.details?.matrix_update_available === "true" ? " · Update available" : ""}
                {p.details?.matrix_update_requires_context === "true" ? " · Requires ctx update" : ""}
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
                        onClick={() => void copyTextToClipboard(installs[p.provider_id].error ?? "")}
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
                      <button type="button" onClick={() => void copyTextToClipboard(text)}>
                        Copy error
                      </button>
                    </div>
                  );
                })()}
              </div>
            )}

            {installControlsEnabled ? (
              <div className="row">
                <button
                  type="button"
                  onClick={() => onInstall(p.provider_id)}
                  disabled={installDisabled}
                  title={installSupported ? "Install this provider" : "Install not supported yet"}
                >
                  {busy === p.provider_id || installRunning
                    ? "Installing…"
                    : p.installed && p.health === "unsupported_version"
                      ? "Update"
                      : p.installed
                        ? "Reinstall"
                        : "Install"}
                </button>
              </div>
            ) : null}
          </li>
          );
        })}
        {providers.length === 0 && <li className="muted">No providers.</li>}
      </ul>
    </div>
  );
}
