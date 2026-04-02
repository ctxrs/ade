import { useCallback, useMemo, useState, useSyncExternalStore } from "react";
import { Link, useLocation } from "react-router-dom";
import { applyDaemonDesktopConnection, listWorkspaceTasks, listWorkspaces } from "../api/client";
import { useDaemonBaseUrl } from "../api/useDaemonConnection";
import {
  checkDaemonAvailabilityNow,
  getDaemonAvailabilitySnapshot,
  subscribeDaemonAvailability,
} from "../state/daemonAvailabilityMonitor";
import {
  desktopApplyAppUpdate,
  isDesktopApp,
  desktopRestartLocalDaemon,
  desktopUpdateRemoteDaemon,
  type DesktopConnectionInfo,
} from "../utils/desktop";

const overlaySuppressed = (pathname: string): boolean => {
  if (pathname === "/") return true;
  if (pathname === "/workspace-setup") return true;
  return false;
};

const SUPPRESSED_AVAILABILITY = {
  status: "unknown",
  checking: false,
  error: null,
  desktopKind: null,
  desktopVersion: null,
  mismatch: null,
} as const;

const trimError = (value: string): string => {
  const text = String(value || "").trim();
  if (!text) return "";
  return text.length > 220 ? `${text.slice(0, 220)}...` : text;
};

export default function DaemonAvailabilityOverlay() {
  const location = useLocation();
  const suppressed = overlaySuppressed(location.pathname);
  const [restartBusy, setRestartBusy] = useState(false);
  const [remoteUpdateBusy, setRemoteUpdateBusy] = useState(false);
  const [desktopAppUpdateBusy, setDesktopAppUpdateBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [restartLock, setRestartLock] = useState(false);
  const [remoteUpdateLock, setRemoteUpdateLock] = useState(false);
  const isDesktop = isDesktopApp();
  const daemonBaseUrl = useDaemonBaseUrl();
  const availability = useSyncExternalStore(
    useCallback(
      (listener: Parameters<typeof subscribeDaemonAvailability>[0]) =>
        suppressed ? () => {} : subscribeDaemonAvailability(listener),
      [suppressed],
    ),
    useCallback(
      () => (suppressed ? SUPPRESSED_AVAILABILITY : getDaemonAvailabilitySnapshot()),
      [suppressed],
    ),
    useCallback(
      () => (suppressed ? SUPPRESSED_AVAILABILITY : getDaemonAvailabilitySnapshot()),
      [suppressed],
    ),
  );
  const checking = availability.checking;
  const status = availability.status;
  const desktopKind = availability.desktopKind;
  const mismatch = availability.mismatch;
  const error = actionError ?? availability.error;

  const checkNow = useCallback(async () => {
    setActionError(null);
    await checkDaemonAvailabilityNow();
  }, []);

  const applyConnection = (info: DesktopConnectionInfo) => {
    applyDaemonDesktopConnection(info);
  };

  const restartDaemon = useCallback(async () => {
    if (!isDesktop || restartLock) return;
    setRestartLock(true);
    setRestartBusy(true);
    setActionError(null);
    setNotice(null);
    try {
      const info = await desktopRestartLocalDaemon();
      applyConnection(info);
      await checkNow();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setActionError(trimError(message || "Unable to restart the daemon."));
    } finally {
      setRestartLock(false);
      setRestartBusy(false);
    }
  }, [checkNow, isDesktop, restartLock]);

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
    if (!isDesktop || remoteUpdateLock) return;
    setRemoteUpdateLock(true);
    setRemoteUpdateBusy(true);
    setActionError(null);
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
      setActionError(trimError(message || "Unable to update the remote daemon."));
    } finally {
      setRemoteUpdateLock(false);
      setRemoteUpdateBusy(false);
    }
  }, [checkNow, isDesktop, remoteUpdateLock]);

  const onMismatchUpdateDesktop = useCallback(async () => {
    if (!isDesktop || desktopAppUpdateBusy) return;
    if (!confirmInterruptingAction("update_desktop")) return;
    setDesktopAppUpdateBusy(true);
    setActionError(null);
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
      setActionError(trimError(message || "Unable to update the desktop app."));
    } finally {
      setDesktopAppUpdateBusy(false);
    }
  }, [checkNow, desktopAppUpdateBusy, isDesktop]);

  const target = useMemo(() => {
    return daemonBaseUrl ?? "";
  }, [daemonBaseUrl]);

  const showOverlay =
    (status === "down" || status === "mismatch") && !suppressed;
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
