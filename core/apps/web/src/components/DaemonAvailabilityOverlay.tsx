import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useLocation } from "react-router-dom";
import { daemonFetchRaw, getDaemonBaseUrl, setDaemonBaseUrl } from "../api/client";
import {
  desktopConnectLocal,
  desktopGetConnection,
  desktopGetVersion,
  isDesktopApp,
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
  if (pathname === "/app-settings") return true;
  return false;
};

const trimError = (value: string): string => {
  const text = String(value || "").trim();
  if (!text) return "";
  return text.length > 220 ? `${text.slice(0, 220)}…` : text;
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
  const [error, setError] = useState<string | null>(null);
  const [desktopKind, setDesktopKind] = useState<DesktopConnectionKind | null>(null);
  const [desktopVersion, setDesktopVersion] = useState<string | null>(null);
  const [mismatch, setMismatch] = useState<VersionMismatch | null>(null);
  const checkingRef = useRef(false);
  const requestIdRef = useRef(0);
  const isDesktop = isDesktopApp();

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
        let parsed: any = null;
        if (resp.body) {
          try {
            parsed = JSON.parse(resp.body);
          } catch {
            parsed = null;
          }
        }
        const daemonVersion = String(parsed?.daemon_version ?? parsed?.version ?? "").trim();
        const expectedVersion = String(
          parsed?.compatibility?.desktop_exact_version ?? daemonVersion ?? "",
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
    const base = String(info.base_url ?? "").trim();
    const token = String(info.token ?? "").trim();
    if (base) setDaemonBaseUrl(base, true);
    else setDaemonBaseUrl(null, false);
    try {
      if (token) sessionStorage.setItem("ctxAuthToken", token);
      else sessionStorage.removeItem("ctxAuthToken");
    } catch {
      // ignore
    }
  };

  const restartDaemon = useCallback(async () => {
    if (!isDesktop || restartBusy) return;
    setRestartBusy(true);
    setError(null);
    try {
      const info = await desktopConnectLocal();
      applyConnection(info);
      setDesktopKind(info.kind);
      await checkNow();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(trimError(message || "Unable to restart the daemon."));
    } finally {
      setRestartBusy(false);
    }
  }, [checkNow, isDesktop, restartBusy]);

  const target = useMemo(() => {
    const base = getDaemonBaseUrl();
    if (base) return base;
    if (!isDesktop) return window.location.origin;
    return "";
  }, [isDesktop]);

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
        ? "The daemon is older than this desktop app. Run ctx self-update on the remote host, then retry."
        : "The daemon is older than this desktop app. Run ctx self-update or use Diagnostics > Updates, then retry.";
    }
    if (mismatch.kind === "desktop_older") {
      return "The desktop app is older than the daemon. Update the desktop app, then retry.";
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
          <div className="daemon-overlay-actions">
            <button
              type="button"
              className="daemon-overlay-button"
              onClick={checkNow}
              disabled={checking}
            >
              {checking ? "Retrying..." : "Retry"}
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
