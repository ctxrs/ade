import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { resetDaemonConnection } from "../api/client";
import { useDaemonBaseUrl } from "../api/useDaemonConnection";
import { desktopDisconnect, isDesktopApp } from "../utils/desktop";
import { clearLauncherRecents, getLauncherRecentsCount } from "../state/launcherRecentsStore";

export default function AppSettingsPage() {
  const baseUrl = useDaemonBaseUrl();
  const [recentsCount, setRecentsCount] = useState(0);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    getLauncherRecentsCount()
      .then((count) => {
        if (!cancelled) setRecentsCount(count);
      })
      .catch(() => {
        if (!cancelled) setRecentsCount(0);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const onDisconnect = async () => {
    setBusy(true);
    try {
      if (isDesktopApp()) {
        await desktopDisconnect();
      }
      resetDaemonConnection({ persistBaseUrl: true, clearPersistedBaseUrl: true });
    } finally {
      setBusy(false);
    }
  };

  const onClearRecents = async () => {
    setBusy(true);
    try {
      await clearLauncherRecents();
      setRecentsCount(0);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <div className="header">
        <Link to="/">← Launcher</Link>
      </div>
      <h1>Settings</h1>

      <div className="card">
        <div className="row">
          <strong>Connection</strong>
        </div>
        <div className="muted" style={{ marginTop: 6 }}>
          {baseUrl ? `Connected to ${baseUrl}` : "Not connected"}
        </div>
        <div className="row" style={{ marginTop: 12 }}>
          <button type="button" onClick={onDisconnect} disabled={busy}>
            Disconnect
          </button>
          {baseUrl && (
            <Link to="/workspaces" style={{ marginLeft: 12 }}>
              Go to workspaces
            </Link>
          )}
        </div>
      </div>

      <div className="card">
        <div className="row">
          <strong>Recents</strong>
        </div>
        <div className="muted" style={{ marginTop: 6 }}>
          {recentsCount} saved entries
        </div>
        <div className="row" style={{ marginTop: 12 }}>
          <button type="button" onClick={onClearRecents} disabled={busy || recentsCount === 0}>
            Clear recents
          </button>
        </div>
      </div>

      <div className="muted" style={{ marginTop: 12 }}>
        Daemon-specific settings are available at <Link to="/settings">Daemon settings</Link> (requires an active connection).
      </div>
    </div>
  );
}
