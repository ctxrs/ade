import { useEffect, useState } from "react";
import { Camera, Link2, QrCode, Server, Unplug } from "lucide-react";
import { Link, useNavigate } from "react-router-dom";
import {
  clearDaemonConnection,
  getDaemonConnectionReadiness,
  listWorkspaces,
  normalizeDaemonBaseUrl,
  setDaemonConnection,
} from "../../api/client";
import { pairManagedMobileQrPayload } from "../../api/mobileSecureClient";
import { useDaemonConnection } from "../../api/useDaemonConnection";
import { TextInput, Textarea } from "../../components/ui/text-input";
import { errorMessage } from "../../utils/errorMessage";
import { MobileShellChrome } from "./MobileShellChrome";
import { canUseQrCameraScanner, MobileQrScanner } from "./MobileQrScanner";

const requireProductionDirectHttpsBaseUrl = (input: string): string | null => {
  const normalized = normalizeDaemonBaseUrl(input);
  if (!normalized) return null;
  try {
    const url = new URL(normalized);
    return url.protocol === "https:" ? normalized : null;
  } catch {
    return null;
  }
};

export function MobileConnectPage() {
  const navigate = useNavigate();
  const connection = useDaemonConnection();
  const readiness = getDaemonConnectionReadiness(connection);
  const [baseUrl, setBaseUrl] = useState(connection.baseUrl ?? "");
  const [token, setToken] = useState(connection.authToken ?? "");
  const [busy, setBusy] = useState(false);
  const [pairingBusy, setPairingBusy] = useState(false);
  const [scannerOpen, setScannerOpen] = useState(false);
  const [qrPayload, setQrPayload] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setBaseUrl(connection.baseUrl ?? "");
    setToken(connection.authToken ?? "");
  }, [connection.authToken, connection.baseUrl]);

  const connect = async () => {
    const normalizedBaseUrl = requireProductionDirectHttpsBaseUrl(baseUrl);
    if (!normalizedBaseUrl) {
      setError("Enter a reachable HTTPS daemon URL.");
      return;
    }
    const trimmedToken = token.trim();
    if (!trimmedToken) {
      setError("Enter the daemon bearer token.");
      return;
    }

    setBusy(true);
    setError(null);
    setDaemonConnection(
      {
        baseUrl: normalizedBaseUrl,
        authToken: trimmedToken,
        mobileSecure: null,
        source: "mobile_manual_connect",
      },
      { persistBaseUrl: true, persistAuthToken: true },
    );
    try {
      await listWorkspaces();
      navigate("/", { replace: true });
    } catch (err) {
      clearDaemonConnection({
        clearPersistedBaseUrl: true,
        clearPersistedAuthToken: true,
      });
      setBaseUrl(normalizedBaseUrl);
      setToken(trimmedToken);
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const disconnect = () => {
    const nextBaseUrl = connection.baseUrl ?? baseUrl;
    clearDaemonConnection({
      clearPersistedBaseUrl: true,
      clearPersistedAuthToken: true,
    });
    setBaseUrl(nextBaseUrl);
    setToken("");
    setError(null);
  };

  const pairQrPayload = async (payload = qrPayload) => {
    const trimmed = payload.trim();
    if (!trimmed) {
      setError("Paste the mobile pairing QR payload.");
      return;
    }
    setPairingBusy(true);
    setError(null);
    try {
      await pairManagedMobileQrPayload(trimmed);
      setQrPayload("");
      navigate("/", { replace: true });
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setPairingBusy(false);
    }
  };

  return (
    <MobileShellChrome
      title="Connect to daemon"
    >
      <form
        className="mobile-shell-form"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void pairQrPayload();
        }}
      >
        <div className="mobile-shell-block-header">
          <div>
            <div className="mobile-shell-block-kicker">
              <QrCode size={14} aria-hidden="true" />
              Managed tunnel
            </div>
            <h2>Pair from QR</h2>
          </div>
        </div>

        {scannerOpen ? (
          <MobileQrScanner
            onCancel={() => {
              setScannerOpen(false);
              setError(null);
            }}
            onDetected={(payload) => {
              setScannerOpen(false);
              setQrPayload(payload);
              void pairQrPayload(payload);
            }}
            onError={(message) => {
              setScannerOpen(false);
              setError(message);
            }}
          />
        ) : (
          <button
            type="button"
            className="mobile-shell-scan-button"
            onClick={() => {
              if (!canUseQrCameraScanner()) {
                setError("Camera QR scanning is unavailable here. Paste the QR payload instead.");
                return;
              }
              setError(null);
              setScannerOpen(true);
            }}
          >
            <Camera size={18} aria-hidden="true" />
            Scan QR
          </button>
        )}

        <label className="mobile-shell-field">
          <span>QR payload</span>
          <Textarea
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            placeholder="{&quot;type&quot;:&quot;context_mobile_e2ee&quot;,...}"
            rows={5}
            value={qrPayload}
            onChange={(event) => setQrPayload(event.target.value)}
          />
        </label>

        <div className="mobile-shell-actions">
          <button type="submit" className="mobile-shell-btn mobile-shell-btn-primary" disabled={pairingBusy}>
            <QrCode size={15} aria-hidden="true" />
            {pairingBusy ? "Pairing..." : "Pair"}
          </button>
        </div>
      </form>

      <form
        className="mobile-shell-form"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void connect();
        }}
      >
        <div className="mobile-shell-block-header">
          <div>
            <div className="mobile-shell-block-kicker">
              <Server size={14} aria-hidden="true" />
              Direct host
            </div>
            <h2>Daemon endpoint</h2>
          </div>
        </div>

        <label className="mobile-shell-field">
          <span>Daemon URL</span>
          <TextInput
            type="url"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            placeholder="https://daemon.example.com"
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.target.value)}
          />
        </label>

        <label className="mobile-shell-field">
          <span>Bearer token</span>
          <TextInput
            type="password"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            placeholder="ctx daemon token"
            value={token}
            onChange={(event) => setToken(event.target.value)}
          />
        </label>

        <div className="mobile-shell-actions">
          <button type="submit" className="mobile-shell-btn mobile-shell-btn-primary" disabled={busy}>
            <Link2 size={15} aria-hidden="true" />
            {busy ? "Connecting..." : "Connect"}
          </button>
          {readiness.isReady ? (
            <>
              <Link className="mobile-shell-btn mobile-shell-btn-secondary" to="/">
                Open workspaces
              </Link>
              <button type="button" className="mobile-shell-btn mobile-shell-btn-secondary" onClick={disconnect} disabled={busy}>
                <Unplug size={15} aria-hidden="true" />
                Disconnect
              </button>
            </>
          ) : null}
        </div>

        {error ? <div className="mobile-shell-error">{error}</div> : null}
      </form>

    </MobileShellChrome>
  );
}
