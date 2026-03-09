import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useLocation } from "react-router-dom";
import { applyDaemonDesktopConnection, daemonFetchRaw, listWorkspaceTasks, listWorkspaces } from "../api/client";
import { useDaemonBaseUrl } from "../api/useDaemonConnection";
import {
  desktopApplyAppUpdate,
  desktopGetConnection,
  desktopGetVersion,
  isDesktopApp,
  desktopRestartLocalDaemon,
  desktopUpdateRemoteDaemon,
  type DesktopConnectionKind,
  type DesktopConnectionInfo,
} from "../utils/desktop";

type DaemonStatus = "unknown" | "ok" | "down" | "mismatch";
type VersionMismatchKind = "daemon_older" | "desktop_older" | "unknown";

type VersionMismatch = {
  desktop_version: string;
  daemon_version: string;
  expected_version: string;
  kind: VersionMismatchKind;
};

const overlaySuppressed = (pathname: string): boolean => {
  if (pathname === "/") return true;
  if (pathname === "/workspace-setup") return true;
  return false;
};

const trimError = (value: string): string => {
  const text = String(value || "").trim();
  if (!text) return "";
  return text.length > 220 ? `${text.slice(0, 220)}…` : text;
};

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

const extractErrorMessage = (resp: { status: number; body: string }): string => {
  const raw = String(resp.body ?? "").trim();
  if (!raw) return `Daemon responded with ${resp.status}.`;
  try {
    const parsed = JSON.parse(raw);
    const msg = parsed?.error ?? parsed?.message;
    if (typeof msg === "string" && msg.trim()) return trimError(msg);
  } catch {
    // ignore parse errors
  }
  return trimError(raw);
};

const normalizeVersionParts = (value: string): number[] | null => {
  const trimmed = String(value || "").trim();
  if (!trimmed) return null;
  const cleaned = trimmed.replace(/^v/i, "");
  const parts = cleaned.split(".");
  const nums = parts.map((part) => {
    const match = part.match(/^(\d+)/);
    return match ? Number(match[1]) : Number.NaN;
  });
  if (nums.some((n) => Number.isNaN(n))) return null;
  return nums;
};

const normalizeVersionString = (value: string): string => {
  const trimmed = String(value || "").trim();
  if (!trimmed) return "";
  return trimmed.replace(/^v/i, "");
};

const compareVersions = (left: string, right: string): number | null => {
  const leftParts = normalizeVersionParts(left);
  const rightParts = normalizeVersionParts(right);
  if (!leftParts || !rightParts) return null;
  const len = Math.max(leftParts.length, rightParts.length);
  for (let i = 0; i < len; i += 1) {
    const a = leftParts[i] ?? 0;
    const b = rightParts[i] ?? 0;
    if (a < b) return -1;
    if (a > b) return 1;
  }
  return 0;
};

