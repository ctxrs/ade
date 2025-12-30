import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useLocation } from "react-router-dom";
import { daemonFetchRaw, getDaemonBaseUrl, setDaemonBaseUrl } from "../api/client";
import {
  desktopConnectLocal,
  desktopGetConnection,
  isDesktopApp,
  type DesktopConnectionKind,
  type DesktopConnectionInfo,
} from "../utils/desktop";

type DaemonStatus = "unknown" | "ok" | "down";

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

export default function DaemonAvailabilityOverlay() {
  const location = useLocation();
  const [status, setStatus] = useState<DaemonStatus>("unknown");
  const [checking, setChecking] = useState(false);
  const [restartBusy, setRestartBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [desktopKind, setDesktopKind] = useState<DesktopConnectionKind | null>(null);
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
        setStatus("ok");
        setError(null);
      } else {
        setStatus("down");
        setError(extractErrorMessage(resp));
      }
    } catch (err) {
      if (requestId !== requestIdRef.current) return;
      const message = err instanceof Error ? err.message : String(err);
      setStatus("down");
      setError(trimError(message || "Unable to reach the Context daemon."));
    } finally {
      if (requestId === requestIdRef.current) {
        checkingRef.current = false;
        setChecking(false);
      }
    }
  }, []);

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
    const intervalMs = status === "down" ? 8000 : 20000;
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
      if (token) sessionStorage.setItem("contextAuthToken", token);
      else sessionStorage.removeItem("contextAuthToken");
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

  const showOverlay = status === "down" && !overlaySuppressed(location.pathname);
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

  return (
    <div className="daemon-overlay" role="dialog" aria-modal="true">
      <div className="daemon-overlay-card">
        <div className="daemon-overlay-eyebrow">Connection lost</div>
        <h2>Context daemon unavailable</h2>
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
