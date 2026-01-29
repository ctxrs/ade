import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { getDaemonBaseUrl, setDaemonAuthToken, setDaemonBaseUrl } from "../api/client";
import { desktopDisconnect, isDesktopApp } from "../utils/desktop";

const RECENTS_KEY = "contextDesktopRecentsV1";

export default function AppSettingsPage() {
  const [baseUrl, setBaseUrl] = useState<string | null>(getDaemonBaseUrl());
  const [recentsCount, setRecentsCount] = useState(0);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    try {
      const raw = localStorage.getItem(RECENTS_KEY);
      const parsed = raw ? JSON.parse(raw) : null;
      setRecentsCount(Array.isArray(parsed) ? parsed.length : 0);
    } catch {
      setRecentsCount(0);
    }
  }, []);

  const onDisconnect = async () => {
    setBusy(true);
    try {
      if (isDesktopApp()) {
        await desktopDisconnect();
      }
      setDaemonAuthToken(null);
      setDaemonBaseUrl(null, true);
      setBaseUrl(null);
    } finally {
      setBusy(false);
    }
  };

  const onClearRecents = () => {
    try {
      localStorage.removeItem(RECENTS_KEY);
      setRecentsCount(0);
    } catch {
      // ignore
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