export default function DaemonAvailabilityOverlay() {
  const location = useLocation();
  const [status, setStatus] = useState<DaemonStatus>("unknown");
  const [checking, setChecking] = useState(false);
  const [restartBusy, setRestartBusy] = useState(false);
  const [remoteUpdateBusy, setRemoteUpdateBusy] = useState(false);
  const [desktopAppUpdateBusy, setDesktopAppUpdateBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [desktopKind, setDesktopKind] = useState<DesktopConnectionKind | null>(null);
  const [desktopVersion, setDesktopVersion] = useState<string | null>(null);
  const [mismatch, setMismatch] = useState<VersionMismatch | null>(null);
  const checkingRef = useRef(false);
  const requestIdRef = useRef(0);
  const restartLockRef = useRef(false);
  const remoteUpdateLockRef = useRef(false);
  const isDesktop = isDesktopApp();
  const daemonBaseUrl = useDaemonBaseUrl();

  const refreshDesktopKind = useCallback(async () => {
    if (!isDesktop) return null;
    try {
      const info = await desktopGetConnection();
      setDesktopKind(info.kind);
      return info;
    } catch {
      setDesktopKind(null);
      return null;
    }
  }, [isDesktop]);

  const refreshDesktopVersion = useCallback(async () => {
    if (!isDesktop) return null;
    try {
      const version = await desktopGetVersion();
      setDesktopVersion(version);
      return version;
    } catch {
      setDesktopVersion(null);
      return null;
    }
  }, [isDesktop]);

  const checkNow = useCallback(async () => {
    if (checkingRef.current) return;
    checkingRef.current = true;
    setChecking(true);
    const requestId = ++requestIdRef.current;
    void refreshDesktopKind();
    try {
      const resp = await daemonFetchRaw("/api/health");
      if (requestId !== requestIdRef.current) return;
      if (resp.status >= 200 && resp.status < 300) {
        let parsed: unknown = null;
        if (resp.body) {
          try {
            parsed = JSON.parse(resp.body);
          } catch {
            parsed = null;
          }
        }
        const parsedRecord = asRecord(parsed);
        const compat = asRecord(parsedRecord.compatibility);
        const daemonVersion = String(parsedRecord.daemon_version ?? parsedRecord.version ?? "").trim();
        const expectedVersion = String(
          compat.desktop_exact_version ?? daemonVersion ?? "",
        ).trim();
        const currentDesktopVersion =
          desktopVersion ?? (await refreshDesktopVersion()) ?? "";
        const normalizedDesktopVersion = normalizeVersionString(currentDesktopVersion);
        const normalizedExpectedVersion = normalizeVersionString(expectedVersion);
        const cmp = compareVersions(normalizedDesktopVersion, normalizedExpectedVersion);
        const versionsMatch =
          cmp === 0 || (cmp === null && normalizedDesktopVersion === normalizedExpectedVersion);
        if (isDesktop && normalizedDesktopVersion && normalizedExpectedVersion && !versionsMatch) {
          const kind: VersionMismatchKind =
            cmp === 1 ? "daemon_older" : cmp === -1 ? "desktop_older" : "unknown";
          setMismatch({
            desktop_version: currentDesktopVersion,
            daemon_version: daemonVersion || expectedVersion,
            expected_version: expectedVersion,
            kind,
          });
          setStatus("mismatch");
          setError(null);
        } else {
          setMismatch(null);
          setStatus("ok");
          setError(null);
        }
      } else {
        setMismatch(null);
        setStatus("down");
        setError(extractErrorMessage(resp));
      }
    } catch (err) {
      if (requestId !== requestIdRef.current) return;
      setMismatch(null);
      const message = err instanceof Error ? err.message : String(err);
      setStatus("down");
      setError(trimError(message || "Unable to reach the ctx daemon."));
    } finally {
      if (requestId === requestIdRef.current) {
        checkingRef.current = false;
        setChecking(false);
      }
    }
  }, [desktopVersion, isDesktop, refreshDesktopKind, refreshDesktopVersion]);

  useEffect(() => {
    checkNow();
    const handleOnline = () => {
      checkNow();
    };
    window.addEventListener("online", handleOnline);
    return () => {
      window.removeEventListener("online", handleOnline);
    };
  }, [checkNow]);

  useEffect(() => {
    void refreshDesktopKind();
  }, [refreshDesktopKind]);

  useEffect(() => {
    void refreshDesktopVersion();
  }, [refreshDesktopVersion]);

  useEffect(() => {
    const intervalMs = status === "down" || status === "mismatch" ? 8000 : 20000;
    const intervalId = window.setInterval(() => {
      if (document.visibilityState !== "visible") return;
      checkNow();
    }, intervalMs);
    return () => {
      window.clearInterval(intervalId);
    };
  }, [checkNow, status]);

  const applyConnection = (info: DesktopConnectionInfo) => {
    applyDaemonDesktopConnection(info);
  };

  const restartDaemon = useCallback(async () => {
    if (!isDesktop || restartLockRef.current) return;
    restartLockRef.current = true;
    setRestartBusy(true);
    setError(null);
    setNotice(null);
    try {
      const info = await desktopRestartLocalDaemon();
      applyConnection(info);
      setDesktopKind(info.kind);
      await checkNow();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(trimError(message || "Unable to restart the daemon."));
    } finally {
      restartLockRef.current = false;
      setRestartBusy(false);
    }
  }, [checkNow, isDesktop]);

  const confirmInterruptingAction = (action: "restart" | "update_remote" | "update_desktop"): boolean => {
    if (typeof window === "undefined") return true;
    const label = action === "update_remote"
      ? "update the remote daemon"
      : action === "update_desktop"
        ? "update the desktop app"
        : "restart the daemon";
    return window.confirm(
      `This will ${label} and may interrupt active agent activity. Continue?`,
    );
  };

  const onMismatchRestartLocal = useCallback(async () => {
    if (!isDesktop || restartBusy) return;
    if (!confirmInterruptingAction("restart")) return;
    await restartDaemon();
  }, [isDesktop, restartBusy, restartDaemon]);

  const onMismatchUpdateRemote = useCallback(async () => {
    if (!isDesktop || remoteUpdateLockRef.current) return;
    remoteUpdateLockRef.current = true;
    setRemoteUpdateBusy(true);
    setError(null);
    setNotice(null);
    try {
      let runningTaskCount: number | null = null;
      try {
        const workspaces = await listWorkspaces();
        const workspaceIds = workspaces
          .map((workspace) => String(workspace.id ?? "").trim())
          .filter(Boolean);
        if (workspaceIds.length === 0) {
          runningTaskCount = 0;
        } else {
          const taskLists = await Promise.all(
            workspaceIds.map(async (workspaceId) => listWorkspaceTasks(workspaceId)),
          );
          runningTaskCount = taskLists
            .flat()
            .filter((task) => {
              const status = String(task.status ?? "").toLowerCase();
              return status === "running" || Boolean(task.has_active_session);
            }).length;
        }
      } catch {
        runningTaskCount = null;
      }

      if (runningTaskCount === null && !confirmInterruptingAction("update_remote")) return;
      if (
        runningTaskCount !== null
        && runningTaskCount > 0
        && typeof window !== "undefined"
        && !window.confirm(
          `Detected ${runningTaskCount} running task(s). Updating the remote daemon may interrupt them. Continue?`,
        )
      ) {
        return;
      }
      await desktopUpdateRemoteDaemon("stable");
      await checkNow();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(trimError(message || "Unable to update the remote daemon."));
    } finally {
      remoteUpdateLockRef.current = false;
      setRemoteUpdateBusy(false);
    }
  }, [checkNow, isDesktop]);

  const onMismatchUpdateDesktop = useCallback(async () => {
    if (!isDesktop || desktopAppUpdateBusy) return;
    if (!confirmInterruptingAction("update_desktop")) return;
    setDesktopAppUpdateBusy(true);
    setError(null);
    setNotice(null);
    try {
      const resp = await desktopApplyAppUpdate("stable");
      if (resp.needs_restart) {
        const details = String(resp.message || "").trim();
        const guidance = details
          ? `${details} Restart the desktop app to apply the update.`
          : "Desktop update installed. Restart the desktop app to apply the update.";
        setNotice(trimError(guidance));
        return;
      }
      await checkNow();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(trimError(message || "Unable to update the desktop app."));
    } finally {
      setDesktopAppUpdateBusy(false);
    }
  }, [checkNow, desktopAppUpdateBusy, isDesktop]);

  const target = useMemo(() => {
    return daemonBaseUrl ?? "";
  }, [daemonBaseUrl]);

  const showOverlay =
    (status === "down" || status === "mismatch") && !overlaySuppressed(location.pathname);
  if (!showOverlay) return null;

  const canRestart = isDesktop && (desktopKind === "local" || desktopKind === "none");
  const primaryAction = canRestart ? restartDaemon : checkNow;
  const primaryLabel = canRestart
    ? restartBusy
      ? "Restarting..."
      : "Restart daemon"
    : checking
      ? "Retrying..."
      : "Retry";

  const bodyCopy = canRestart
    ? "The daemon is not reachable. Restart it to continue."
    : isDesktop
      ? "The daemon is not reachable. Reconnect to a host from the launcher."
      : "The daemon is not reachable. Start it, then retry this screen.";

  const mismatchCopy = (() => {
    if (!mismatch) return null;
    if (mismatch.kind === "daemon_older") {
      return desktopKind === "ssh"
        ? "The remote daemon is older than this desktop app. If no tasks are running, update proceeds automatically. If tasks are active, this dialog asks for confirmation."
        : "The local daemon is older than this desktop app. Restart it from this dialog.";
    }
    if (mismatch.kind === "desktop_older") {
      return "The desktop app is older than the daemon. Update the desktop app from this dialog, then retry.";
    }
    return "The desktop app and daemon versions do not match. Update both to the same version, then retry.";
  })();

  if (status === "mismatch" && mismatch) {
    return (
      <div className="daemon-overlay" role="dialog" aria-modal="true">
        <div className="daemon-overlay-card">
          <div className="daemon-overlay-eyebrow">Version mismatch</div>
          <h2>Desktop and daemon are out of sync</h2>
          <p className="daemon-overlay-body">{mismatchCopy}</p>
          <div className="daemon-overlay-target">
            Desktop: <span className="daemon-overlay-mono">{mismatch.desktop_version}</span>
          </div>
          <div className="daemon-overlay-target">
            Daemon: <span className="daemon-overlay-mono">{mismatch.daemon_version}</span>
          </div>
          {target && (
            <div className="daemon-overlay-target">
              Target: <span className="daemon-overlay-mono">{target}</span>
            </div>
          )}
          {error && <div className="daemon-overlay-error">{error}</div>}
          {notice && <div className="daemon-overlay-notice">{notice}</div>}
          <div className="daemon-overlay-actions">
            {mismatch.kind === "daemon_older" && isDesktop && desktopKind !== "ssh" && (
              <button
                type="button"
                className="daemon-overlay-button"
                onClick={onMismatchRestartLocal}
                disabled={checking || restartBusy || remoteUpdateBusy}
              >
                {restartBusy ? "Restarting..." : "Restart local daemon"}
              </button>
            )}
            {mismatch.kind === "daemon_older" && isDesktop && desktopKind === "ssh" && (
              <button
                type="button"
                className="daemon-overlay-button"
                onClick={onMismatchUpdateRemote}
                disabled={checking || restartBusy || remoteUpdateBusy || desktopAppUpdateBusy}
              >
                {remoteUpdateBusy ? "Updating..." : "Update remote daemon"}
              </button>
            )}
            {mismatch.kind === "desktop_older" && isDesktop && (
              <button
                type="button"
                className="daemon-overlay-button"
                onClick={onMismatchUpdateDesktop}
                disabled={checking || restartBusy || remoteUpdateBusy || desktopAppUpdateBusy}
              >
                {desktopAppUpdateBusy ? "Updating..." : "Update desktop app"}
              </button>
            )}
            <button
              type="button"
              className="daemon-overlay-button"
              onClick={checkNow}
              disabled={checking || restartBusy || remoteUpdateBusy || desktopAppUpdateBusy}
            >
              {checking ? "Retrying..." : "Retry"}
            </button>
            {mismatch.kind === "desktop_older" && (
              <Link className="daemon-overlay-secondary" to="/diagnostics">
                Open diagnostics
              </Link>
            )}
            {isDesktop && (
              <Link className="daemon-overlay-secondary" to="/">
                Open launcher
              </Link>
            )}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="daemon-overlay" role="dialog" aria-modal="true">
      <div className="daemon-overlay-card">
        <div className="daemon-overlay-eyebrow">Connection lost</div>
        <h2>ctx daemon unavailable</h2>
        <p className="daemon-overlay-body">{bodyCopy}</p>
        {target && (
          <div className="daemon-overlay-target">
            Target: <span className="daemon-overlay-mono">{target}</span>
          </div>
        )}
        {error && <div className="daemon-overlay-error">{error}</div>}
        {notice && <div className="daemon-overlay-notice">{notice}</div>}
        <div className="daemon-overlay-actions">
          <button
            type="button"
            className="daemon-overlay-button"
            onClick={primaryAction}
            disabled={checking || restartBusy}
          >
            {primaryLabel}
          </button>
          {isDesktop && (
            <Link className="daemon-overlay-secondary" to="/">
              Open launcher
            </Link>
          )}
        </div>
      </div>
    </div>
  );
}
